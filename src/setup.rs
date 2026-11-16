//! A short recording starts only after the user presses the wizard's test
//! button. Its private workspace is removed when the preview is dismissed.
use crate::{
    export::Snapshot,
    i18n::text,
    jobs::Control,
    recorder::{Recorder, SessionConfig, SessionInfo},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub struct Preview {
    pub path: PathBuf,
    pub info: SessionInfo,
    _workspace: Snapshot,
}

pub fn record(
    ffmpeg: &Path,
    root: &Path,
    mut config: SessionConfig,
    control: &Control,
) -> Result<Preview, String> {
    let workspace = Snapshot::new(root, &[])?;
    config.clips_dir = workspace.dir.clone();
    config.keep_full_session = true;
    config.replay_seconds = 30;
    let mut recorder = Recorder::new(ffmpeg.to_owned());
    control.progress(0.0, text("Testaufnahme starten", "Starting test recording"));
    control.check()?;
    let info = recorder.start(config)?;
    while recorder.elapsed() < Duration::from_secs(10) {
        if control.cancelled() {
            let _ = recorder.stop();
            return Err(crate::jobs::CANCELLED.into());
        }
        if recorder.poll().is_some() {
            let _ = recorder.stop();
            return Err(text(
                "Die Testaufnahme wurde unterbrochen. Prüfe Quelle und Encoder.",
                "The test recording stopped. Check the source and encoder.",
            )
            .into());
        }
        control.progress(
            recorder.elapsed().as_secs_f64() / 12.0,
            text(
                "Testaufnahme läuft · 10 Sekunden",
                "Test recording · 10 seconds",
            ),
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    control.progress(
        0.9,
        text("Testaufnahme abschließen", "Finishing test recording"),
    );
    let path = recorder.stop()?.ok_or(text(
        "Die Testaufnahme hat kein Video erzeugt.",
        "The test recording produced no video.",
    ))?;
    control.check()?;
    Ok(Preview {
        path,
        info,
        _workspace: workspace,
    })
}
