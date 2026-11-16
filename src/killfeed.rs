//! Experimental, offline OCR of a user-selected recording. It only proposes
//! ranges: no game hooks, process-memory reads, automatic exports or uploads.
use crate::{capture::command, export::Snapshot, i18n::text, jobs::Control, recorder};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Region {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}
impl Default for Region {
    fn default() -> Self {
        Self {
            left: 0.55,
            top: 0.02,
            width: 0.44,
            height: 0.30,
        }
    }
}
impl Region {
    pub fn valid(&self) -> bool {
        [self.left, self.top, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.left >= 0.0
            && self.top >= 0.0
            && self.width >= 0.1
            && self.height >= 0.05
            && self.left + self.width <= 1.001
            && self.top + self.height <= 1.001
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Suggestion {
    pub at: f64,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Review {
    pub suggestions: Vec<Suggestion>,
    pub samples: usize,
    pub recognized_rows: usize,
}

#[derive(Clone, Debug)]
struct Word {
    text: String,
    x: i32,
    y: i32,
    height: i32,
    right: i32,
}

fn normalized(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.chars().count() > 32 || normalized(name).len() < 3 || !name.is_ascii() {
        return Err(text("Gib den im Killfeed sichtbaren Namen ein (mindestens 3 Zeichen, derzeit nur lateinische ASCII-Zeichen).", "Enter the name shown in the killfeed (at least 3 characters; currently Latin ASCII characters only).").into());
    }
    Ok(())
}

/// Require a left-hand killer name followed by a separated right-hand name.
/// A matching victim, partial name or isolated label must not become a kill.
fn own_kills(mut words: Vec<Word>, player: &str) -> Vec<String> {
    words.sort_by_key(|w| w.y);
    let mut rows: Vec<Vec<Word>> = Vec::new();
    for word in words {
        if let Some(row) = rows
            .iter_mut()
            .find(|r| (r[0].y - word.y).abs() <= (r[0].height.max(word.height) / 2).max(5))
        {
            row.push(word);
        } else {
            rows.push(vec![word]);
        }
    }
    let player = normalized(player);
    rows.into_iter()
        .filter_map(|mut row| {
            row.sort_by_key(|w| w.x);
            let mut prefix = String::new();
            for i in 0..row.len().saturating_sub(1) {
                prefix.push_str(&normalized(&row[i].text));
                if prefix == player && row[i + 1].x - row[i].right >= row[i].height.max(8) {
                    let victim = row[i + 1..]
                        .iter()
                        .map(|w| w.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if normalized(&victim).len() < 2 || normalized(&victim) == player {
                        return None;
                    }
                    return Some(format!(
                        "{} → {}",
                        row[..=i]
                            .iter()
                            .map(|w| w.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" "),
                        victim
                    ));
                }
            }
            None
        })
        .collect()
}

#[derive(Default)]
struct Tracker {
    seen: HashMap<String, (f64, u32, bool)>,
    suggestions: Vec<Suggestion>,
}
impl Tracker {
    fn frame(&mut self, at: f64, rows: &[String], duration: f64, before: f64, after: f64) {
        // Persistent killfeed lines and short OCR gaps count once. A line must
        // appear in two sampled frames before it is suggested.
        let mut in_frame = std::collections::HashSet::new();
        for row in rows {
            let key = normalized(row);
            if !in_frame.insert(key.clone()) {
                continue;
            }
            let item = self.seen.entry(key).or_insert((at, 0, false));
            if at - item.0 > 3.0 {
                *item = (at, 0, false);
            }
            item.1 += 1;
            if item.1 >= 2 && !item.2 {
                let event = (at - 0.5).max(0.0);
                self.suggestions.push(Suggestion {
                    at: event,
                    start: (event - before).max(0.0),
                    end: (event + after).min(duration),
                    text: row.chars().take(160).collect(),
                });
                item.2 = true;
            }
            item.0 = at;
        }
        self.seen.retain(|_, (last, _, _)| at - *last <= 8.0);
    }
}

pub fn models_dir() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("STILLCLIP_MODELS_DIR").map(PathBuf::from),
        Some(crate::settings::application_dir().join("models")),
        std::env::current_dir().ok().map(|p| p.join("models")),
    ];
    candidates.into_iter().flatten().find(|p| {
        p.join("text-detection.onnx").is_file() && p.join("text-recognition.onnx").is_file()
    })
}

fn engine(directory: &Path) -> Result<OcrEngine, String> {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../packaging/ocr-models.json"))
            .map_err(|e| e.to_string())?;
    for model in manifest["models"]
        .as_array()
        .ok_or("Invalid model manifest.")?
    {
        let path = directory.join(model["file"].as_str().ok_or("Missing model filename.")?);
        let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut bytes = [0; 65536];
        loop {
            let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&bytes[..n]);
        }
        if model["sha256"] != format!("{:x}", hash.finalize()) {
            return Err(text(
                "Die OCR-Modelle sind beschädigt. Installiere Stillclip erneut.",
                "The OCR models are damaged. Reinstall Stillclip.",
            )
            .into());
        }
    }
    OcrEngine::new(OcrEngineParams {
        detection_model: Some(
            rten::Model::load_file(directory.join("text-detection.onnx"))
                .map_err(|e| e.to_string())?,
        ),
        recognition_model: Some(
            rten::Model::load_file(directory.join("text-recognition.onnx"))
                .map_err(|e| e.to_string())?,
        ),
        ..Default::default()
    })
    .map_err(|e| e.to_string())
}

pub fn analyze(
    ffmpeg: &Path,
    input: &Path,
    scratch: &Path,
    player: &str,
    region: Region,
    start: f64,
    end: f64,
    before: f64,
    after: f64,
    control: &Control,
) -> Result<Review, String> {
    validate_name(player)?;
    if !region.valid() {
        return Err(text("Ungültiger Killfeed-Bereich.", "Invalid killfeed region.").into());
    }
    let info = recorder::probe_media(ffmpeg, input)?;
    if !start.is_finite()
        || !end.is_finite()
        || start < 0.0
        || end <= start
        || end > info.duration + 0.1
        || end - start > 1800.0
    {
        return Err(text(
            "Wähle für die experimentelle Analyse einen Bereich von maximal 30 Minuten.",
            "Select a range of up to 30 minutes for experimental analysis.",
        )
        .into());
    }
    let root = models_dir().ok_or(text(
        "OCR-Modelle fehlen. Verwende den Installer oder das vollständige Portable-Paket.",
        "OCR models are missing. Use the installer or the complete portable package.",
    ))?;
    control.progress(
        0.01,
        text("Lokale OCR-Modelle laden", "Loading local OCR models"),
    );
    let engine = engine(&root)?;
    // The single worker and small batches bound disk/memory usage. A new
    // recording cancels this task instead of competing with game capture.
    let staging = Snapshot::new(scratch, &[])?;
    let crop = format!(
        "crop=iw*{:.6}:ih*{:.6}:iw*{:.6}:ih*{:.6},fps=2,scale=960:-1",
        region.width, region.height, region.left, region.top
    );
    let mut tracker = Tracker::default();
    let mut samples = 0;
    let mut recognized_rows = 0;
    let mut at = start;
    while at < end {
        control.check()?;
        let length = (end - at).min(10.0);
        let mut cmd = command(ffmpeg);
        cmd.args([
            "-hide_banner",
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-threads",
            "2",
            "-ss",
        ])
        .arg(format!("{at:.6}"))
        .arg("-i")
        .arg(input)
        .arg("-t")
        .arg(format!("{length:.6}"))
        .args(["-vf", &crop, "-an", "-threads", "2", "-frames:v", "20"])
        .arg(staging.dir.join("frame-%03d.png"));
        let result = control
            .part(0.02 + 0.95 * (at - start) / (end - start), 0.0)
            .run(&mut cmd, length, text("Killfeed lesen", "Reading killfeed"))?;
        if !result.status.success() {
            return Err(recorder::diagnostic(&result.stderr));
        }
        for frame in 1..=20 {
            control.check()?;
            let image = staging.dir.join(format!("frame-{frame:03}.png"));
            if !image.is_file() {
                break;
            }
            let pixels = image::open(&image).map_err(|e| e.to_string())?.into_rgb8();
            let input = engine
                .prepare_input(
                    ImageSource::from_bytes(pixels.as_raw(), pixels.dimensions())
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            let boxes = engine.detect_words(&input).map_err(|e| e.to_string())?;
            let lines = engine.find_text_lines(&input, &boxes);
            let lines = engine
                .recognize_text(&input, &lines)
                .map_err(|e| e.to_string())?;
            let mut words = Vec::new();
            for line in lines.iter().flatten() {
                for word in line.words() {
                    let rect = word.bounding_rect();
                    words.push(Word {
                        text: word.to_string(),
                        x: rect.left(),
                        right: rect.right(),
                        y: rect.top() + rect.height() / 2,
                        height: rect.height(),
                    });
                }
            }
            let rows = own_kills(words, player);
            recognized_rows += rows.len();
            let time = (at + (frame - 1) as f64 * 0.5).min(end);
            tracker.frame(
                time,
                &rows,
                info.duration,
                before.clamp(1.0, 120.0),
                after.clamp(1.0, 60.0),
            );
            samples += 1;
            fs::remove_file(image).map_err(|e| e.to_string())?;
            control.progress(
                0.02 + 0.97 * (time - start) / (end - start),
                text(
                    "Killfeed lesen · experimentell",
                    "Reading killfeed · experimental",
                ),
            );
        }
        at += length;
    }
    control.check()?;
    Ok(Review {
        suggestions: tracker.suggestions,
        samples,
        recognized_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn word(text: &str, x: i32) -> Word {
        Word {
            text: text.into(),
            x,
            right: x + 70,
            y: 20,
            height: 20,
        }
    }
    #[test]
    fn own_kill_requires_full_left_name_and_separate_victim() {
        assert_eq!(
            own_kills(vec![word("Elvin", 10), word("Opponent", 250)], "Elvin").len(),
            1
        );
        assert!(own_kills(vec![word("Opponent", 10), word("Elvin", 250)], "Elvin").is_empty());
        assert!(own_kills(vec![word("Elvin2", 10), word("Opponent", 250)], "Elvin").is_empty());
        assert!(own_kills(vec![word("Elvin", 10)], "Elvin").is_empty());
        assert!(own_kills(vec![word("Elvin", 10), word("Opponent", 85)], "Elvin").is_empty());
        assert!(validate_name("ab").is_err());
        assert!(validate_name("プレイヤー").is_err());
    }
    #[test]
    fn persistent_lines_ocr_gaps_and_single_frame_noise_do_not_duplicate_kills() {
        let mut tracker = Tracker::default();
        let rows = vec!["Elvin → Opponent".into()];
        tracker.frame(5.0, &rows, 30.0, 10.0, 5.0);
        assert!(tracker.suggestions.is_empty());
        tracker.frame(5.5, &rows, 30.0, 10.0, 5.0);
        tracker.frame(6.0, &[], 30.0, 10.0, 5.0);
        tracker.frame(7.0, &rows, 30.0, 10.0, 5.0);
        assert_eq!(tracker.suggestions.len(), 1);
        assert_eq!(tracker.suggestions[0].start, 0.0);
        tracker.frame(20.0, &rows, 30.0, 10.0, 5.0);
        tracker.frame(20.5, &rows, 30.0, 10.0, 5.0);
        assert_eq!(tracker.suggestions.len(), 2);
    }

    #[test]
    #[ignore = "requires bundled OCR models; synthetic image only"]
    fn synthetic_killfeed_models_read_killer_and_victim_positions() {
        let engine = engine(&models_dir().expect("bundled OCR models")).unwrap();
        let pixels = image::load_from_memory(include_bytes!("../tests/fixtures/killfeed.png"))
            .unwrap()
            .into_rgb8();
        let input = engine
            .prepare_input(ImageSource::from_bytes(pixels.as_raw(), pixels.dimensions()).unwrap())
            .unwrap();
        let boxes = engine.detect_words(&input).unwrap();
        let lines = engine.find_text_lines(&input, &boxes);
        let lines = engine.recognize_text(&input, &lines).unwrap();
        let mut words = Vec::new();
        for line in lines.iter().flatten() {
            for word in line.words() {
                let r = word.bounding_rect();
                words.push(Word {
                    text: word.to_string(),
                    x: r.left(),
                    right: r.right(),
                    y: r.top() + r.height() / 2,
                    height: r.height(),
                });
            }
        }
        let own = own_kills(words.clone(), "Elvin");
        assert_eq!(own.len(), 1, "Recognized words: {words:?}");
        assert!(own[0].contains("Opponent"), "{own:?}");
    }
}
