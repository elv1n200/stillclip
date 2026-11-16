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
        return Err(crate::i18n::text("Aktiviere Systemton oder Mikrofon für den Audio-Test.", "Enable system audio or microphone for the audio test.").into());
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
    let inputs = pipes.iter().map(|p| p.input.clone()).collect::<Vec<_>>();
    let mut cmd = mix_command(ffmpeg, &inputs, &path);
    let result = control.run(&mut cmd, 10.0, crate::i18n::text("Audio-Test läuft", "Audio test running"));
    drop(pipes); // FFmpeg has exited, so pipe writes can finish before joining WASAPI threads.
    let output = result?;
    if !output.status.success() {
        return Err(crate::tr_format!(
            "Audio-Test fehlgeschlagen: {}", "Audio test failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let info = crate::recorder::probe_media(ffmpeg, &path)?;
    if !info.audio || info.duration < 9.5 {
        return Err(crate::i18n::text("Der Audio-Test wurde vorzeitig beendet. Prüfe das Audiogerät.", "The audio test ended early. Check your audio device.").into());
    }
    control.check()?;
    Ok(Preview {
        path,
        _workspace: workspace,
    })
}

fn mix_command(
    ffmpeg: &Path,
    inputs: &[crate::capture::AudioInput],
    path: &Path,
) -> std::process::Command {
    let mut cmd = crate::capture::command(ffmpeg);
    cmd.args(["-hide_banner", "-v", "error", "-nostdin", "-y"]);
    for input in inputs {
        cmd.args([
            "-thread_queue_size",
            "512",
            "-probesize",
            "32",
            "-analyzeduration",
            "0",
            "-f",
            input.sample_format,
            "-ar",
            "48000",
            "-ac",
            "2",
            "-i",
        ])
        .arg(&input.path);
    }
    let mut filters: Vec<String> = inputs
        .iter()
        .enumerate()
        .map(|(i, p)| format!("[{i}:a]volume={:.3}[a{i}]", p.gain))
        .collect();
    let labels = (0..inputs.len())
        .map(|i| format!("[a{i}]"))
        .collect::<String>();
    filters.push(format!(
        "{labels}amix=inputs={}:duration=longest:dropout_transition=0:normalize=0[out]",
        inputs.len()
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
    cmd
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires FFmpeg; generated PCM only, no device access"]
    fn synthetic_audio_preview_uses_selected_gains_and_duration() {
        let ffmpeg = crate::recorder::find_ffmpeg().unwrap();
        let root =
            std::env::temp_dir().join(format!("stillclip-audio-preview-{}", std::process::id()));
        let workspace = Snapshot::new(&root, &[]).unwrap();
        let mut inputs = Vec::new();
        for (i, sample, gain) in [(0, 0.1_f32, 2.0), (1, -0.3_f32, 0.5)] {
            let path = workspace.dir.join(format!("source-{i}.pcm"));
            std::fs::write(&path, sample.to_le_bytes().repeat(48_000 * 2 * 10)).unwrap();
            inputs.push(crate::capture::AudioInput {
                path: path.to_string_lossy().into_owned(),
                sample_format: "f32le",
                rate: 48_000,
                channels: 2,
                gain,
            });
        }
        let output = workspace.dir.join("preview.wav");
        let result = Control::new()
            .run(&mut mix_command(&ffmpeg, &inputs, &output), 10.0, "Test")
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let info = crate::recorder::probe_media(&ffmpeg, &output).unwrap();
        assert!(info.audio);
        assert!((info.duration - 10.0).abs() < 0.01);
        let decoded = crate::capture::command(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-t", "0.1", "-f", "f32le", "-"])
            .output()
            .unwrap();
        assert!(decoded.status.success());
        assert!(!decoded.stdout.is_empty());
        for sample in decoded.stdout.chunks_exact(4) {
            assert!((f32::from_le_bytes(sample.try_into().unwrap()) - 0.05).abs() < 0.002);
        }
        drop(workspace);
        let _ = std::fs::remove_dir_all(root);
    }
}
