//! Source tracks are identified by an explicit marker triplet, never guessed
//! from a clip's name or number of tracks. Legacy mixed audio stays mixed.
use crate::{capture::command, export::Snapshot, i18n::text, jobs::Control, recorder};
use serde::{Deserialize, Serialize};
use std::{fs, io::{BufReader, Read}, path::Path};

#[derive(Clone, Debug, Serialize)]
pub struct Track {
    pub index: usize,
    pub role: String,
    pub peaks: Vec<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Analysis {
    pub separated: bool,
    pub duration: f64,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct Mix {
    pub game: u32,
    pub microphone: u32,
    pub mixed: u32,
}

impl Mix {
    pub fn validate(self) -> Result<Self, String> {
        if self.game > 200 || self.microphone > 200 || self.mixed > 200 {
            Err(text("Lautstärke muss zwischen 0 und 200 % liegen.", "Volume must be between 0 and 200%.").into())
        } else { Ok(self) }
    }
}

fn roles(streams: &[serde_json::Value]) -> bool {
    streams.len() == 3 && streams.iter().zip(["qaa", "qab", "qac"])
        .all(|(stream, language)| stream["tags"]["language"] == language)
}

pub fn tracks(ffmpeg: &Path, input: &Path) -> Result<(bool, usize), String> {
    let probe = recorder::ffprobe_for(ffmpeg).ok_or(text("FFprobe fehlt.", "FFprobe is missing."))?;
    let output = command(&probe).args(["-v", "error", "-select_streams", "a", "-show_entries", "stream=index:stream_tags=language", "-of", "json"])
        .arg(input).output().map_err(|e| e.to_string())?;
    if !output.status.success() { return Err(recorder::diagnostic(&output.stderr)); }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let streams = value["streams"].as_array().ok_or("Missing audio stream data.")?;
    Ok((roles(streams), streams.len()))
}

/// Bounded memory: reduce a temporary PCM stream into at most 1,200 peaks.
fn peaks(path: &Path) -> Result<Vec<f32>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let samples = file.metadata().map_err(|e| e.to_string())?.len() / 2;
    let bucket = samples.div_ceil(1200).max(1);
    let mut input = BufReader::new(file);
    let mut result = Vec::with_capacity(1200);
    let mut bytes = [0; 2];
    let mut count = 0u64;
    let mut maximum = 0f32;
    while input.read_exact(&mut bytes).is_ok() {
        maximum = maximum.max(f32::from(i16::from_le_bytes(bytes)).abs() / 32768.0);
        count += 1;
        if count == bucket { result.push(maximum); count = 0; maximum = 0.0; }
    }
    if count > 0 { result.push(maximum); }
    Ok(result)
}

pub fn analyze(ffmpeg: &Path, input: &Path, scratch: &Path, control: &Control) -> Result<Analysis, String> {
    let info = recorder::probe_media(ffmpeg, input)?;
    let (separated, count) = tracks(ffmpeg, input)?;
    if info.duration > 6.0 * 3600.0 { return Err(text("Bitte für die Wellenform einen Clip unter sechs Stunden auswählen.", "Choose a clip shorter than six hours for waveform analysis.").into()); }
    let staging = Snapshot::new(scratch, &[])?;
    let selected = if separated { vec![(1, "game"), (2, "microphone")] } else if count > 0 { vec![(0, "mixed")] } else { Vec::new() };
    let mut result = Analysis { separated, duration: info.duration, tracks: Vec::new() };
    for (position, (index, role)) in selected.iter().enumerate() {
        control.check()?;
        let pcm = staging.dir.join("waveform.pcm");
        let mut cmd = command(ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-threads", "2", "-i"])
            .arg(input).args(["-map", &format!("0:a:{index}"), "-vn", "-ac", "1", "-ar", "8000", "-c:a", "pcm_s16le", "-f", "s16le"]).arg(&pcm);
        let part = control.part(position as f64 / selected.len() as f64, 1.0 / selected.len() as f64);
        let output = part.run(&mut cmd, info.duration, text("Wellenform lesen", "Reading waveform"))?;
        if !output.status.success() { return Err(recorder::diagnostic(&output.stderr)); }
        control.check()?;
        result.tracks.push(Track { index: *index, role: (*role).into(), peaks: peaks(&pcm)? });
        fs::remove_file(pcm).map_err(|e| e.to_string())?;
    }
    Ok(result)
}

fn mix_filter(separated: bool, mix: Mix) -> String {
    if separated {
        format!("[0:a:1]volume={:.3}[game];[0:a:2]volume={:.3}[mic];[game][mic]amix=inputs=2:normalize=0:duration=longest,alimiter=limit=0.98:level=0[audio]", mix.game as f64 / 100.0, mix.microphone as f64 / 100.0)
    } else {
        format!("[0:a:0]volume={:.3},alimiter=limit=0.98:level=0[audio]", mix.mixed as f64 / 100.0)
    }
}

pub struct Preview { pub path: std::path::PathBuf, _workspace: Snapshot }
pub fn preview(ffmpeg: &Path, input: &Path, scratch: &Path, start: f64, end: f64, mix: Mix, control: &Control) -> Result<Preview, String> {
    let mix = mix.validate()?;
    let duration = recorder::probe_duration(ffmpeg, input)?;
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start || start >= duration || end > duration + 0.1 {
        return Err(text("Ungültiger Clipbereich.", "Invalid clip range.").into());
    }
    let (separated, count) = tracks(ffmpeg, input)?;
    if count == 0 { return Err(text("Dieser Clip enthält keinen Ton.", "This clip has no audio.").into()); }
    let workspace = Snapshot::new(scratch, &[])?;
    let path = workspace.dir.join("mix.wav");
    let length = (end.min(duration) - start).min(20.0);
    let mut cmd = command(ffmpeg);
    cmd.args(["-hide_banner", "-v", "error", "-nostdin", "-y", "-filter_complex_threads", "2", "-ss"])
        .arg(format!("{start:.6}")).arg("-i").arg(input).arg("-t").arg(format!("{length:.6}"))
        .args(["-filter_complex", &mix_filter(separated, mix), "-map", "[audio]", "-vn", "-c:a", "pcm_s16le", "-ar", "48000", "-ac", "2"]).arg(&path);
    let result = control.run(&mut cmd, length, text("Audiovorschau vorbereiten", "Preparing audio preview"))?;
    if !result.status.success() { return Err(recorder::diagnostic(&result.stderr)); }
    control.check()?;
    Ok(Preview { path, _workspace: workspace })
}

/// Renders the chosen audio mix into a new, precisely trimmed MP4. The
/// original file (including isolated tracks) remains available for re-editing.
pub fn export(ffmpeg: &Path, input: &Path, output: &Path, start: f64, end: f64, mix: Mix,
    crop: Option<crate::portrait::Crop>, control: &Control) -> Result<(), String> {
    let mix = mix.validate()?;
    let info = recorder::probe_media(ffmpeg, input)?;
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start || end > info.duration + 0.1 {
        return Err(text("Ungültiger Clipbereich.", "Invalid clip range.").into());
    }
    let (separated, count) = tracks(ffmpeg, input)?;
    if count == 0 { return Err(text("Dieser Clip enthält keinen Ton.", "This clip has no audio.").into()); }
    let staging = Snapshot::new(output.parent().ok_or("Missing output directory.")?, &[])?;
    let staged = staging.dir.join("audio-mix.mp4");
    let filter = crop.map(|c| c.validate()).transpose()?.map_or_else(|| "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(), |c| c.filter());
    let mut cmd = command(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-filter_complex_threads", "2", "-ss"])
        .arg(format!("{start:.6}")).arg("-i").arg(input).arg("-t").arg(format!("{:.6}", end.min(info.duration) - start))
        .args(["-filter_complex", &mix_filter(separated, mix), "-map", "0:v:0", "-map", "[audio]", "-vf", &filter,
            "-c:v", "libx264", "-preset", "veryfast", "-threads", "4", "-crf", "20", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "160k", "-metadata:s:a:0", "language=und", "-metadata:s:a:0", "title=Edited mix",
            "-movflags", "+faststart"]).arg(&staged);
    let result = control.run(&mut cmd, end - start, text("Audiomix exportieren", "Exporting audio mix"))?;
    if !result.status.success() { return Err(recorder::diagnostic(&result.stderr)); }
    control.check()?;
    staging.publish(&staged, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn mixed_and_arbitrary_multitrack_files_never_promise_isolated_microphone() {
        assert!(!roles(&[json!({}), json!({}), json!({})]));
        assert!(!roles(&[json!({"tags":{"language":"qaa"}}), json!({"tags":{"language":"qac"}})]));
        assert!(roles(&["qaa", "qab", "qac"].map(|language| json!({"tags":{"language":language}}))));
        assert!(Mix { game: 201, ..Mix::default() }.validate().is_err());
    }

    #[test]
    #[ignore = "requires FFmpeg; generated media only"]
    fn synthetic_separated_audio_survives_ts_remux_and_muted_export() {
        let ffmpeg = recorder::find_ffmpeg().expect("FFmpeg");
        let root = std::env::temp_dir().join(format!("stillclip-audio-edit-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let staging = Snapshot::new(&root, &[]).unwrap();
        let ts = staging.dir.join("input.ts"); let mp4 = staging.dir.join("input.mp4"); let edited = staging.dir.join("edited.mp4");
        let result = command(&ffmpeg).args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i", "color=s=320x180:r=30:d=3", "-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-f", "lavfi", "-i", "sine=frequency=880:duration=3", "-filter_complex", "[1:a]asplit[g][game];[2:a]asplit[m][mic];[g][m]amix=inputs=2:normalize=0[mix]", "-map", "0:v", "-map", "[mix]", "-map", "[game]", "-map", "[mic]", "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-metadata:s:a:0", "language=qaa", "-metadata:s:a:1", "language=qab", "-metadata:s:a:2", "language=qac"]).arg(&ts).output().unwrap();
        assert!(result.status.success(), "{}", recorder::diagnostic(&result.stderr));
        assert!(command(&ffmpeg).args(["-v", "error", "-y", "-i"]).arg(&ts).args(["-map", "0", "-c", "copy"]).arg(&mp4).status().unwrap().success());
        assert_eq!(tracks(&ffmpeg, &mp4).unwrap(), (true, 3));
        let analysis = analyze(&ffmpeg, &mp4, &root, &Control::new()).unwrap();
        assert_eq!(analysis.tracks.len(), 2);
        assert!(analysis.tracks.iter().all(|t| !t.peaks.is_empty() && t.peaks.len() <= 1200 && t.peaks.iter().any(|p| *p > 0.05)));
        export(&ffmpeg, &mp4, &edited, 0.0, 2.0, Mix { game: 0, microphone: 0, mixed: 0 }, None, &Control::new()).unwrap();
        let analysis = analyze(&ffmpeg, &edited, &root, &Control::new()).unwrap();
        assert!(!analysis.separated);
        assert!(analysis.tracks[0].peaks.iter().all(|p| *p < 0.001));
        assert!(mp4.is_file());
        drop(staging); fs::remove_dir_all(root).unwrap();
    }
}
