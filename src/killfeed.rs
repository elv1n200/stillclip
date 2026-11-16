//! Shared local image reader for review and automatic VALORANT highlights.
//! It observes saved video pixels only; game state is never accessed.
use crate::{capture::command, export::Snapshot, i18n::text, jobs::Control, recorder};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
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
    pub events: Vec<f64>,
    pub kills: usize,
    pub confidence: crate::valorant::Confidence,
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
fn own_rows(mut words: Vec<Word>, player: &str) -> Vec<crate::valorant::Row> {
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
                if let Some(exact) = crate::valorant::name_matches(&prefix, &player)
                    .filter(|_| row[i + 1].x - row[i].right >= row[i].height.max(8))
                {
                    let victim = row[i + 1..]
                        .iter()
                        .map(|w| w.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if normalized(&victim).len() < 2 || normalized(&victim) == player {
                        return None;
                    }
                    return Some(crate::valorant::Row {
                        text: format!(
                            "{} → {}",
                            row[..=i]
                                .iter()
                                .map(|w| w.text.as_str())
                                .collect::<Vec<_>>()
                                .join(" "),
                            victim
                        ),
                        victim,
                        exact,
                    });
                }
            }
            None
        })
        .collect()
}

pub struct Reader {
    engine: OcrEngine,
}
pub struct Observation {
    pub rows: Vec<crate::valorant::Row>,
    pub score: Option<(u32, u32)>,
    pub round_banner: bool,
}
impl Reader {
    pub fn load() -> Result<Self, String> {
        let root = models_dir().ok_or(text(
            "OCR-Modelle fehlen. Nutze den vollständigen Installer.",
            "OCR models are missing. Use the complete installer.",
        ))?;
        Ok(Self {
            engine: engine(&root)?,
        })
    }
    fn words(&self, pixels: &image::RgbImage) -> Result<Vec<Word>, String> {
        let input = self
            .engine
            .prepare_input(
                ImageSource::from_bytes(pixels.as_raw(), pixels.dimensions())
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        let boxes = self
            .engine
            .detect_words(&input)
            .map_err(|e| e.to_string())?;
        let lines = self.engine.find_text_lines(&input, &boxes);
        let lines = self
            .engine
            .recognize_text(&input, &lines)
            .map_err(|e| e.to_string())?;
        Ok(lines
            .iter()
            .flatten()
            .flat_map(|line| line.words())
            .map(|word| {
                let rect = word.bounding_rect();
                Word {
                    text: word.to_string(),
                    x: rect.left(),
                    right: rect.right(),
                    y: rect.top() + rect.height() / 2,
                    height: rect.height(),
                }
            })
            .collect())
    }
    pub fn observe(
        &self,
        frame: &image::RgbImage,
        player: &str,
        region: Region,
    ) -> Result<Observation, String> {
        if !region.valid() {
            return Err(text("Ungültiger Killfeed-Bereich.", "Invalid killfeed region.").into());
        }
        let feed = crop(frame, region, 960, 480);
        // Score and round banners corroborate boundaries; they never generate
        // kills. Restrict score candidates to the two sides of the timer.
        let hud = crop(
            frame,
            Region {
                left: 0.32,
                top: 0.0,
                width: 0.36,
                height: 0.25,
            },
            640,
            320,
        );
        // The detection model has a fixed input size. Packing the two regions
        // into one image avoids running that expensive model twice per frame.
        // A blank gutter keeps words from the feed and HUD separate.
        let hud_y = feed.height() + 32;
        let mut combined =
            image::RgbImage::new(feed.width().max(hud.width()), hud_y + hud.height());
        image::imageops::replace(&mut combined, &feed, 0, 0);
        image::imageops::replace(&mut combined, &hud, 0, hud_y as i64);
        let mut feed_words = Vec::new();
        let mut words = Vec::new();
        for mut word in self.words(&combined)? {
            if word.y < feed.height() as i32 {
                feed_words.push(word);
            } else if word.y >= hud_y as i32 && word.x < hud.width() as i32 {
                word.y -= hud_y as i32;
                words.push(word);
            }
        }
        let rows = own_rows(feed_words, player);
        let numbers = |lo: f64, hi: f64| -> Option<u32> {
            let mut values = words
                .iter()
                .filter(|w| {
                    let x = (w.x + w.right) as f64 / (2.0 * hud.width() as f64);
                    let y = w.y as f64 / hud.height() as f64;
                    x >= lo
                        && x <= hi
                        && y < 0.32
                        && w.text.len() <= 2
                        && w.text.bytes().all(|b| b.is_ascii_digit())
                })
                .filter_map(|w| w.text.parse::<u32>().ok());
            let value = values.next()?;
            values.next().is_none().then_some(value)
        };
        let score = numbers(0.20, 0.45).zip(numbers(0.55, 0.80));
        let banner = words
            .iter()
            .map(|w| w.text.to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(" ");
        let banner = banner.split_whitespace().collect::<Vec<_>>().join(" ");
        let round_banner = [
            "BUY PHASE",
            "KAUFPHASE",
            "ROUND WON",
            "ROUND LOST",
            "VICTORY",
            "DEFEAT",
            "GEWONNEN",
            "VERLOREN",
        ]
        .iter()
        .any(|needle| banner.contains(needle));
        Ok(Observation {
            rows,
            score,
            round_banner,
        })
    }
}
fn crop(
    frame: &image::RgbImage,
    region: Region,
    max_width: u32,
    max_height: u32,
) -> image::RgbImage {
    let x = (region.left * frame.width() as f64).floor() as u32;
    let y = (region.top * frame.height() as f64).floor() as u32;
    let width = ((region.width * frame.width() as f64).ceil() as u32)
        .max(1)
        .min(frame.width() - x);
    let height = ((region.height * frame.height() as f64).ceil() as u32)
        .max(1)
        .min(frame.height() - y);
    let image = image::imageops::crop_imm(frame, x, y, width, height).to_image();
    let scale = (max_width as f64 / width as f64).min(max_height as f64 / height as f64);
    image::imageops::resize(
        &image,
        (width as f64 * scale).round().max(1.0) as u32,
        (height as f64 * scale).round().max(1.0) as u32,
        image::imageops::FilterType::Triangle,
    )
}

pub fn decode_frames(
    ffmpeg: &Path,
    input: &Path,
    directory: &Path,
    start: f64,
    length: f64,
    control: &Control,
) -> Result<Vec<PathBuf>, String> {
    control.check()?;
    let mut cmd = command(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000 | 0x00004000); // No window; below-normal priority.
    }
    cmd.args([
        "-hide_banner",
        "-v",
        "error",
        "-nostdin",
        "-y",
        "-threads",
        "1",
        "-ss",
    ])
    .arg(format!("{start:.6}"))
    .arg("-i")
    .arg(input)
    .arg("-t")
    .arg(format!("{length:.6}"))
    .args([
        "-vf",
        "fps=2,scale=w='min(1920,iw)':h='min(1080,ih)':force_original_aspect_ratio=decrease",
        "-an",
        "-threads",
        "1",
        "-compression_level",
        "1",
        "-frames:v",
        "24",
    ])
    .arg(directory.join("frame-%03d.png"));
    let result = control.run(
        &mut cmd,
        length,
        text("Spielereignisse lesen", "Reading game events"),
    )?;
    if !result.status.success() {
        return Err(recorder::diagnostic(&result.stderr));
    }
    Ok((1..=24)
        .map(|i| directory.join(format!("frame-{i:03}.png")))
        .take_while(|p| p.is_file())
        .collect())
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
    control.progress(
        0.01,
        text("Lokale OCR-Modelle laden", "Loading local OCR models"),
    );
    let reader = Reader::load()?;
    let staging = Snapshot::new(scratch, &[])?;
    let mut tracker = crate::valorant::Tracker::default();
    let mut rounds = crate::valorant::Rounds::default();
    let mut moments = crate::valorant::Moments::new(crate::valorant::Preferences::default());
    let mut found = Vec::new();
    let mut samples = 0;
    let mut recognized_rows = 0;
    let mut at = start;
    while at < end {
        control.check()?;
        let length = (end - at).min(10.0);
        let frames = decode_frames(
            ffmpeg,
            input,
            &staging.dir,
            at,
            length,
            &control.part(0.02 + 0.95 * (at - start) / (end - start), 0.0),
        )?;
        for (index, path) in frames.iter().enumerate() {
            control.check()?;
            let time = (at + index as f64 * 0.5).min(end);
            let pixels = image::open(path).map_err(|e| e.to_string())?.into_rgb8();
            let observation = reader.observe(&pixels, player, region)?;
            recognized_rows += observation.rows.len();
            if rounds.frame(time, observation.score, observation.round_banner) {
                moments.boundary();
            }
            for event in tracker.frame(time, &observation.rows) {
                moments.event(event);
            }
            moments.advance(time);
            found.extend(moments.take_ready(time, false));
            samples += 1;
            fs::remove_file(path).map_err(|e| e.to_string())?;
            control.progress(
                0.02 + 0.97 * (time - start) / (end - start),
                text(
                    "Highlights erkennen · experimentell",
                    "Detecting highlights · experimental",
                ),
            );
        }
        at += length;
    }
    found.extend(moments.take_ready(info.duration, true));
    control.check()?;
    let suggestions = found
        .into_iter()
        .map(|m| {
            let at = m.events[0].at;
            let last = m.events.last().unwrap().at;
            let confidence = if m.confirmed_kills() == m.events.len() {
                crate::valorant::Confidence::Confirmed
            } else {
                crate::valorant::Confidence::Review
            };
            Suggestion {
                at,
                start: (at - before.clamp(1.0, 120.0)).max(start),
                end: (last + after.clamp(1.0, 60.0)).min(info.duration),
                text: if m.events.len() == 1 {
                    m.events[0].text.clone()
                } else {
                    m.label()
                },
                kills: m.events.len(),
                events: m.events.iter().map(|e| e.at).collect(),
                confidence,
            }
        })
        .collect();
    Ok(Review {
        suggestions,
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
            own_rows(vec![word("Elvin", 10), word("Opponent", 250)], "Elvin").len(),
            1
        );
        assert!(own_rows(vec![word("Opponent", 10), word("Elvin", 250)], "Elvin").is_empty());
        assert!(own_rows(vec![word("Elvin2", 10), word("Opponent", 250)], "Elvin").is_empty());
        assert!(own_rows(vec![word("Elvin", 10)], "Elvin").is_empty());
        assert!(own_rows(vec![word("Elvin", 10), word("Opponent", 85)], "Elvin").is_empty());
        assert!(validate_name("ab").is_err());
        assert!(validate_name("プレイヤー").is_err());
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
        let own = own_rows(words.clone(), "Elvin");
        assert_eq!(own.len(), 1, "Recognized words: {words:?}");
        assert!(own[0].text.contains("Opponent"), "{own:?}");
    }
}
