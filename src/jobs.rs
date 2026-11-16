//! One FIFO for editor exports, with bounded subprocess output and cancellation.
use serde::Serialize;
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub const LIMIT: usize = 12;
pub const CANCELLED: &str = "Export abgebrochen.";

#[derive(Default)]
struct Progress {
    fraction: f64,
    phase: String,
}
struct Shared {
    cancel: AtomicBool,
    progress: Mutex<Progress>,
    started: Instant,
}

#[derive(Clone)]
pub struct Control {
    shared: Arc<Shared>,
    from: f64,
    span: f64,
}
impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}
impl Control {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                cancel: AtomicBool::new(false),
                progress: Mutex::new(Progress::default()),
                started: Instant::now(),
            }),
            from: 0.0,
            span: 1.0,
        }
    }
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::SeqCst);
    }
    pub fn cancelled(&self) -> bool {
        self.shared.cancel.load(Ordering::SeqCst)
    }
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err(CANCELLED.into())
        } else {
            Ok(())
        }
    }
    pub fn part(&self, from: f64, span: f64) -> Self {
        Self {
            shared: self.shared.clone(),
            from: self.from + self.span * from,
            span: self.span * span,
        }
    }
    pub fn progress(&self, fraction: f64, phase: &str) {
        if let Ok(mut p) = self.shared.progress.lock() {
            p.fraction = p
                .fraction
                .max((self.from + self.span * fraction.clamp(0.0, 1.0)).clamp(0.0, 1.0));
            p.phase = phase.to_owned();
        }
    }
    pub fn view(&self) -> (u32, String, Option<u64>) {
        let Ok(p) = self.shared.progress.lock() else {
            return (0, String::new(), None);
        };
        let elapsed = self.shared.started.elapsed().as_secs_f64();
        let eta = (p.fraction >= 0.03 && elapsed > 2.0)
            .then(|| (elapsed * (1.0 - p.fraction) / p.fraction).ceil() as u64);
        ((p.fraction * 100.0).floor() as u32, p.phase.clone(), eta)
    }

    pub fn run(&self, command: &mut Command, seconds: f64, phase: &str) -> Result<Output, String> {
        self.check()?;
        self.progress(0.0, phase);
        command.args(["-nostats", "-stats_period", "0.25", "-progress", "pipe:1"]);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let (mut child, group) = crate::process_tree::spawn(command)
            .map_err(|e| format!("FFmpeg konnte nicht gestartet werden: {e}"))?;
        let stdout = child.stdout.take().ok_or("FFmpeg-Fortschritt fehlt.")?;
        let stderr = child.stderr.take().ok_or("FFmpeg-Protokoll fehlt.")?;
        let meter = self.clone();
        let label = phase.to_owned();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(value) = line
                    .strip_prefix("out_time_us=")
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|v| v.is_finite())
                {
                    if seconds > 0.0 {
                        meter.progress((value / 1_000_000.0 / seconds).clamp(0.0, 0.995), &label);
                    }
                }
            }
        });
        let errors = std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut tail = Vec::new();
            let mut buffer = [0; 4096];
            while let Ok(n) = stderr.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buffer[..n]);
                if tail.len() > 16_384 {
                    tail.drain(..tail.len() - 16_384);
                }
            }
            tail
        });
        let result = loop {
            if self.cancelled() {
                group.terminate(&mut child);
                break child.wait();
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(40)),
                Err(error) => {
                    group.terminate(&mut child);
                    let _ = child.wait();
                    break Err(error);
                }
            }
        };
        // Close the per-export job before joining readers: no detached child may keep a pipe open.
        drop(group);
        let _ = reader.join();
        let stderr = errors.join().unwrap_or_default();
        self.check()?;
        let status = result.map_err(|e| e.to_string())?;
        if status.success() {
            self.progress(1.0, phase);
        }
        Ok(Output {
            status,
            stdout: Vec::new(),
            stderr,
        })
    }
}

pub type Work = Box<dyn FnOnce(Control) -> Result<crate::JobOutcome, String> + Send>;
pub struct Pending {
    pub id: u64,
    pub label: String,
    pub sources: Vec<PathBuf>,
    pub work: Work,
}
struct Active {
    id: u64,
    label: String,
    sources: Vec<PathBuf>,
    control: Control,
}
#[derive(Serialize)]
pub struct View {
    pub id: u64,
    pub label: String,
    pub running: bool,
    pub percent: u32,
    pub phase: String,
    pub eta_seconds: Option<u64>,
    pub cancelling: bool,
}
#[derive(Default)]
pub struct Queue {
    next: u64,
    pending: VecDeque<Pending>,
    active: Option<Active>,
}
impl Queue {
    pub fn push(&mut self, label: String, sources: Vec<PathBuf>, work: Work) -> Result<(), String> {
        if self.pending.len() + usize::from(self.active.is_some()) >= LIMIT {
            return Err(format!(
                "Die Exportliste ist voll (höchstens {LIMIT} Aufträge)."
            ));
        }
        self.next += 1;
        self.pending.push_back(Pending {
            id: self.next,
            label,
            sources,
            work,
        });
        Ok(())
    }
    pub fn start_next(&mut self) -> Option<(Pending, Control)> {
        if self.active.is_some() {
            return None;
        }
        let job = self.pending.pop_front()?;
        let control = Control::new();
        self.active = Some(Active {
            id: job.id,
            label: job.label.clone(),
            sources: job.sources.clone(),
            control: control.clone(),
        });
        Some((job, control))
    }
    pub fn finish(&mut self, id: u64) -> bool {
        if self.active.as_ref().is_some_and(|a| a.id == id) {
            self.active = None;
            true
        } else {
            false
        }
    }
    pub fn cancel(&mut self, id: u64) {
        self.pending.retain(|p| p.id != id);
        if let Some(a) = &self.active {
            if a.id == id {
                a.control.cancel();
            }
        }
    }
    pub fn cancel_all(&mut self) {
        self.pending.clear();
        if let Some(a) = &self.active {
            a.control.cancel();
        }
    }
    pub fn is_empty(&self) -> bool {
        self.active.is_none() && self.pending.is_empty()
    }
    pub fn uses(&self, path: &Path) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| a.sources.iter().any(|p| p == path))
            || self
                .pending
                .iter()
                .any(|p| p.sources.iter().any(|s| s == path))
    }
    pub fn views(&self) -> Vec<View> {
        let mut items = Vec::new();
        if let Some(a) = &self.active {
            let (percent, phase, eta_seconds) = a.control.view();
            items.push(View {
                id: a.id,
                label: a.label.clone(),
                running: true,
                percent,
                phase,
                eta_seconds,
                cancelling: a.control.cancelled(),
            });
        }
        items.extend(self.pending.iter().map(|p| View {
            id: p.id,
            label: p.label.clone(),
            running: false,
            percent: 0,
            phase: "Wartet".into(),
            eta_seconds: None,
            cancelling: false,
        }));
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fifo_cancellation_keeps_other_jobs_and_source_locks() {
        let mut q = Queue::default();
        for name in ["a", "b", "c"] {
            q.push(
                name.into(),
                vec![PathBuf::from(name)],
                Box::new(|_| Err("unused".into())),
            )
            .unwrap();
        }
        let (first, control) = q.start_next().unwrap();
        assert_eq!(first.label, "a");
        assert!(q.start_next().is_none());
        q.cancel(2);
        assert!(!q.uses(Path::new("b")));
        q.cancel(first.id);
        assert!(control.cancelled());
        assert!(q.uses(Path::new("a")));
        assert!(!q.finish(999));
        assert!(q.finish(first.id));
        assert!(!q.uses(Path::new("a")));
        assert_eq!(q.start_next().unwrap().0.label, "c");
    }
    #[test]
    #[ignore = "requires FFmpeg; synthetic source only"]
    fn synthetic_cancel_stops_ffmpeg_and_discards_partial_files() {
        let ffmpeg = crate::recorder::find_ffmpeg().unwrap();
        let root = std::env::temp_dir().join(format!("stillclip-cancel-{}", std::process::id()));
        let workspace = crate::export::Snapshot::new(&root, &[]).unwrap();
        let dir = workspace.dir.clone();
        let output = dir.join("partial.mp4");
        let control = Control::new();
        let cancel = control.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            cancel.cancel();
        });
        let mut cmd = crate::capture::command(&ffmpeg);
        cmd.args([
            "-v",
            "error",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30",
            "-t",
            "60",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ])
        .arg(&output);
        let started = Instant::now();
        assert_eq!(control.run(&mut cmd, 60.0, "Test").unwrap_err(), CANCELLED);
        assert!(started.elapsed() < Duration::from_secs(5));
        thread.join().unwrap();
        drop(workspace);
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
