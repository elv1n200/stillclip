//! Short, private audio preview using exactly the capture sources and gains.
use crate::{
    audio::{AudioPipe, AudioSource, Meter},
    export::Snapshot,
    jobs::Control,
};
use std::path::{Path, PathBuf};

pub struct Preview {
    pub path: PathBuf,
    _workspace: Snapshot,
}
pub type Meters = Vec<(String, Meter)>;

pub fn record(
    ffmpeg: &Path,
    root: &Path,
    sources: Vec<(AudioSource, f32)>,
    control: Control,
    ready: impl FnOnce(Meters),
) -> Result<Preview, String> {
    if sources.is_empty() {
        return Err("Aktiviere Systemton oder Mikrofon für den Audio-Test.".into());
    }
    let workspace = Snapshot::new(root, &[])?;
    let path = workspace.dir.join("audio-test.wav");
    let mut pipes = Vec::new();
    for (source, gain) in sources {
        control.check()?;
        pipes.push(AudioPipe::start(source, gain)?);
    }
    ready(
        pipes
            .iter()
            .map(|p| (p.source.label().into(), p.meter.clone()))
            .collect(),
    );
    let mut cmd = crate::capture::command(ffmpeg);
    cmd.args(["-hide_banner", "-v", "error", "-nostdin", "-y"]);
    for pipe in &pipes {
        cmd.args([
            "-thread_queue_size",
            "512",
            "-f",
            pipe.input.sample_format,
            "-ar",
            "48000",
            "-ac",
            "2",
            "-i",
        ])
        .arg(&pipe.input.path);
    }
    let mut filters: Vec<String> = pipes
        .iter()
        .enumerate()
        .map(|(i, p)| format!("[{i}:a]volume={:.3}[a{i}]", p.input.gain))
        .collect();
    let inputs = (0..pipes.len())
        .map(|i| format!("[a{i}]"))
        .collect::<String>();
    filters.push(format!(
        "{inputs}amix=inputs={}:duration=longest:dropout_transition=0:normalize=0[out]",
        pipes.len()
    ));
    cmd.args([
        "-filter_complex",
        &filters.join(";"),
        "-map",
        "[out]",
        "-t",
        "10",
        "-c:a",
        "pcm_s16le",
    ])
    .arg(&path);
    let result = control.run(&mut cmd, 10.0, "Audio-Test läuft");
    drop(pipes); // FFmpeg has exited, so pipe writes can finish before joining WASAPI threads.
    let output = result?;
    if !output.status.success() {
        return Err(format!(
            "Audio-Test fehlgeschlagen: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let info = crate::recorder::probe_media(ffmpeg, &path)?;
    if !info.audio || info.duration < 9.5 {
        return Err("Der Audio-Test wurde vorzeitig beendet. Prüfe das Audiogerät.".into());
    }
    control.check()?;
    Ok(Preview {
        path,
        _workspace: workspace,
    })
}

#[derive(Default)]
pub struct Test {
    pub id: u64,
    pub active: Option<Control>,
    pub meters: Meters,
    pub levels: Vec<(String, f32)>,
    pub preview: Option<Preview>,
    pub error: Option<String>,
}
impl Test {
    pub fn cancel(&self) {
        if let Some(c) = &self.active {
            c.cancel();
        }
    }
    pub fn poll(&mut self) {
        self.levels = self
            .meters
            .iter()
            .map(|(label, m)| (label.clone(), m.take()))
            .collect();
    }
    pub fn state(&self) -> serde_json::Value {
        let (percent, phase, _) = self.active.as_ref().map(|c| c.view()).unwrap_or_default();
        serde_json::json!({ "running": self.active.is_some(), "percent": percent, "phase": phase, "levels": self.levels,
            "error": self.error, "src": self.preview.as_ref().map(|_| format!("http://clip.localhost/audio-test/{}", self.id)) })
    }
}
