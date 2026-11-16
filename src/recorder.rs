//! Continuous recording into a bounded ring of MPEG-TS segments, replay export,
//! full-session export and clip trimming. FFmpeg stays a separate executable.
//!
//! While recording, FFmpeg writes `seg-000000.ts`, `seg-000001.ts`, … with
//! continuous timestamps plus a CSV list of finished segments. Saving a replay
//! byte-concatenates the newest segments (`concat:` protocol) and cuts at the
//! keyframe closest to the requested length, so nothing is re-encoded and the
//! recording never pauses.
use crate::audio::{AudioPipe, AudioSource};
use crate::capture::{
    self, AudioInput, Capabilities, CaptureRequest, Display, Encoder, SEGMENT_SECONDS,
};
use crate::settings::Quality;
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const BUFFER_DIR_NAME: &str = ".stillclip-puffer";
/// `Config.get_ClipCooldownSeconds` falls back to one second between clips.
pub const CLIP_COOLDOWN: Duration = Duration::from_secs(1);

fn command(executable: &Path) -> Command {
    capture::command(executable)
}

fn ffmpeg_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// Checks an explicit override, bundled tools, PATH, and existing Windows installs.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("STILLCLIP_FFMPEG") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(executable) = env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("tools").join(ffmpeg_name()));
            candidates.push(directory.join(ffmpeg_name()));
        }
    }
    if let Ok(directory) = env::current_dir() {
        candidates.push(directory.join("tools").join(ffmpeg_name()));
        candidates.push(directory.join("app").join("tools").join(ffmpeg_name()));
    }
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|dir| dir.join(ffmpeg_name())));
    }
    #[cfg(windows)]
    {
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(directory) = env::var_os(variable) {
                candidates.push(PathBuf::from(directory).join(
                    "Streamlabs OBS/resources/app.asar.unpacked/node_modules/obs-studio-node/ffmpeg.exe",
                ));
            }
        }
        if let Some(directory) = env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(directory).join("Microsoft/WinGet/Links/ffmpeg.exe"));
        }
        candidates.push(PathBuf::from("C:/ffmpeg/bin/ffmpeg.exe"));
    }
    candidates.into_iter().find(|path| {
        path.is_file()
            && command(path)
                .args(["-hide_banner", "-version"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
    })
}

pub fn ffprobe_for(ffmpeg: &Path) -> Option<PathBuf> {
    let probe = ffmpeg.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    probe.is_file().then_some(probe)
}

/// Everything a recording session needs to know up front.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub window: Option<crate::games::RunningApp>,
    pub audio_process: Option<u32>,
    pub display: Display,
    pub encoder: Encoder,
    pub capabilities: Capabilities,
    pub fps: u32,
    pub quality: Quality,
    pub show_cursor: bool,
    pub system_audio: bool,
    /// `Some(device id)` enables the microphone; an empty id is the default device.
    pub microphone: Option<String>,
    pub microphone_separate_track: bool,
    /// Loudness factors; 1.0 keeps the original level.
    pub system_gain: f32,
    pub microphone_gain: f32,
    /// Seconds the segment ring keeps, which is the longest clip length in use.
    pub replay_seconds: u32,
    pub keep_full_session: bool,
    /// Stop the recording while less than this many bytes are free; 0 is off.
    pub min_free_bytes: u64,
    /// Re-encode replay clips to the exact requested length.
    pub precise_clips: bool,
    pub clips_dir: PathBuf,
}

/// What a running session actually uses, for the UI.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SessionInfo {
    pub strategy: String,
    pub width: u32,
    pub height: u32,
    pub audio: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct SessionMarker {
    keep_full_session: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub index: u64,
    pub path: PathBuf,
    /// Seconds, CSV timeline; `None` while the segment is still being written.
    pub start: Option<f64>,
    pub end: Option<f64>,
}

impl Segment {
    pub fn duration(&self) -> Option<f64> {
        Some(self.end? - self.start?)
    }
}

struct Session {
    window: Option<crate::games::RunningApp>,
    child: Child,
    started: Instant,
    dir: PathBuf,
    log_path: PathBuf,
    list_path: PathBuf,
    audio: Vec<AudioPipe>,
    keep_full: bool,
    replay_seconds: u32,
    fps: u32,
    clips_dir: PathBuf,
    info: SessionInfo,
    last_clip: Option<Instant>,
    precise: bool,
    encoder: Encoder,
    rates: crate::encoding::CaptureEncoding,
    min_free_bytes: u64,
    ticks: u32,
}

/// A clip that was just written, plus where it begins on the session timeline.
/// The offset turns marked moments into positions inside the finished clip.
#[derive(Debug)]
pub struct SavedClip {
    pub path: PathBuf,
    pub start: f64,
}

/// Something that happened on its own while recording and that the app has to
/// react to. Returned by [`Recorder::poll`].
pub struct PollIssue {
    pub message: String,
    /// FFmpeg died by itself; starting a fresh capture is worth a try.
    pub capture_lost: bool,
    /// Low disk space or a lost capture window requires an orderly stop.
    pub stop_requested: bool,
}

pub struct Recorder {
    ffmpeg: PathBuf,
    ffprobe: Option<PathBuf>,
    session: Option<Session>,
}

pub fn buffer_dir(clips_dir: &Path) -> PathBuf {
    clips_dir.join(BUFFER_DIR_NAME)
}

/// `fs::canonicalize` yields `\?\C:\…` on Windows, which FFmpeg's concat
/// demuxer and plain path comparisons cannot handle. Strip the prefix.
pub fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

/// Forward slashes for FFmpeg list files and protocol strings.
fn ffmpeg_path(path: &Path) -> String {
    plain_path(path).to_string_lossy().replace('\\', "/")
}

fn segment_index(path: &Path) -> Option<u64> {
    path.file_name()?
        .to_str()?
        .strip_prefix("seg-")?
        .strip_suffix(".ts")?
        .parse()
        .ok()
}

/// Parses FFmpeg's CSV segment list: `name,start,end` per finished segment.
pub fn parse_segment_list(text: &str, dir: &Path) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    for line in text.lines() {
        let mut fields = line.trim().rsplitn(3, ',');
        let (Some(end), Some(start), Some(name)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let path = dir.join(Path::new(name).file_name().unwrap_or_default());
        let (Some(index), Ok(start), Ok(end)) = (
            segment_index(&path),
            start.trim().parse::<f64>(),
            end.trim().parse::<f64>(),
        ) else {
            continue;
        };
        if !(start.is_finite() && end.is_finite() && end >= start) {
            continue;
        }
        if let Some(existing) = segments.iter_mut().find(|s| s.index == index) {
            existing.start = Some(start);
            existing.end = Some(end);
        } else {
            segments.push(Segment {
                index,
                path,
                start: Some(start),
                end: Some(end),
            });
        }
    }
    segments.sort_by_key(|s| s.index);
    segments
}

/// All segment files in the buffer, finished ones annotated from the list.
fn scan_segments(dir: &Path, list_path: &Path) -> Vec<Segment> {
    let mut segments = parse_segment_list(&fs::read_to_string(list_path).unwrap_or_default(), dir);
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(index) = segment_index(&path) {
                if !segments.iter().any(|s| s.index == index) {
                    segments.push(Segment {
                        index,
                        path,
                        start: None,
                        end: None,
                    });
                }
            }
        }
    }
    segments.sort_by_key(|s| s.index);
    segments.retain(|s| s.path.is_file());
    segments
}

/// Time span still backed by files, rather than the complete CSV history.
fn available_span(dir: &Path, list_path: &Path) -> Option<(f64, f64)> {
    // CSV history includes files already removed by the ring. Only live
    // files can back a marker that the UI still offers to save.
    let segments = scan_segments(dir, list_path);
    let from = segments
        .iter()
        .filter_map(|s| s.start)
        .fold(f64::INFINITY, f64::min);
    let to = segments
        .iter()
        .filter_map(|s| s.end)
        .fold(f64::NEG_INFINITY, f64::max);
    (from.is_finite() && to > from).then_some((from, to))
}

/// Picks the newest segments whose combined length covers `wanted` seconds.
/// Unknown (in-progress) durations are estimated with `estimate`.
pub fn tail_segments(segments: &[Segment], wanted: f64, estimate: f64) -> Vec<Segment> {
    let mut chosen = Vec::new();
    let mut total = 0.0;
    for segment in segments.iter().rev() {
        chosen.push(segment.clone());
        total += segment.duration().unwrap_or(estimate).max(0.0);
        if total >= wanted {
            break;
        }
    }
    chosen.reverse();
    chosen
}

/// Segments older than the replay window plus a safety margin.
pub fn expired_segments(segments: &[Segment], replay_seconds: u32) -> Vec<Segment> {
    let Some(latest_end) = segments
        .iter()
        .filter_map(|s| s.end)
        .fold(None, |m: Option<f64>, e| Some(m.map_or(e, |m| m.max(e))))
    else {
        return Vec::new();
    };
    let keep_from = latest_end - f64::from(replay_seconds) - 2.0 * f64::from(SEGMENT_SECONDS);
    segments
        .iter()
        .filter(|s| s.end.is_some_and(|end| end < keep_from))
        .cloned()
        .collect()
}

impl Recorder {
    pub fn new(ffmpeg: PathBuf) -> Self {
        let ffprobe = ffprobe_for(&ffmpeg);
        Self {
            ffmpeg,
            ffprobe,
            session: None,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.session.is_some()
    }

    pub fn info(&self) -> Option<&SessionInfo> {
        self.session.as_ref().map(|s| &s.info)
    }

    pub fn diagnostics(&self) -> Option<crate::diagnostics::Progress> {
        let session = self.session.as_ref()?;
        crate::diagnostics::read(
            &session.dir.join("progress.txt"),
            session.started.elapsed().as_secs_f64(),
        )
    }

    pub fn elapsed(&self) -> Duration {
        self.session
            .as_ref()
            .map_or(Duration::ZERO, |s| s.started.elapsed())
    }

    /// Bytes currently held in the segment buffer.
    pub fn buffer_bytes(&self) -> u64 {
        self.session
            .as_ref()
            .and_then(|s| fs::read_dir(&s.dir).ok())
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    /// A replay can be saved successfully even if something else went wrong.
    fn start_audio(
        config: &SessionConfig,
        warnings: &mut Vec<String>,
    ) -> Result<Vec<AudioPipe>, String> {
        let mut pipes = Vec::new();
        if config.system_audio {
            let source = config
                .audio_process
                .map_or(AudioSource::Loopback, |pid| AudioSource::Process { pid });
            match AudioPipe::start(source, config.system_gain) {
                Ok(pipe) => pipes.push(pipe),
                Err(error) if config.audio_process.is_some() => {
                    return Err(crate::tr_format!("Spielton konnte nicht aufgenommen werden. {error}", "Could not record game audio. {error}"))
                }
                Err(error) => warnings.push(crate::tr_format!("Systemton nicht verfügbar: {error}", "System audio unavailable: {error}")),
            }
        }
        if let Some(device) = &config.microphone {
            match AudioPipe::start(
                AudioSource::Microphone {
                    device_id: device.clone(),
                },
                config.microphone_gain,
            ) {
                Ok(pipe) => pipes.push(pipe),
                Err(error) => warnings.push(crate::tr_format!("Mikrofon nicht verfügbar: {error}", "Microphone unavailable: {error}")),
            }
        }
        Ok(pipes)
    }

    /// Starts recording. Nothing is captured until this is called.
    pub fn start(&mut self, config: SessionConfig) -> Result<SessionInfo, String> {
        if self.session.is_some() {
            return Err(crate::i18n::text("Eine Aufnahme läuft bereits.", "A recording is already running.").into());
        }
        if !cfg!(windows) {
            return Err(crate::i18n::text("Die Bildschirmaufnahme ist für Windows eingerichtet.", "Screen capture is configured for Windows.").into());
        }
        if !(1..=240).contains(&config.fps) {
            return Err(crate::i18n::text("Die Bildrate muss zwischen 1 und 240 FPS liegen.", "Frame rate must be between 1 and 240 FPS.").into());
        }
        fs::create_dir_all(&config.clips_dir)
            .map_err(|error| crate::tr_format!("Der Clip-Ordner konnte nicht erstellt werden: {error}", "Could not create clips folder: {error}"))?;
        if config.min_free_bytes > 0 {
            if let Some(free) = crate::platform::free_bytes(&config.clips_dir) {
                if free < config.min_free_bytes {
                    return Err(crate::tr_format!(
                        "Auf dem Laufwerk des Clip-Ordners sind nur noch {} frei. Mindestens {} werden verlangt.", "The clips drive has only {} free. At least {} is required.",
                        format_bytes(free),
                        format_bytes(config.min_free_bytes)
                    ));
                }
            }
        }
        let clips_dir = plain_path(
            &fs::canonicalize(&config.clips_dir)
                .map_err(|error| crate::tr_format!("Der Clip-Ordner ist nicht verfügbar: {error}", "Clips folder unavailable: {error}"))?,
        );
        let dir = buffer_dir(&clips_dir);
        prepare_buffer_dir(&dir)?;
        let marker = serde_json::to_vec(&SessionMarker {
            keep_full_session: config.keep_full_session,
        })
        .unwrap_or_default();
        let _ = fs::write(dir.join("session.json"), marker);
        let list_path = dir.join("segments.csv");
        let log_path = dir.join("ffmpeg.log");
        let strategies = if let Some(window) = &config.window {
            if !config.capabilities.gfxcapture {
                return Err(crate::i18n::text("Diese FFmpeg-Version bietet keine Fensteraufnahme. Wähle eine Version mit gfxcapture in den Einstellungen.", "This FFmpeg version does not support window capture. Choose a build with gfxcapture in Settings.").into());
            }
            if crate::games::window_size(window.hwnd, window.pid).is_none() {
                return Err(crate::i18n::text("Das gewählte Fenster ist geschlossen oder minimiert.", "The selected window is closed or minimized.").into());
            }
            let mut encoders = vec![config.encoder, Encoder::Cpu];
            encoders.dedup();
            encoders
                .into_iter()
                .map(|encoder| capture::Strategy {
                    source: capture::VideoSource::Window { hwnd: window.hwnd },
                    encoder,
                })
                .collect()
        } else {
            capture::strategies(&config.display, config.encoder, config.capabilities.ddagrab)
        };
        let (width, height) = capture::output_size(&config.display, config.quality);
        let mut failures = Vec::new();
        for strategy in &strategies {
            let mut warnings = Vec::new();
            let audio = Self::start_audio(&config, &mut warnings)?;
            // Audio activation can take seconds. Recheck the HWND's owner after
            // that wait so a closed/reused handle cannot select another app.
            if config
                .window
                .as_ref()
                .is_some_and(|w| crate::games::window_size(w.hwnd, w.pid).is_none())
            {
                return Err(crate::i18n::text("Das Aufnahmefenster ist nicht mehr verfügbar.", "The capture window is no longer available.").into());
            }
            let inputs: Vec<AudioInput> = audio.iter().map(|pipe| pipe.input.clone()).collect();
            let _ = fs::remove_file(&list_path);
            let log = File::create(&log_path).map_err(|error| {
                crate::tr_format!("Das Aufnahmeprotokoll konnte nicht geöffnet werden: {error}", "Could not open recording log: {error}")
            })?;
            let mut command = capture::build_command(
                &self.ffmpeg,
                &CaptureRequest {
                    display: &config.display,
                    strategy,
                    fps: config.fps,
                    quality: config.quality,
                    show_cursor: config.show_cursor,
                    audio: &inputs,
                    separate_microphone: config.microphone_separate_track && inputs.len() > 1,
                    segment_dir: &dir,
                    list_path: &list_path,
                },
            );
            let description = capture::describe(&command);
            let started = Instant::now();
            let child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::from(log))
                .spawn();
            let mut child = match child {
                Ok(child) => child,
                Err(error) => {
                    failures.push(format!("{}: {error}", strategy.label()));
                    continue;
                }
            };
            // If Stillclip is killed, Windows ends this recorder with it.
            capture::adopt_child(&child);
            // Give FFmpeg a moment: device or encoder failures surface immediately.
            let deadline = Instant::now() + Duration::from_millis(2000);
            let mut died = None;
            while Instant::now() < deadline {
                if config
                    .window
                    .as_ref()
                    .is_some_and(|w| crate::games::window_size(w.hwnd, w.pid).is_none())
                {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(
                        crate::i18n::text("Das Aufnahmefenster wurde während des Starts geschlossen oder minimiert.", "The capture window closed or minimized while recording was starting.")
                            .into(),
                    );
                }
                match child.try_wait() {
                    Ok(Some(status)) => {
                        died = Some(status);
                        break;
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(50)),
                    Err(error) => {
                        died = None;
                        failures.push(format!("{}: {error}", strategy.label()));
                        let _ = child.kill();
                        break;
                    }
                }
            }
            if let Some(status) = died {
                let detail = log_detail(&log_path);
                failures.push(format!("{} ({status}): {detail}", strategy.label()));
                drop(audio);
                continue;
            }
            if child.try_wait().ok().flatten().is_some() {
                drop(audio);
                continue;
            }
            let info = SessionInfo {
                strategy: strategy.label(),
                width,
                height,
                audio: audio.iter().map(|p| p.source.label().to_owned()).collect(),
                warnings,
            };
            let _ = fs::write(dir.join("command.txt"), description);
            self.session = Some(Session {
                window: config.window.clone(),
                child,
                started,
                dir,
                log_path,
                list_path,
                audio,
                keep_full: config.keep_full_session,
                replay_seconds: config.replay_seconds,
                fps: config.fps,
                clips_dir,
                info: info.clone(),
                last_clip: None,
                precise: config.precise_clips,
                encoder: strategy.encoder,
                rates: {
                    let (cw, ch, mbps) = config.quality.configured();
                    crate::encoding::CaptureEncoding::for_output(
                        width, height, config.fps, cw, ch, mbps,
                    )
                },
                min_free_bytes: config.min_free_bytes,
                ticks: 0,
            });
            return Ok(info);
        }
        let _ = fs::remove_dir_all(&dir);
        Err(crate::tr_format!(
            "Die Bildschirmaufnahme konnte nicht gestartet werden. {}", "Could not start screen capture. {}",
            failures.join(" · ")
        ))
    }

    /// Call periodically while recording: surfaces failures, watches the free
    /// space and trims old segments.
    pub fn poll(&mut self) -> Option<PollIssue> {
        let session = self.session.as_mut()?;
        if session
            .window
            .as_ref()
            .is_some_and(|window| crate::games::window_size(window.hwnd, window.pid).is_none())
        {
            return Some(PollIssue { message: crate::i18n::text("Das Aufnahmefenster wurde geschlossen oder minimiert. Die Aufnahme wird beendet.", "The capture window closed or minimized. Stopping recording.").into(), capture_lost: false, stop_requested: true });
        }
        let keep_full = session.keep_full;
        match session.child.try_wait() {
            Ok(None) => {}
            Ok(Some(status)) => {
                let session = self.session.take()?;
                let detail = log_detail(&session.log_path);
                // Without full-session mode the buffer is worthless now; clearing
                // it lets a fresh capture start on the spot.
                if !keep_full {
                    let _ = fs::remove_dir_all(&session.dir);
                }
                return Some(PollIssue {
                    message: crate::tr_format!(
                        "Die Bildschirmaufnahme wurde unerwartet beendet ({status}). {detail}", "Screen capture stopped unexpectedly ({status}). {detail}"
                    ),
                    capture_lost: !keep_full,
                    stop_requested: false,
                });
            }
            Err(error) => {
                if let Some(mut session) = self.session.take() {
                    let _ = session.child.kill();
                    let _ = session.child.wait();
                }
                return Some(PollIssue {
                    message: crate::tr_format!("Der Aufnahmestatus konnte nicht geprüft werden: {error}", "Could not check recording status: {error}"),
                    capture_lost: false,
                    stop_requested: false,
                });
            }
        }
        let session = self.session.as_mut()?;
        session.ticks = session.ticks.wrapping_add(1);
        // Roughly every ten seconds: a full drive would corrupt the recording
        // and leave Windows without room, so stop while there is still space.
        if session.min_free_bytes > 0 && session.ticks % 20 == 0 {
            if let Some(free) = crate::platform::free_bytes(&session.clips_dir) {
                if free < session.min_free_bytes {
                    return Some(PollIssue {
                        message: crate::tr_format!(
                            "Auf dem Laufwerk sind nur noch {} frei. Die Aufnahme wird beendet.", "The drive has only {} free. Stopping recording.",
                            format_bytes(free)
                        ),
                        capture_lost: false,
                        stop_requested: true,
                    });
                }
            }
        }
        if !session.keep_full {
            let segments = parse_segment_list(
                &fs::read_to_string(&session.list_path).unwrap_or_default(),
                &session.dir,
            );
            for expired in expired_segments(&segments, session.replay_seconds) {
                let _ = fs::remove_file(expired.path);
            }
        }
        None
    }

    /// Stops recording. Returns the full-session video when that mode is on.
    pub fn stop(&mut self) -> Result<Option<PathBuf>, String> {
        let mut session = self.session.take().ok_or(crate::i18n::text("Es läuft keine Aufnahme.", "No recording is running."))?;
        let status = finish_process(&mut session.child, Duration::from_secs(30));
        for pipe in &mut session.audio {
            pipe.stop();
        }
        let status = status
            .map_err(|error| crate::tr_format!("Die Aufnahme konnte nicht abgeschlossen werden: {error}", "Could not finalize recording: {error}"))?;
        if !status.success() {
            let detail = log_detail(&session.log_path);
            if !session.keep_full {
                let _ = fs::remove_dir_all(&session.dir);
            }
            return Err(crate::tr_format!(
                "FFmpeg hat die Aufnahme nicht sauber beendet ({status}). {detail}", "FFmpeg did not finish recording cleanly ({status}). {detail}"
            ));
        }
        let mut saved = None;
        if session.keep_full {
            let segments = scan_segments(&session.dir, &session.list_path);
            if segments.is_empty() {
                let _ = fs::remove_dir_all(&session.dir);
                return Err(crate::i18n::text("Die Aufnahme hat keine Videodaten erzeugt.", "The recording produced no video data.").into());
            }
            let output = clip_path(&session.clips_dir, crate::i18n::text("Aufnahme", "Recording"));
            self.export_all(&segments, &output)?;
            saved = Some(output);
        }
        let _ = fs::remove_dir_all(&session.dir);
        Ok(saved)
    }

    /// Saves the last `seconds` of the running recording as a new clip.
    /// The recording keeps running; nothing is re-encoded unless precise clips
    /// are enabled. `prefix` names the file, e.g. the detected game.
    pub fn prepare_replay(
        &mut self,
        seconds: u32,
        prefix: &str,
    ) -> Result<crate::export::ReplayExport, String> {
        if seconds == 0 {
            return Err(crate::i18n::text("Die Cliplänge muss mindestens eine Sekunde betragen.", "Clip length must be at least one second.").into());
        }
        let session = self
            .session
            .as_mut()
            .ok_or(crate::i18n::text("Starte zuerst eine Aufnahme.", "Start recording first."))?;
        if session.started.elapsed() < Duration::from_millis(1500) {
            return Err(crate::i18n::text("Die Aufnahme ist noch zu kurz. Warte einen Moment.", "The recording is still too short. Please wait a moment.").into());
        }
        if session
            .last_clip
            .is_some_and(|at| at.elapsed() < CLIP_COOLDOWN)
        {
            return Err(crate::i18n::text("Der letzte Clip wurde gerade erst gesichert.", "The last clip was only just saved.").into());
        }
        let segments = scan_segments(&session.dir, &session.list_path);
        if segments.is_empty() {
            return Err(crate::i18n::text("Es liegen noch keine Videodaten vor. Warte einen Moment.", "No video data is available yet. Please wait a moment.").into());
        }
        let gop = 0.4_f64.max(f64::from(session.fps.max(1)).recip() * 4.0);
        let elapsed = session.started.elapsed().as_secs_f64();
        let finished_end = segments
            .iter()
            .filter_map(|s| s.end)
            .fold(0.0_f64, f64::max);
        let estimate = (elapsed - finished_end).clamp(0.0, f64::from(SEGMENT_SECONDS) + 1.0);
        let chosen = tail_segments(&segments, f64::from(seconds) + gop, estimate);
        let prefix = sanitize_prefix(prefix);
        let output = clip_path(&session.clips_dir, &prefix);
        let snapshot = crate::export::Snapshot::new(&session.clips_dir, &chosen)?;
        session.last_clip = Some(Instant::now());
        Ok(crate::export::ReplayExport {
            snapshot,
            output,
            ffmpeg: self.ffmpeg.clone(),
            ffprobe: self.ffprobe.clone(),
            span: None,
            seconds: f64::from(seconds),
            gop,
            precise: session.precise,
            encoder: session.encoder,
            rates: session.rates.clone(),
        })
    }

    #[cfg(test)]
    pub fn save_replay(&mut self, seconds: u32, prefix: &str) -> Result<SavedClip, String> {
        self.prepare_replay(seconds, prefix)?.run()
    }

    /// Seconds of finished video the buffer currently holds, as `(from, to)`
    /// on the session timeline. The UI uses it to tell whether a marked moment
    /// can still be saved before the ring deletes it.
    pub fn buffered_span(&self) -> Option<(f64, f64)> {
        let session = self.session.as_ref()?;
        available_span(&session.dir, &session.list_path)
    }

    /// Saves the window around a marked moment. `at` is seconds since this
    /// session started; `before` and `after` extend the clip on both sides.
    /// The recording keeps running, and nothing is re-encoded unless exact
    /// clips are switched on.
    pub fn prepare_marker(
        &mut self,
        at: f64,
        before: f64,
        after: f64,
        prefix: &str,
    ) -> Result<crate::export::ReplayExport, String> {
        let session = self
            .session
            .as_mut()
            .ok_or(crate::i18n::text("Starte zuerst eine Aufnahme.", "Start recording first."))?;
        if session
            .last_clip
            .is_some_and(|at| at.elapsed() < CLIP_COOLDOWN)
        {
            return Err(crate::i18n::text("Der letzte Clip wurde gerade erst gesichert.", "The last clip was only just saved.").into());
        }
        let segments = scan_segments(&session.dir, &session.list_path);
        let earliest = segments
            .iter()
            .filter_map(|s| s.start)
            .fold(f64::INFINITY, f64::min);
        let latest = segments
            .iter()
            .filter_map(|s| s.end)
            .fold(f64::NEG_INFINITY, f64::max);
        if !earliest.is_finite() || latest <= earliest {
            return Err(crate::i18n::text("Es liegen noch keine fertigen Videodaten vor. Warte einen Moment.", "No completed video data is available yet. Please wait a moment.").into());
        }
        if at + after > latest + 0.001 {
            return Err(
                crate::i18n::text("Der Moment ist noch nicht vollständig aufgezeichnet. Warte einen Moment.", "The moment has not finished recording. Please wait a moment.").into(),
            );
        }
        let from = (at - before).max(earliest);
        let to = (at + after).min(latest);
        if to - from < 0.5 {
            return Err(crate::i18n::text("Die Markierung liegt nicht mehr im Puffer.", "This marker is no longer in the buffer.").into());
        }
        let chosen = span_segments(&segments, from, to);
        if chosen.is_empty() {
            return Err(crate::i18n::text("Die Markierung liegt nicht mehr im Puffer.", "This marker is no longer in the buffer.").into());
        }
        let gop = 0.4_f64.max(f64::from(session.fps.max(1)).recip() * 4.0);
        let output = clip_path(&session.clips_dir, &sanitize_prefix(prefix));
        let snapshot = crate::export::Snapshot::new(&session.clips_dir, &chosen)?;
        session.last_clip = Some(Instant::now());
        Ok(crate::export::ReplayExport {
            snapshot,
            output,
            ffmpeg: self.ffmpeg.clone(),
            ffprobe: self.ffprobe.clone(),
            span: Some((from, to)),
            seconds: to - from,
            gop,
            precise: session.precise,
            encoder: session.encoder,
            rates: session.rates.clone(),
        })
    }

    #[cfg(test)]
    pub fn save_marker(
        &mut self,
        at: f64,
        before: f64,
        after: f64,
        prefix: &str,
    ) -> Result<SavedClip, String> {
        self.prepare_marker(at, before, after, prefix)?.run()
    }

    fn export_all(&self, segments: &[Segment], output: &Path) -> Result<(), String> {
        let dir = segments[0]
            .path
            .parent()
            .ok_or(crate::i18n::text("Der Pufferordner fehlt.", "The buffer folder is missing."))?
            .to_path_buf();
        let list_path = dir.join("export-list.txt");
        let mut list = String::new();
        for segment in segments {
            let escaped = ffmpeg_path(&segment.path).replace('\'', "'\\''");
            list.push_str(&format!("file '{escaped}'\n"));
        }
        fs::write(&list_path, list)
            .map_err(|error| crate::tr_format!("Die Exportliste konnte nicht geschrieben werden: {error}", "Could not write export list: {error}"))?;
        let result = command(&self.ffmpeg)
            .current_dir(&dir)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
            ])
            .arg(&list_path)
            .args(["-c", "copy", "-movflags", "+faststart", "-f", "mp4"])
            .arg(output)
            .output()
            .map_err(|error| crate::tr_format!("Die Aufnahme konnte nicht gespeichert werden: {error}", "Could not save recording: {error}"))?;
        if !result.status.success() || fs::metadata(output).map_or(true, |m| m.len() == 0) {
            let _ = fs::remove_file(output);
            return Err(crate::tr_format!(
                "Die Aufnahme konnte nicht gespeichert werden. {}", "Could not save recording. {}",
                diagnostic(&result.stderr)
            ));
        }
        Ok(())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(mut session) = self.session.take() {
            let _ = finish_process(&mut session.child, Duration::from_secs(5));
            for pipe in &mut session.audio {
                pipe.stop();
            }
            if !session.keep_full {
                let _ = fs::remove_dir_all(&session.dir);
            }
        }
    }
}

fn prepare_buffer_dir(dir: &Path) -> Result<(), String> {
    if dir.is_dir() {
        let _ = fs::remove_dir_all(dir);
    }
    fs::create_dir_all(dir)
        .map_err(|error| crate::tr_format!("Der Pufferordner konnte nicht erstellt werden: {error}", "Could not create buffer folder: {error}"))?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN};
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN);
        }
    }
    Ok(())
}

/// Segments left behind by a crash. With full-session mode they become a
/// recovered recording; otherwise they are simply removed.
pub fn recover_leftovers(ffmpeg: &Path, clips_dir: &Path) -> Result<Option<PathBuf>, String> {
    let _ = fs::remove_dir_all(clips_dir.join(".stillclip-exports"));
    let dir = buffer_dir(clips_dir);
    if !dir.is_dir() {
        return Ok(None);
    }
    let marker: Option<SessionMarker> = fs::read(dir.join("session.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let segments = scan_segments(&dir, &dir.join("segments.csv"));
    let mut saved = None;
    if marker.is_some_and(|m| m.keep_full_session) && !segments.is_empty() {
        let recorder = Recorder::new(ffmpeg.to_path_buf());
        let output = clip_path(clips_dir, crate::i18n::text("Wiederhergestellte Aufnahme", "Recovered recording"));
        recorder.export_all(&segments, &output)?;
        saved = Some(output);
    }
    let _ = fs::remove_dir_all(&dir);
    Ok(saved)
}

#[derive(Debug, Default, Clone, Copy)]
struct Range {
    start: f64,
    duration: f64,
}

fn probe_range(ffprobe: &Path, path: &Path) -> Option<Range> {
    let output = command(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=start_time,duration",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut range = Range::default();
    let mut seen = 0;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("start_time=") {
            range.start = value.trim().parse().ok()?;
            seen += 1;
        } else if let Some(value) = line.strip_prefix("duration=") {
            range.duration = value.trim().parse().ok()?;
            seen += 1;
        }
    }
    (seen == 2 && range.duration.is_finite() && range.start.is_finite()).then_some(range)
}

/// Presentation times of video keyframes in the file's own timeline.
fn keyframes(ffprobe: &Path, path: &Path) -> Vec<f64> {
    keyframes_in(ffprobe, path, None)
}

/// The keyframe at or before `at`, or `at` itself when none can be read.
/// Only a short window around the spot is scanned, so this stays quick even
/// on a long video.
pub fn keyframe_before(ffprobe: &Path, path: &Path, at: f64) -> f64 {
    let from = (at - 12.0).max(0.0);
    keyframes_in(ffprobe, path, Some((from, at + 0.5)))
        .into_iter()
        .filter(|key| *key <= at + 0.0005)
        .fold(None, |best: Option<f64>, key| {
            Some(best.map_or(key, |b| b.max(key)))
        })
        .unwrap_or(at)
}

/// Presentation time of the first video packet, which a stream copy shifts so
/// the earliest packet of any stream lands at zero.
fn first_video_pts(ffprobe: &Path, path: &Path) -> Option<f64> {
    let output = command(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-read_intervals",
            "%+1.0",
            "-show_entries",
            "packet=pts_time",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').parse::<f64>().ok())
        .fold(None, |best: Option<f64>, pts| {
            Some(best.map_or(pts, |b| b.min(pts)))
        })
}

/// Keyframe times, optionally limited to `from..to` seconds of the file.
fn keyframes_in(ffprobe: &Path, path: &Path, window: Option<(f64, f64)>) -> Vec<f64> {
    let mut probe = command(ffprobe);
    probe.args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "packet=pts_time,flags",
        "-of",
        "csv=p=0",
    ]);
    if let Some((from, to)) = window {
        probe
            .arg("-read_intervals")
            .arg(format!("{from:.3}%+{:.3}", (to - from).max(0.1)));
    }
    let Ok(output) = probe.arg(path).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (pts, flags) = line.trim().split_once(',')?;
            flags
                .starts_with('K')
                .then(|| pts.parse::<f64>().ok())
                .flatten()
        })
        .collect()
}

/// Cuts the newest `seconds` out of `segments` into an MP4 without re-encoding.
/// Segments that overlap `[from, to]` on the CSV timeline. Only finished
/// segments take part: a marker is clipped once its whole window is written.
pub fn span_segments(segments: &[Segment], from: f64, to: f64) -> Vec<Segment> {
    segments
        .iter()
        .filter(|segment| match (segment.start, segment.end) {
            (Some(start), Some(end)) => end > from + 0.001 && start < to - 0.001,
            _ => false,
        })
        .cloned()
        .collect()
}

/// Cuts `[from, to]` of the buffered timeline into an MP4 without re-encoding.
/// Both bounds are seconds on the CSV timeline. The cut moves back to the
/// keyframe at or before `from`, so the clip may start a little earlier but
/// never ends early.
/// Returns where the finished clip begins on that timeline, which is at or
/// shortly before `from`.
pub(crate) fn export_span(
    ffmpeg: &Path,
    ffprobe: Option<&Path>,
    segments: &[Segment],
    from: f64,
    to: f64,
    gop: f64,
    output: &Path,
) -> Result<f64, String> {
    let first = segments
        .first()
        .ok_or(crate::i18n::text("Keine Segmente zum Sichern vorhanden.", "No segments available to save."))?;
    let dir = first.path.parent().ok_or(crate::i18n::text("Der Pufferordner fehlt.", "The buffer folder is missing."))?;
    if to <= from {
        return Err(crate::i18n::text("Der gewünschte Ausschnitt ist leer.", "The requested selection is empty.").into());
    }
    // The concat input keeps the recording's own timestamps, so every offset is
    // measured from the first chosen segment's start.
    let base = first.start.unwrap_or(0.0);
    let mut cut = (from - base).max(0.0);
    if let Some(ffprobe) = ffprobe {
        if let Some(first_range) = probe_range(ffprobe, &first.path) {
            let target = first_range.start + cut;
            let mut starts = Vec::with_capacity(segments.len());
            let mut previous_end = base;
            for segment in segments {
                let csv_start = segment.start.unwrap_or(previous_end);
                starts.push(first_range.start + (csv_start - base));
                previous_end = segment.end.unwrap_or(csv_start);
            }
            let containing = starts
                .iter()
                .rposition(|start| *start <= target)
                .unwrap_or(0);
            let key = keyframes(ffprobe, &segments[containing].path)
                .into_iter()
                .filter(|key| *key <= target + 0.0005)
                .fold(None, |best: Option<f64>, key| {
                    Some(best.map_or(key, |b| b.max(key)))
                })
                .unwrap_or_else(|| starts[containing].max(first_range.start));
            cut = (key - first_range.start).max(0.0);
        }
    } else {
        // Without ffprobe begin a keyframe distance earlier instead of guessing.
        cut = (cut - gop).max(0.0);
    }
    let length = (to - base) - cut;
    if length < 0.2 {
        return Err(crate::i18n::text("Der gewünschte Ausschnitt ist zu kurz.", "The requested selection is too short.").into());
    }
    let input = format!(
        "concat:{}",
        segments
            .iter()
            .map(|s| s
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned())
            .collect::<Vec<_>>()
            .join("|")
    );
    let mut export = command(ffmpeg);
    export
        .current_dir(dir)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    if cut > 0.05 {
        export.arg("-ss").arg(format!("{cut:.6}"));
    }
    let result = export
        .args(["-i", &input, "-t"])
        .arg(format!("{length:.6}"))
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a?",
            "-c",
            "copy",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(output)
        .output()
        .map_err(|error| crate::tr_format!("Der Clip konnte nicht gespeichert werden: {error}", "Could not save clip: {error}"))?;
    if !result.status.success() {
        return Err(crate::tr_format!(
            "Der Clip konnte nicht gespeichert werden. {}", "Could not save clip. {}",
            diagnostic(&result.stderr)
        ));
    }
    if fs::metadata(output).map_or(true, |metadata| metadata.len() == 0) {
        return Err(crate::i18n::text("Der Clip enthält keine Videodaten.", "The clip contains no video data.").into());
    }
    Ok(base + cut)
}

/// Returns where the finished clip begins on the CSV timeline.
pub(crate) fn export_tail(
    ffmpeg: &Path,
    ffprobe: Option<&Path>,
    segments: &[Segment],
    seconds: f64,
    gop: f64,
    output: &Path,
) -> Result<f64, String> {
    let first = segments
        .first()
        .ok_or(crate::i18n::text("Keine Segmente zum Sichern vorhanden.", "No segments available to save."))?;
    let last = segments
        .last()
        .ok_or(crate::i18n::text("Keine Segmente zum Sichern vorhanden.", "No segments available to save."))?;
    let dir = first.path.parent().ok_or(crate::i18n::text("Der Pufferordner fehlt.", "The buffer folder is missing."))?;
    let mut seek = None;
    if let Some(ffprobe) = ffprobe {
        if let (Some(first_range), Some(last_range)) = (
            probe_range(ffprobe, &first.path),
            probe_range(ffprobe, &last.path),
        ) {
            let end = last_range.start + last_range.duration;
            let target = end - seconds;
            if target > first_range.start + 0.05 {
                // Estimate where each segment starts in the file timeline from the
                // list's offsets, then find the keyframe at or before the target so
                // audio and video begin together and the clip is never short.
                let base = first.start.unwrap_or(0.0);
                let mut starts = Vec::with_capacity(segments.len());
                let mut previous_end = base;
                for segment in segments {
                    let csv_start = segment.start.unwrap_or(previous_end);
                    starts.push(first_range.start + (csv_start - base));
                    previous_end = segment.end.unwrap_or(csv_start);
                }
                let containing = starts
                    .iter()
                    .rposition(|start| *start <= target)
                    .unwrap_or(0);
                let cut = keyframes(ffprobe, &segments[containing].path)
                    .into_iter()
                    .filter(|key| *key <= target + 0.0005)
                    .fold(None, |best: Option<f64>, key| {
                        Some(best.map_or(key, |b| b.max(key)))
                    })
                    .unwrap_or_else(|| (target - gop).max(first_range.start));
                let relative = cut - first_range.start - 0.001;
                if relative > 0.0 {
                    seek = Some(relative);
                }
            }
        }
    }
    if seek.is_none() && ffprobe.is_none() {
        // Without ffprobe fall back to the listed durations and a keyframe margin.
        let total: f64 = segments.iter().filter_map(Segment::duration).sum::<f64>()
            + segments.iter().filter(|s| s.duration().is_none()).count() as f64
                * f64::from(SEGMENT_SECONDS)
                * 0.5;
        let relative = total - seconds - gop;
        if relative > 0.5 {
            seek = Some(relative);
        }
    }
    let input = format!(
        "concat:{}",
        segments
            .iter()
            .map(|s| s
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned())
            .collect::<Vec<_>>()
            .join("|")
    );
    let mut export = command(ffmpeg);
    export
        .current_dir(dir)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    if let Some(seek) = seek {
        export.arg("-ss").arg(format!("{seek:.6}"));
    }
    let result = export
        .args([
            "-i",
            &input,
            "-map",
            "0:v:0",
            "-map",
            "0:a?",
            "-c",
            "copy",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(output)
        .output()
        .map_err(|error| crate::tr_format!("Der Clip konnte nicht gespeichert werden: {error}", "Could not save clip: {error}"))?;
    if !result.status.success() {
        return Err(crate::tr_format!(
            "Der Clip konnte nicht gespeichert werden. {}", "Could not save clip. {}",
            diagnostic(&result.stderr)
        ));
    }
    if fs::metadata(output).map_or(true, |metadata| metadata.len() == 0) {
        return Err(crate::i18n::text("Der Clip enthält keine Videodaten.", "The clip contains no video data.").into());
    }
    Ok(first.start.unwrap_or(0.0) + seek.unwrap_or(0.0))
}

fn finish_process(
    child: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
    match child.try_wait() {
        Ok(Some(status)) => return Ok(status),
        Ok(None) => {}
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.to_string());
        }
    }
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(b"q\n");
        let _ = input.flush();
    }
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(40)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(crate::i18n::text("FFmpeg hat nicht rechtzeitig reagiert.", "FFmpeg did not respond in time.").into());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    }
}

/// Byte counts for messages: `4,7 GB`, `820 MB`.
pub fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let megabytes = bytes as f64 / MB;
    if megabytes >= 1024.0 {
        format!("{:.1} GB", megabytes / 1024.0).replace('.', ",")
    } else {
        format!("{} MB", megabytes.round() as u64)
    }
}

/// Removes characters Windows does not allow in file names.
pub fn sanitize_prefix(prefix: &str) -> String {
    let cleaned: String = prefix
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = cleaned.trim_end_matches('.').trim().to_owned();
    if trimmed.is_empty() {
        "Clip".into()
    } else {
        trimmed.chars().take(60).collect()
    }
}

/// Re-encodes the tail of `path` in place so the clip lasts exactly `seconds`.
/// Returns the seconds that were dropped from the front.
pub(crate) fn trim_to_exact_length(
    ffmpeg: &Path,
    path: &Path,
    seconds: f64,
    encoder: Encoder,
    rates: &crate::encoding::CaptureEncoding,
) -> Result<f64, String> {
    let duration = probe_duration(ffmpeg, path)?;
    let excess = duration - seconds;
    if excess < 0.04 {
        return Ok(0.0);
    }
    let temporary = path.with_extension("exact.mp4");
    let mut encode = command(ffmpeg);
    encode
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-ss",
        ])
        .arg(format!("{excess:.6}"))
        .arg("-i")
        .arg(path)
        .args(["-map", "0:v:0", "-map", "0:a?"]);
    capture::encoder_args(&mut encode, encoder, rates, false);
    let result = encode
        .args(["-c:a", "copy", "-movflags", "+faststart", "-f", "mp4"])
        .arg(&temporary)
        .output()
        .map_err(|error| crate::tr_format!("Der Clip konnte nicht exakt zugeschnitten werden: {error}", "Could not trim clip precisely: {error}"))?;
    if !result.status.success() || fs::metadata(&temporary).map_or(true, |m| m.len() == 0) {
        let _ = fs::remove_file(&temporary);
        return Err(crate::tr_format!(
            "Der Clip konnte nicht exakt zugeschnitten werden. {}", "Could not trim clip precisely. {}",
            diagnostic(&result.stderr)
        ));
    }
    fs::rename(&temporary, path)
        .map(|()| excess)
        .map_err(|error| {
            let _ = fs::remove_file(&temporary);
            crate::tr_format!("Der exakte Clip konnte nicht gespeichert werden: {error}", "Could not save precisely trimmed clip: {error}")
        })
}

/// `Clip 2026-11-14 02-21-33.mp4`, with a counter when the name is taken.
pub fn clip_path(directory: &Path, prefix: &str) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    let base = format!("{prefix} {stamp}");
    let mut candidate = directory.join(format!("{base}.mp4"));
    let mut number = 2;
    while candidate.exists() {
        candidate = directory.join(format!("{base} ({number}).mp4"));
        number += 1;
    }
    candidate
}

pub(crate) fn diagnostic(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<_> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(5)..]
        .join(" ")
        .chars()
        .take(1600)
        .collect()
}

fn log_detail(path: &Path) -> String {
    fs::read(path)
        .map(|bytes| diagnostic(&bytes))
        .unwrap_or_default()
}

/// Facts about a media file for the library and the editor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub audio: bool,
}

pub fn probe_media(ffmpeg: &Path, input: &Path) -> Result<MediaInfo, String> {
    if let Some(ffprobe) = ffprobe_for(ffmpeg) {
        let output = command(&ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=codec_type,width,height",
                "-of",
                "json",
            ])
            .arg(input)
            .output()
            .map_err(|error| crate::tr_format!("Die Videodaten konnten nicht gelesen werden: {error}", "Could not read video data: {error}"))?;
        if output.status.success() {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                let duration = value["format"]["duration"]
                    .as_str()
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let mut info = MediaInfo {
                    duration,
                    ..MediaInfo::default()
                };
                for stream in value["streams"].as_array().into_iter().flatten() {
                    match stream["codec_type"].as_str() {
                        Some("video") if info.width == 0 => {
                            info.width = stream["width"].as_u64().unwrap_or(0) as u32;
                            info.height = stream["height"].as_u64().unwrap_or(0) as u32;
                        }
                        Some("audio") => info.audio = true,
                        _ => {}
                    }
                }
                if info.duration.is_finite() && info.duration > 0.0 && info.width > 0 {
                    return Ok(info);
                }
            }
        }
    }
    let output = command(ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(input)
        .output()
        .map_err(|error| crate::tr_format!("Die Videodaten konnten nicht gelesen werden: {error}", "Could not read video data: {error}"))?;
    let text = String::from_utf8_lossy(&output.stderr);
    let duration = parse_duration(&text).ok_or_else(|| {
        crate::tr_format!(
            "Die Datei enthält kein lesbares Video. {}", "The file contains no readable video. {}",
            diagnostic(&output.stderr)
        )
    })?;
    let (width, height) = parse_dimensions(&text).unwrap_or((0, 0));
    Ok(MediaInfo {
        duration,
        width,
        height,
        audio: text.contains("Audio:"),
    })
}

/// Returns the actual media duration, using FFprobe when available.
pub fn probe_duration(ffmpeg: &Path, input: &Path) -> Result<f64, String> {
    probe_media(ffmpeg, input).map(|info| info.duration)
}

fn parse_duration(text: &str) -> Option<f64> {
    let duration = text.split("Duration: ").nth(1)?.split(',').next()?.trim();
    let mut parts = duration.split(':');
    let hours = parts.next()?.parse::<f64>().ok()?;
    let minutes = parts.next()?.parse::<f64>().ok()?;
    let seconds = parts.next()?.parse::<f64>().ok()?;
    let total = hours * 3600.0 + minutes * 60.0 + seconds;
    (total.is_finite() && total > 0.0).then_some(total)
}

fn parse_dimensions(text: &str) -> Option<(u32, u32)> {
    let video = text.split("Video:").nth(1)?;
    video
        .split(|c: char| c == ',' || c == ' ')
        .filter_map(|token| {
            let (w, h) = token.split_once('x')?;
            Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))
        })
        .find(|(w, h)| *w > 0 && *h > 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimMode {
    /// Re-encode: exact boundaries, slower.
    Precise,
    /// Stream copy: starts at the previous keyframe, instant.
    Fast,
}

/// Exports a section as a new MP4; the source file is never modified.
/// Returns where the new clip begins inside the source, which in fast mode is
/// the keyframe at or before `start`.
pub fn trim_clip(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    start: f64,
    end: f64,
    mode: TrimMode,
) -> Result<f64, String> {
    trim_controlled(
        ffmpeg,
        input,
        output,
        start,
        end,
        mode,
        None,
        &crate::jobs::Control::new(),
    )
}

pub fn trim_controlled(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    start: f64,
    end: f64,
    mode: TrimMode,
    crop: Option<crate::portrait::Crop>,
    control: &crate::jobs::Control,
) -> Result<f64, String> {
    control.check()?;
    let crop = crop.map(|c| c.validate()).transpose()?;
    let mode = if crop.is_some() {
        TrimMode::Precise
    } else {
        mode
    };
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
        return Err(crate::i18n::text("Wähle einen gültigen Clipbereich mit Ende nach dem Anfang.", "Choose a valid clip range with the end after the start.").into());
    }
    if !input.is_file() {
        return Err(crate::i18n::text("Die ausgewählte Videodatei wurde nicht gefunden.", "The selected video file was not found.").into());
    }
    if output.exists() {
        return Err(crate::i18n::text("Die Zieldatei existiert bereits. Wähle einen neuen Dateinamen.", "The output file already exists. Choose a different filename.").into());
    }
    let duration = probe_duration(ffmpeg, input)?;
    if start >= duration || end > duration + 0.1 {
        return Err(crate::tr_format!(
            "Der Clipbereich liegt außerhalb des Videos ({duration:.1} Sekunden).", "The selected range is outside the video ({duration:.1} seconds)."
        ));
    }
    // Exclusively reserve our destination before FFmpeg writes it. On failure only
    // this owned file is removed; existing user files are never overwritten or deleted.
    let destination = File::options()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|error| crate::tr_format!("Die Zieldatei konnte nicht angelegt werden: {error}", "Could not create output file: {error}"))?;
    drop(destination);
    let length = end.min(duration) - start;
    let mut export = command(ffmpeg);
    let filter = crop.map_or_else(
        || "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string(),
        |c| c.filter(),
    );
    export.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-filter_threads",
        "2",
    ]);
    match mode {
        TrimMode::Precise => {
            export
                .arg("-ss")
                .arg(format!("{start:.6}"))
                .arg("-i")
                .arg(input)
                .arg("-t")
                .arg(format!("{length:.6}"))
                .args([
                    "-map", "0:v:0", "-map", "0:a?", "-vf", &filter, "-c:v", "libx264", "-preset",
                    "veryfast", "-threads", "4", "-crf", "20", "-pix_fmt", "yuv420p", "-c:a",
                    "aac", "-b:a", "160k",
                ]);
        }
        TrimMode::Fast => {
            export
                .arg("-ss")
                .arg(format!("{start:.6}"))
                .arg("-i")
                .arg(input)
                .arg("-t")
                .arg(format!("{length:.6}"))
                .args([
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a?",
                    "-c",
                    "copy",
                    "-avoid_negative_ts",
                    "make_zero",
                ]);
        }
    }
    export
        .args(["-movflags", "+faststart", "-f", "mp4"])
        .arg(output);
    let result = control
        .run(&mut export, length, crate::i18n::text("Clip kodieren", "Encoding clip"))
        .map_err(|error| {
            let _ = fs::remove_file(output);
            error
        })?;
    if !result.status.success() {
        let _ = fs::remove_file(output);
        return Err(crate::tr_format!(
            "Der Clip konnte nicht exportiert werden. {}", "Could not export clip. {}",
            diagnostic(&result.stderr)
        ));
    }
    if fs::metadata(output).map_or(true, |metadata| metadata.len() == 0) {
        let _ = fs::remove_file(output);
        return Err(crate::i18n::text("Der Export hat keine Videodaten erzeugt.", "The export produced no video data.").into());
    }
    Ok(match mode {
        TrimMode::Precise => start,
        // The copy begins at the keyframe before the cut, and the stream whose
        // first packet lands at zero may sit even earlier: both together say
        // which moment of the source the new clip starts with.
        TrimMode::Fast => ffprobe_for(ffmpeg)
            .map(|ffprobe| {
                let keyframe = keyframe_before(&ffprobe, input, start);
                keyframe - first_video_pts(&ffprobe, output).unwrap_or(0.0)
            })
            .unwrap_or(start),
    })
}

/// Writes a JPEG poster frame for the library.
pub fn make_thumbnail(ffmpeg: &Path, input: &Path, output: &Path, at: f64) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| crate::tr_format!("Vorschauordner fehlt: {error}", "Preview folder is missing: {error}"))?;
    }
    let temporary = output.with_extension("tmp.jpg");
    let result = command(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-ss",
        ])
        .arg(format!("{:.3}", at.max(0.0)))
        .arg("-i")
        .arg(input)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=480:-2",
            "-q:v",
            "4",
            "-f",
            "image2",
        ])
        .arg(&temporary)
        .output()
        .map_err(|error| crate::tr_format!("Vorschau fehlgeschlagen: {error}", "Preview failed: {error}"))?;
    if !result.status.success() || fs::metadata(&temporary).map_or(true, |m| m.len() == 0) {
        let _ = fs::remove_file(&temporary);
        return Err(crate::tr_format!(
            "Vorschau fehlgeschlagen. {}", "Preview failed. {}",
            diagnostic(&result.stderr)
        ));
    }
    fs::rename(&temporary, output)
        .map_err(|error| crate::tr_format!("Vorschau konnte nicht gespeichert werden: {error}", "Could not save preview: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_availability_excludes_segments_already_removed_from_the_ring() {
        let dir = env::temp_dir().join(format!("stillclip-span-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let list = dir.join("segments.csv");
        fs::write(
            &list,
            "seg-000000.ts,0,5\nseg-000001.ts,5,10\nseg-000002.ts,10,15\n",
        )
        .unwrap();
        fs::write(dir.join("seg-000001.ts"), b"video").unwrap();
        fs::write(dir.join("seg-000002.ts"), b"video").unwrap();
        assert_eq!(available_span(&dir, &list), Some((5.0, 15.0)));
        fs::remove_file(dir.join("seg-000001.ts")).unwrap();
        assert_eq!(available_span(&dir, &list), Some((10.0, 15.0)));
        fs::remove_file(dir.join("seg-000002.ts")).unwrap();
        assert_eq!(available_span(&dir, &list), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_media_duration_and_rejects_invalid_media() {
        assert_eq!(
            parse_duration("Duration: 01:02:03.45, start: 0.0"),
            Some(3723.45)
        );
        assert_eq!(parse_duration("Duration: N/A, start: 0.0"), None);
        assert_eq!(parse_duration("Duration: 00:00:NaN, start: 0.0"), None);
        assert_eq!(
            parse_dimensions("Stream #0:0: Video: h264 (High), yuv420p(progressive), 1280x720 [SAR 1:1 DAR 16:9], 60 fps"),
            Some((1280, 720))
        );
    }

    #[test]
    fn recording_must_be_started_before_stop_or_replay() {
        let mut recorder = Recorder::new(PathBuf::from("missing-ffmpeg"));
        assert!(!recorder.is_recording());
        assert!(recorder.stop().is_err());
        assert!(recorder.save_replay(30, "Clip").is_err());
        assert_eq!(
            sanitize_prefix("  Counter-Strike: 2 ?? "),
            "Counter-Strike 2"
        );
        assert_eq!(sanitize_prefix(""), "Clip");
        assert_eq!(sanitize_prefix("..."), "Clip");
    }

    /// The free-space guard has to refuse before FFmpeg is ever started.
    #[test]
    fn recording_refuses_to_start_without_free_space() {
        let dir = env::temp_dir().join(format!("stillclip-space-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let mut recorder = Recorder::new(PathBuf::from("missing-ffmpeg"));
        let config = SessionConfig {
            window: None,
            audio_process: None,
            display: Display {
                name: "\\\\.\\DISPLAY1".into(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                primary: true,
                output_idx: 0,
                adapter_idx: 0,
            },
            encoder: Encoder::Cpu,
            capabilities: Capabilities::default(),
            fps: 60,
            quality: Quality::P720,
            show_cursor: false,
            system_audio: false,
            microphone: None,
            microphone_separate_track: false,
            system_gain: 1.0,
            microphone_gain: 1.0,
            replay_seconds: 30,
            keep_full_session: false,
            min_free_bytes: u64::MAX,
            precise_clips: false,
            clips_dir: dir.clone(),
        };
        let error = recorder.start(config).expect_err("no space, no recording");
        assert!(error.contains("frei"), "{error}");
        assert!(!recorder.is_recording());
        // Nothing was prepared: no buffer folder was left behind.
        assert!(!buffer_dir(&dir).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_invalid_trim_ranges_before_running_ffmpeg() {
        for (start, end) in [(1.0, 0.0), (-1.0, 5.0), (0.0, f64::NAN)] {
            assert!(trim_clip(
                Path::new("missing"),
                Path::new("input"),
                Path::new("output"),
                start,
                end,
                TrimMode::Precise,
            )
            .is_err());
        }
    }

    #[test]
    fn segment_lists_are_parsed_and_tailed() {
        let dir = Path::new("buffer");
        let text = "seg-000000.ts,0.000000,5.016667\nseg-000001.ts,5.016667,10.000000\nbroken line\nseg-000002.ts,10.000000,15.050000\n";
        let segments = parse_segment_list(text, dir);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[1].path, dir.join("seg-000001.ts"));
        assert!((segments[2].duration().unwrap() - 5.05).abs() < 1e-9);
        let mut all = segments.clone();
        all.push(Segment {
            index: 3,
            path: dir.join("seg-000003.ts"),
            start: None,
            end: None,
        });
        let tail = tail_segments(&all, 7.0, 2.0);
        assert_eq!(tail.iter().map(|s| s.index).collect::<Vec<_>>(), vec![2, 3]);
        let tail = tail_segments(&all, 7.1, 2.0);
        assert_eq!(
            tail.iter().map(|s| s.index).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        let tail = tail_segments(&all, 100.0, 2.0);
        assert_eq!(tail.len(), 4);
        let tail = tail_segments(&all, 1.0, 2.0);
        assert_eq!(tail.iter().map(|s| s.index).collect::<Vec<_>>(), vec![3]);
        // Keep 30 s plus two segments of margin: with 15 s recorded nothing expires.
        assert!(expired_segments(&all, 30).is_empty());
        let mut long = Vec::new();
        for index in 0..20_u64 {
            long.push(Segment {
                index,
                path: dir.join(format!("seg-{index:06}.ts")),
                start: Some(index as f64 * 5.0),
                end: Some(index as f64 * 5.0 + 5.0),
            });
        }
        let expired = expired_segments(&long, 30);
        // latest end 100 s; keep from 100 - 30 - 10 = 60 s → segments ending before 60 s.
        assert_eq!(expired.last().map(|s| s.index), Some(10));
        assert_eq!(expired.len(), 11);
    }

    #[test]
    fn verbatim_prefixes_are_removed_for_ffmpeg() {
        assert_eq!(
            plain_path(Path::new(r"\\?\C:\Clips\a.mp4")),
            PathBuf::from(r"C:\Clips\a.mp4")
        );
        assert_eq!(
            plain_path(Path::new(r"\\?\UNC\nas\share\a.mp4")),
            PathBuf::from(r"\\nas\share\a.mp4")
        );
        assert_eq!(
            plain_path(Path::new(r"C:\Clips\a.mp4")),
            PathBuf::from(r"C:\Clips\a.mp4")
        );
        assert_eq!(
            ffmpeg_path(Path::new(r"\\?\C:\Clips\seg-000001.ts")),
            "C:/Clips/seg-000001.ts"
        );
    }

    #[test]
    fn byte_counts_read_like_german_sizes() {
        assert_eq!(format_bytes(0), "0 MB");
        assert_eq!(format_bytes(820 * 1024 * 1024), "820 MB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5,0 GB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 3 / 2), "1,5 GB");
    }

    #[test]
    fn clip_names_are_dated_and_unique() {
        let dir = env::temp_dir().join(format!("stillclip-names-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let first = clip_path(&dir, "Clip");
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("Clip 20"));
        fs::write(&first, b"x").unwrap();
        let second = clip_path(&dir, "Clip");
        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with(" (2).mp4"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires local FFmpeg; uses generated test video, never screen capture"]
    fn synthetic_video_can_be_trimmed_to_playable_mp4() {
        let ffmpeg = find_ffmpeg().expect("local FFmpeg is required");
        let directory =
            env::temp_dir().join(format!("stillclip-recorder-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.mp4");
        let result = command(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-n",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=15",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "4",
                "-c:v",
                "libx264",
                "-g",
                "15",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", diagnostic(&result.stderr));
        let original_bytes = fs::read(&source).unwrap();
        let info = probe_media(&ffmpeg, &source).unwrap();
        assert_eq!((info.width, info.height), (320, 180));
        assert!(info.audio);
        for (mode, name) in [
            (TrimMode::Precise, "precise.mp4"),
            (TrimMode::Fast, "fast.mp4"),
        ] {
            let trimmed = directory.join(name);
            // The cut reports where it really starts, so marked moments can move with it.
            let from = trim_clip(&ffmpeg, &source, &trimmed, 1.4, 3.0, mode).unwrap();
            match mode {
                TrimMode::Precise => assert_eq!(from, 1.4),
                // Keyframes sit one second apart here, and the copied audio
                // begins a little before that keyframe.
                TrimMode::Fast => {
                    assert!((0.7..=1.01).contains(&from), "fast cut starts at {from}")
                }
            }
            let duration = probe_duration(&ffmpeg, &trimmed).unwrap();
            assert!(
                (1.5..=2.4).contains(&duration),
                "wrong duration: {duration}"
            );
            // Marked moments of the source move into the new clip; the one
            // before the cut is left behind.
            let moved = crate::marker_offsets(&[0.5, 1.5, 2.9], from, 3.0 - from);
            let expected: Vec<f64> = [1.5 - from, 2.9 - from].into_iter().collect();
            assert_eq!(moved.len(), expected.len(), "{moved:?} for a cut at {from}");
            for (got, want) in moved.iter().zip(&expected) {
                assert!(
                    (got - want).abs() < 0.001,
                    "{moved:?} instead of {expected:?}"
                );
            }
            assert!(
                trim_clip(&ffmpeg, &source, &trimmed, 0.0, 1.0, mode).is_err(),
                "must not overwrite"
            );
        }
        assert_eq!(
            fs::read(&source).unwrap(),
            original_bytes,
            "original must stay intact"
        );
        let thumbnail = directory.join("thumbs").join("source.jpg");
        make_thumbnail(&ffmpeg, &source, &thumbnail, 1.0).unwrap();
        assert!(fs::metadata(&thumbnail).unwrap().len() > 500);
        let _ = fs::remove_dir_all(&directory);
    }

    /// Records the primary display with system audio for `STILLCLIP_SYNC_SECONDS`
    /// seconds into `STILLCLIP_SYNC_OUT` so the result can be analysed for A/V sync.
    #[test]
    #[ignore = "records the developer's own screen; run on purpose"]
    fn sync_capture() {
        let Some(out) = env::var_os("STILLCLIP_SYNC_OUT") else {
            eprintln!("STILLCLIP_SYNC_OUT not set; skipping");
            return;
        };
        let seconds: u64 = env::var("STILLCLIP_SYNC_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(12);
        let ffmpeg = find_ffmpeg().expect("local FFmpeg is required");
        let dir = PathBuf::from(out);
        fs::create_dir_all(&dir).unwrap();
        let displays = capture::displays();
        let wanted = env::var("STILLCLIP_SYNC_DISPLAY").unwrap_or_default();
        let display = displays
            .iter()
            .find(|d| !wanted.is_empty() && d.name.contains(&wanted))
            .or_else(|| capture::choose_display(&displays, ""))
            .expect("a display")
            .clone();
        eprintln!("display {display:?}");
        let capabilities = capture::probe_capabilities(&ffmpeg, &dir);
        let mut recorder = Recorder::new(ffmpeg.clone());
        let info = recorder
            .start(SessionConfig {
                window: None,
                audio_process: None,
                display,
                encoder: capture::select_encoder(
                    crate::settings::EncoderChoice::Auto,
                    &capabilities.encoders,
                ),
                capabilities,
                fps: 60,
                quality: Quality::P720,
                show_cursor: false,
                system_audio: true,
                microphone: None,
                microphone_separate_track: false,
                system_gain: 1.0,
                microphone_gain: 1.0,
                replay_seconds: 30,
                keep_full_session: true,
                min_free_bytes: 0,
                precise_clips: false,
                clips_dir: dir.clone(),
            })
            .unwrap();
        eprintln!("session {info:?}");
        thread::sleep(Duration::from_secs(seconds));
        let full = recorder.stop().unwrap().expect("full session video");
        eprintln!("SYNC_OUTPUT={}", full.display());
    }

    /// A real six-second capture of the primary display with system audio.
    /// Run explicitly on a development machine: `cargo test real_capture -- --ignored --nocapture`.
    #[test]
    #[ignore = "records the developer's own screen for a few seconds"]
    fn real_capture_smoke() {
        let ffmpeg = find_ffmpeg().expect("local FFmpeg is required");
        let dir = env::temp_dir().join(format!("stillclip-real-capture-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let displays = capture::displays();
        let display = capture::choose_display(&displays, "")
            .expect("a display")
            .clone();
        let capabilities = capture::probe_capabilities(&ffmpeg, &dir);
        eprintln!(
            "display {display:?}
capabilities {capabilities:?}"
        );
        let mut recorder = Recorder::new(ffmpeg.clone());
        let info = recorder
            .start(SessionConfig {
                window: None,
                audio_process: None,
                display,
                encoder: capture::select_encoder(
                    crate::settings::EncoderChoice::Auto,
                    &capabilities.encoders,
                ),
                capabilities,
                fps: 60,
                quality: Quality::P720,
                show_cursor: false,
                system_audio: true,
                microphone: None,
                microphone_separate_track: false,
                system_gain: 1.0,
                microphone_gain: 1.0,
                replay_seconds: 30,
                keep_full_session: true,
                min_free_bytes: 0,
                precise_clips: true,
                clips_dir: dir.clone(),
            })
            .unwrap();
        eprintln!("session {info:?}");
        assert!(info.warnings.is_empty(), "{:?}", info.warnings);
        assert_eq!(info.audio, vec!["Systemton".to_owned()]);
        thread::sleep(Duration::from_secs(7));
        assert!(recorder.poll().is_none());
        let clip = recorder.save_replay(4, "Testspiel: Runde?").unwrap();
        let clip_info = probe_media(&ffmpeg, &clip.path).unwrap();
        eprintln!("clip {clip:?} {clip_info:?}");
        assert!(clip
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("Testspiel Runde 20"));
        assert!(clip_info.audio, "replay must carry system audio");
        assert!(
            (3.9..=4.15).contains(&clip_info.duration),
            "precise replay length {}",
            clip_info.duration
        );
        assert_eq!((clip_info.width, clip_info.height), (1280, 720));
        thread::sleep(Duration::from_secs(2));
        let full = recorder.stop().unwrap().expect("full session video");
        let full_info = probe_media(&ffmpeg, &full).unwrap();
        eprintln!("full {full:?} {full_info:?}");
        assert!(full_info.audio);
        assert!(
            (8.0..=14.0).contains(&full_info.duration),
            "full length {}",
            full_info.duration
        );
        assert!(!buffer_dir(&dir).exists(), "buffer must be cleaned up");
        // Audio and video should end together: no drift after ten seconds.
        let ffprobe = ffprobe_for(&ffmpeg).expect("ffprobe");
        let output = command(&ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,duration",
                "-of",
                "csv=p=0",
            ])
            .arg(&full)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        eprintln!("streams {text}");
        let mut durations = Vec::new();
        for line in text.lines() {
            if let Some((_, duration)) = line.trim().split_once(',') {
                if let Ok(value) = duration.parse::<f64>() {
                    durations.push(value);
                }
            }
        }
        assert_eq!(durations.len(), 2, "{text}");
        assert!(
            (durations[0] - durations[1]).abs() < 0.25,
            "audio/video drift: {durations:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires local FFmpeg; uses generated test video, never screen capture"]
    fn synthetic_segments_export_a_keyframe_aligned_tail() {
        let ffmpeg = find_ffmpeg().expect("local FFmpeg is required");
        let ffprobe = ffprobe_for(&ffmpeg);
        let dir = env::temp_dir().join(format!("stillclip-segments-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let list_path = dir.join("segments.csv");
        let result = command(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "12.5",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-g",
                "12",
                "-bf",
                "0",
                "-c:a",
                "aac",
                "-f",
                "segment",
                "-segment_time",
            ])
            .arg(SEGMENT_SECONDS.to_string())
            .args([
                "-segment_format",
                "mpegts",
                "-segment_list_type",
                "csv",
                "-segment_list",
            ])
            .arg(&list_path)
            .arg(dir.join("seg-%06d.ts"))
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", diagnostic(&result.stderr));
        let segments = scan_segments(&dir, &list_path);
        assert!(segments.len() >= 3, "{segments:?}");
        let chosen = tail_segments(&segments, 6.4, 2.5);
        let output = dir.join("tail.mp4");
        export_tail(&ffmpeg, ffprobe.as_deref(), &chosen, 6.0, 0.4, &output).unwrap();
        let info = probe_media(&ffmpeg, &output).unwrap();
        assert!(info.audio);
        assert!(
            (6.0..=6.6).contains(&info.duration),
            "tail should be about six seconds, got {}",
            info.duration
        );
        let recorder = Recorder::new(ffmpeg.clone());
        let full = dir.join("full.mp4");
        recorder.export_all(&segments, &full).unwrap();
        let full_info = probe_media(&ffmpeg, &full).unwrap();
        assert!(
            (full_info.duration - 12.5).abs() < 0.3,
            "{}",
            full_info.duration
        );

        // A marked moment at 7.0 s with four seconds before and two after:
        // the window has to land in the middle of the recording, not at its end.
        let window = span_segments(&segments, 3.0, 9.0);
        assert!(!window.is_empty(), "{segments:?}");
        let marked = dir.join("marker.mp4");
        export_span(&ffmpeg, ffprobe.as_deref(), &window, 3.0, 9.0, 0.4, &marked).unwrap();
        let marked_info = probe_media(&ffmpeg, &marked).unwrap();
        assert!(marked_info.audio);
        assert!(
            (6.0..=6.6).contains(&marked_info.duration),
            "the marked window should be about six seconds, got {}",
            marked_info.duration
        );
        // The ring can disappear while an export is queued. Pinned segments
        // must still produce a complete video, including the precise path.
        let snapshot = crate::export::Snapshot::new(&dir, &chosen).unwrap();
        for segment in &segments {
            fs::remove_file(&segment.path).unwrap();
        }
        let detached = dir.join("detached.mp4");
        crate::export::ReplayExport {
            snapshot,
            output: detached.clone(),
            ffmpeg: ffmpeg.clone(),
            ffprobe,
            span: None,
            seconds: 6.0,
            gop: 0.4,
            precise: true,
            encoder: Encoder::Cpu,
            rates: crate::encoding::CaptureEncoding::for_output(320, 180, 30, 320, 180, 2),
        }
        .run()
        .unwrap();
        let detached_info = probe_media(&ffmpeg, &detached).unwrap();
        assert!(detached_info.audio);
        assert!(
            (detached_info.duration - 6.0).abs() < 0.15,
            "{}",
            detached_info.duration
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Records for a while and cuts the clip for a marked moment out of the
    /// middle of the buffer. `STILLCLIP_SYNC_DISPLAY=DISPLAY2` picks a screen.
    /// Run explicitly: `cargo test marker_capture -- --ignored --nocapture`.
    #[test]
    #[ignore = "records the developer's own screen for half a minute"]
    fn marker_capture() {
        let ffmpeg = find_ffmpeg().expect("local FFmpeg is required");
        let dir = env::temp_dir().join(format!("stillclip-marker-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let displays = capture::displays();
        let wanted = env::var("STILLCLIP_SYNC_DISPLAY").unwrap_or_default();
        let display = displays
            .iter()
            .find(|d| !wanted.is_empty() && d.name.contains(&wanted))
            .or_else(|| capture::choose_display(&displays, ""))
            .expect("a display")
            .clone();
        eprintln!("display {}", display.name);
        let capabilities = capture::probe_capabilities(&ffmpeg, &dir);
        let mut recorder = Recorder::new(ffmpeg.clone());
        recorder
            .start(SessionConfig {
                window: None,
                audio_process: None,
                display,
                encoder: capture::select_encoder(
                    crate::settings::EncoderChoice::Auto,
                    &capabilities.encoders,
                ),
                capabilities,
                fps: 60,
                quality: Quality::P720,
                show_cursor: false,
                system_audio: true,
                microphone: None,
                microphone_separate_track: false,
                system_gain: 1.0,
                microphone_gain: 1.0,
                replay_seconds: 60,
                keep_full_session: false,
                min_free_bytes: 0,
                precise_clips: false,
                clips_dir: dir.clone(),
            })
            .unwrap();
        thread::sleep(Duration::from_secs(24));
        // Too early: the window reaches past what has been written.
        assert!(recorder.save_marker(30.0, 6.0, 4.0, "Zukunft").is_err());
        let (from, to) = recorder.buffered_span().expect("buffered span");
        eprintln!("buffered {from:.1}..{to:.1}");
        assert!(from < 1.0 && to > 15.0, "{from} {to}");
        let clip = recorder.save_marker(12.0, 6.0, 4.0, "Markierung").unwrap();
        // The clip starts at the keyframe at or shortly before the wanted spot.
        assert!(
            (5.6..=6.05).contains(&clip.start),
            "clip starts at {}",
            clip.start
        );
        let info = probe_media(&ffmpeg, &clip.path).unwrap();
        eprintln!("clip {clip:?} {info:?}");
        assert!(info.audio, "a marked moment keeps the sound");
        assert_eq!((info.width, info.height), (1280, 720));
        assert!(
            (10.0..=10.6).contains(&info.duration),
            "ten seconds around the mark, got {}",
            info.duration
        );
        recorder.stop().unwrap();
        assert!(!buffer_dir(&dir).exists(), "buffer must be cleaned up");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn marked_windows_pick_the_overlapping_segments() {
        let dir = Path::new("buffer");
        let segment = |index: u64, start: f64, end: f64| Segment {
            index,
            path: dir.join(format!("seg-{index:06}.ts")),
            start: Some(start),
            end: Some(end),
        };
        let mut segments = vec![
            segment(0, 0.0, 5.0),
            segment(1, 5.0, 10.0),
            segment(2, 10.0, 15.0),
        ];
        // An unfinished segment has no times yet and never takes part.
        segments.push(Segment {
            index: 3,
            path: dir.join("seg-000003.ts"),
            start: None,
            end: None,
        });
        let indexes = |from, to| {
            span_segments(&segments, from, to)
                .iter()
                .map(|s| s.index)
                .collect::<Vec<_>>()
        };
        assert_eq!(indexes(6.0, 9.0), [1]);
        assert_eq!(indexes(3.0, 12.0), [0, 1, 2]);
        assert_eq!(indexes(0.0, 15.0), [0, 1, 2]);
        // Exactly on a boundary the neighbour stays out.
        assert_eq!(indexes(5.0, 10.0), [1]);
        assert!(indexes(20.0, 25.0).is_empty());
    }
}
