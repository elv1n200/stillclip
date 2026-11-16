//! An export owns links to its segments, never the live recorder. The capture
//! can trim its ring or stop while FFmpeg reads this private staging directory.
use crate::recorder::Segment;
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub struct ReplayExport {
    pub snapshot: Snapshot,
    pub output: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: Option<PathBuf>,
    pub span: Option<(f64, f64)>,
    pub seconds: f64,
    pub gop: f64,
    pub precise: bool,
    pub encoder: crate::capture::Encoder,
    pub rates: crate::encoding::CaptureEncoding,
}

impl ReplayExport {
    pub fn run(self) -> Result<crate::recorder::SavedClip, String> {
        let staged = self.snapshot.dir.join("clip.mp4");
        let mut start = if let Some((from, to)) = self.span {
            crate::recorder::export_span(
                &self.ffmpeg,
                self.ffprobe.as_deref(),
                &self.snapshot.segments,
                from,
                to,
                self.gop,
                &staged,
            )?
        } else {
            crate::recorder::export_tail(
                &self.ffmpeg,
                self.ffprobe.as_deref(),
                &self.snapshot.segments,
                self.seconds,
                self.gop,
                &staged,
            )?
        };
        if self.precise {
            start += crate::recorder::trim_to_exact_length(
                &self.ffmpeg,
                &staged,
                self.seconds,
                self.encoder,
                &self.rates,
            )?;
        }
        self.snapshot.publish(&staged, &self.output)?;
        Ok(crate::recorder::SavedClip {
            path: self.output.clone(),
            start,
        })
    }
}

pub struct Snapshot {
    pub segments: Vec<Segment>,
    pub dir: PathBuf,
}

impl Snapshot {
    pub fn new(clips_dir: &Path, segments: &[Segment]) -> Result<Self, String> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = clips_dir.join(".stillclip-exports");
        fs::create_dir_all(&root).map_err(|e| format!("Exportordner: {e}"))?;
        let dir = loop {
            let dir = root.join(format!(
                "{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("Exportordner: {e}")),
            }
        };
        let mut snapshot = Self {
            dir,
            segments: Vec::new(),
        };
        for segment in segments {
            let name = segment.path.file_name().ok_or(crate::i18n::text("Segment ohne Dateinamen", "Segment has no filename"))?;
            let path = snapshot.dir.join(name);
            // Both paths are on the clip volume. FAT/exFAT needs a copy instead.
            fs::hard_link(&segment.path, &path)
                .or_else(|_| fs::copy(&segment.path, &path).map(|_| ()))
                .map_err(|e| crate::tr_format!("Aufnahmesegment konnte nicht gesichert werden: {e}", "Could not preserve recording segment: {e}"))?;
            snapshot.segments.push(Segment {
                path,
                ..segment.clone()
            });
        }
        Ok(snapshot)
    }

    /// Publish only complete output, without overwriting an existing file.
    pub fn publish(&self, staged: &Path, output: &Path) -> Result<(), String> {
        match fs::hard_link(staged, output) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
            Err(_) => {
                let mut input = fs::File::open(staged).map_err(|e| e.to_string())?;
                let mut target = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(output)
                    .map_err(|e| e.to_string())?;
                let result = io::copy(&mut input, &mut target).and_then(|_| target.sync_all());
                drop(target);
                if result.is_err() {
                    let _ = fs::remove_file(output);
                }
                result
            }
        }
        .map_err(|e| crate::tr_format!("Clip konnte nicht gespeichert werden: {e}", "Could not save clip: {e}"))
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_survives_ring_cleanup_and_never_overwrites_a_clip() {
        let root = std::env::temp_dir().join(format!(
            "stillclip-snapshot-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("seg-000001.ts");
        fs::write(&source, b"segment").unwrap();
        let snapshot = Snapshot::new(
            &root,
            &[Segment {
                index: 1,
                path: source.clone(),
                start: Some(0.0),
                end: Some(5.0),
            }],
        )
        .unwrap();
        fs::remove_file(source).unwrap();
        assert_eq!(fs::read(&snapshot.segments[0].path).unwrap(), b"segment");
        let staged = snapshot.dir.join("clip.mp4");
        fs::write(&staged, b"complete clip").unwrap();
        let output = root.join("clip.mp4");
        snapshot.publish(&staged, &output).unwrap();
        let other = snapshot.dir.join("other.mp4");
        fs::write(&other, b"changed").unwrap();
        // The existing path is never replaced, even if a user creates it meanwhile.
        assert!(snapshot.publish(&other, &output).is_err());
        assert_eq!(fs::read(&output).unwrap(), b"complete clip");
        let dir = snapshot.dir.clone();
        drop(snapshot);
        assert!(!dir.exists());
        assert!(output.is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
