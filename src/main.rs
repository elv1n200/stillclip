#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The UI state object has grown past serde_json's default macro depth.
#![recursion_limit = "256"]

mod audio;
mod audio_test;
mod capture;
mod diagnostics;
mod discord;
mod drag;
mod encoding;
mod export;
mod games;
mod highlights;
mod icon;
mod jobs;
mod library;
mod lifecycle;
mod mechanics;
mod media;
mod overlay;
mod platform;
mod portrait;
mod process_tree;
mod recorder;
mod screenshot;
mod settings;

use audio::AudioDevice;
use capture::{Capabilities, Display, Encoder};
use games::{GameEntry, GameWatcher, RunningApp};
use library::{Clip, Index, IndexEntry};
use overlay::Overlay;
use platform::{Hotkeys, Tray, TrayAction};
use recorder::{Recorder, SessionConfig, SessionInfo, TrimMode};
use serde_json::{json, Value};
use settings::Settings;
use std::{
    borrow::Cow,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, Instant, UNIX_EPOCH},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::windows::WindowExtWindows,
    window::{Icon, WindowBuilder},
};
use wry::{http::Response, WebContext, WebView, WebViewBuilder};

const HOTKEY_REPLAY: i32 = 1;
const HOTKEY_RECORD: i32 = 2;
const HOTKEY_REPLAY_LONG: i32 = 3;
const HOTKEY_MARKER: i32 = 4;
/// How often a capture that died on its own is restarted, and within which span.
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(300);
const FPS_CHOICES: [u32; 5] = [24, 30, 60, 120, 144];

pub enum AppEvent {
    Action(Value),
    Finished(JobResult),
    ExportFinished(u64, Result<JobOutcome, String>),
    ClipAnnotation(PathBuf, library::Annotation),
    AudioMeters(u64, audio_test::Meters),
    AudioFinished(u64, Result<audio_test::Preview, String>),
    DragFinished(Result<(), String>),
    Notice(String),
    Hotkey(i32),
    HotkeysApplied(Vec<i32>),
    Tray(TrayAction),
    Capabilities(Capabilities),
    Indexed(String, IndexEntry),
    IndexingDone,
    /// A watched game appeared, together with the display its window sits on.
    GameStarted(GameEntry, String),
    GameStopped(GameEntry),
    /// A saved clip and the marked moments inside it, in seconds from its start.
    ClipMarkers(PathBuf, Vec<f64>),
    ScreenshotFinished(Result<(), String>),
}

pub struct JobOutcome {
    message: String,
    saved: Option<PathBuf>,
    session: Option<SessionInfo>,
    /// A replay clip: show the in-game banner and play the chime.
    replay: bool,
}

pub struct JobResult {
    recorder: Option<Recorder>,
    restore_recorder: bool,
    result: Result<JobOutcome, String>,
}

type MediaFiles = Arc<RwLock<HashMap<String, PathBuf>>>;

/// A moment the user marked while recording. Nothing is written yet: the
/// marker only remembers where in the running session something happened.
#[derive(Clone, Debug)]
struct Marker {
    id: u64,
    /// Seconds since this recording started.
    at: f64,
    /// Name the clip gets, usually the detected game.
    prefix: String,
}

#[derive(Default)]
struct Notice {
    sequence: u64,
    text: String,
    error: bool,
    clip: Option<PathBuf>,
}

struct App {
    config: PathBuf,
    settings: Settings,
    session_settings: Option<Settings>,
    ffmpeg: Option<PathBuf>,
    recorder: Option<Recorder>,
    clips: Vec<(u64, Clip)>,
    media: MediaFiles,
    next_id: u64,
    index: Index,
    indexing: bool,
    busy: bool,
    exports: jobs::Queue,
    audio_test: audio_test::Test,
    dragging: Option<(PathBuf, drag::Control)>,
    recording: bool,
    elapsed: u64,
    status: String,
    error: bool,
    closing: bool,
    capabilities: Capabilities,
    displays: Vec<Display>,
    microphones: Vec<AudioDevice>,
    default_output: Option<String>,
    session: Option<SessionInfo>,
    buffer_bytes: u64,
    hotkeys: Option<Hotkeys>,
    hotkey_failures: Vec<i32>,
    tray: Option<Tray>,
    window_visible: bool,
    tray_hint_shown: bool,
    autostart: bool,
    notice: Notice,
    capture: lifecycle::CaptureControl,
    recovered: Option<PathBuf>,
    watcher: Option<GameWatcher>,
    overlay: Option<Overlay>,
    active_game: Option<GameEntry>,
    /// Display the detected game runs on, used when the setting follows it.
    active_game_display: Option<String>,
    auto_started: bool,
    apps: Vec<RunningApp>,
    /// When captures died on their own, so endless restarts stop.
    restarts: Vec<Instant>,
    free_bytes: u64,
    clips_bytes: u64,
    markers: Vec<Marker>,
    next_marker: u64,
    /// Marker whose clip is currently being written; removed once it worked.
    pending_marker: Option<u64>,
    /// Index key of a clip being renamed, so its markers follow the new name.
    pending_rename: Option<String>,
    /// Seconds of finished video in the buffer, as `(from, to)`.
    buffered: Option<(f64, f64)>,
}

impl App {
    fn new() -> Self {
        let config = std::env::var_os("STILLCLIP_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(settings::default_config_dir);
        let mut settings = settings::load_settings(&config);
        if std::env::var_os("STILLCLIP_DATA_DIR").is_some()
            && !config.join("settings.json").exists()
        {
            settings.clips_dir = config.join("clips");
        }
        let ffmpeg = fs::read_to_string(config.join("ffmpeg-path.txt"))
            .ok()
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| p.is_file())
            .or_else(recorder::find_ffmpeg);
        let recorder = ffmpeg.clone().map(Recorder::new);
        let capabilities = ffmpeg
            .as_deref()
            .and_then(|f| capture::cached_capabilities(f, &config))
            .unwrap_or_default();
        let recovered = ffmpeg.as_deref().and_then(|f| {
            recorder::recover_leftovers(f, &settings.clips_dir)
                .ok()
                .flatten()
        });
        let free_bytes = platform::free_bytes(&settings.clips_dir).unwrap_or(0);
        let mut app = Self {
            index: Index::load(&config),
            config,
            settings,
            session_settings: None,
            ffmpeg,
            recorder,
            clips: Vec::new(),
            media: Arc::new(RwLock::new(HashMap::new())),
            next_id: 1,
            indexing: false,
            busy: false,
            exports: jobs::Queue::default(),
            audio_test: audio_test::Test::default(),
            dragging: None,
            recording: false,
            elapsed: 0,
            status: String::new(),
            error: false,
            closing: false,
            capabilities,
            displays: capture::displays(),
            microphones: audio::capture_devices(),
            default_output: audio::default_output_name(),
            session: None,
            buffer_bytes: 0,
            hotkeys: None,
            hotkey_failures: Vec::new(),
            tray: None,
            window_visible: true,
            tray_hint_shown: false,
            autostart: platform::autostart_enabled(),
            notice: Notice::default(),
            capture: lifecycle::CaptureControl::default(),
            recovered,
            watcher: None,
            overlay: None,
            active_game: None,
            active_game_display: None,
            auto_started: false,
            apps: games::candidate_apps(),
            restarts: Vec::new(),
            free_bytes,
            clips_bytes: 0,
            markers: Vec::new(),
            next_marker: 1,
            pending_marker: None,
            pending_rename: None,
            buffered: None,
        };
        app.refresh();
        app
    }

    /// The display a recording should use: the game's own screen when that is
    /// switched on, otherwise the configured one.
    fn capture_display(&self) -> Option<&Display> {
        let wanted = self
            .active_game_display
            .as_deref()
            .filter(|name| {
                self.settings.follow_game_display
                    && self.displays.iter().any(|display| display.name == *name)
            })
            .unwrap_or(self.settings.display.as_str());
        capture::choose_display(&self.displays, wanted)
    }

    fn runtime_settings(&self) -> &Settings {
        if self.recording || self.capture.transitioning {
            self.session_settings.as_ref().unwrap_or(&self.settings)
        } else {
            &self.settings
        }
    }

    fn thumbnail_dir(&self) -> PathBuf {
        self.config.join("thumbs")
    }

    fn message(&mut self, result: Result<String, String>) {
        self.error = result.is_err();
        self.status = result.unwrap_or_else(|error| error);
    }

    /// Banner in the top-right corner of the recorded display.
    fn banner(&self, text: &str, error: bool) {
        if !self.settings.overlay_notifications {
            return;
        }
        let Some(overlay) = &self.overlay else {
            return;
        };
        let display = self.capture_display();
        let (x, y, limit) = match display {
            Some(d) => (
                d.x + d.width as i32 - 28,
                d.y + 28,
                (d.width as i32 / 2).max(320),
            ),
            None => (1880, 28, 600),
        };
        overlay.show(text, error, x, y, limit);
    }

    fn clip_prefix(&self) -> String {
        self.active_game
            .as_ref()
            .map(|g| g.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Clip".into())
    }

    fn apply_games(&mut self) {
        if let Some(watcher) = &self.watcher {
            watcher.set_games(self.settings.watched_games());
        }
    }

    /// A transient toast in the UI (and a balloon while the window is hidden).
    fn notify(&mut self, text: &str, error: bool, clip: Option<PathBuf>) {
        self.notice = Notice {
            sequence: self.notice.sequence + 1,
            text: text.to_owned(),
            error,
            clip,
        };
        if !self.window_visible {
            if let Some(tray) = &self.tray {
                tray.notify(
                    if error {
                        "Stillclip – Hinweis"
                    } else {
                        "Stillclip"
                    },
                    text,
                );
            }
        }
    }

    /// Deletes the oldest clips when the folder outgrew its limit. Only runs
    /// when the user set one; the newest clip always survives.
    fn enforce_storage_limit(&mut self, clips: &mut Vec<Clip>) {
        let (count, freed, failed) = library::clean_limit_except(
            clips,
            self.settings.storage_limit_bytes(),
            &self.index.favorites,
            |path| fs::remove_file(path),
        );
        if count == 0 {
            if failed > 0 {
                self.notify(
                    "Einige Clips konnten nicht gelöscht werden. Sie bleiben in der Bibliothek.",
                    true,
                    None,
                );
            }
            return;
        }
        let text = if count == 1 {
            format!(
                "Der älteste Clip wurde gelöscht, damit der Ordner unter {} GB bleibt ({} frei).",
                self.settings.storage_limit_gb,
                recorder::format_bytes(freed)
            )
        } else {
            format!(
                "{count} alte Clips wurden gelöscht, damit der Ordner unter {} GB bleibt ({} frei).",
                self.settings.storage_limit_gb,
                recorder::format_bytes(freed)
            )
        };
        let text = if failed > 0 {
            format!("{text} {failed} Clips konnten nicht gelöscht werden und bleiben erhalten.")
        } else {
            text
        };
        self.notify(&text, failed > 0, None);
    }

    fn refresh(&mut self) {
        match library::scan_clips(&self.settings.clips_dir) {
            Ok(mut clips) => {
                if !self.busy
                    && !self.capture.transitioning
                    && self.exports.is_empty()
                    && self.dragging.is_none()
                {
                    self.enforce_storage_limit(&mut clips);
                }
                self.clips_bytes = clips.iter().map(|clip| clip.bytes).sum();
                let previous: HashMap<_, _> = self
                    .clips
                    .iter()
                    .map(|(id, c)| (c.path.clone(), *id))
                    .collect();
                self.clips = clips
                    .into_iter()
                    .map(|clip| {
                        let id = match previous.get(&clip.path) {
                            Some(id) => *id,
                            None => {
                                let id = self.next_id;
                                self.next_id += 1;
                                id
                            }
                        };
                        (id, clip)
                    })
                    .collect();
                let thumbs = self.thumbnail_dir();
                let plain: Vec<Clip> = self.clips.iter().map(|(_, c)| c.clone()).collect();
                // Keep source metadata until an in-flight rename has moved it.
                if !self.busy {
                    self.index.prune(&plain, &thumbs);
                }
                self.update_media_map();
                if !self.busy && !self.capture.transitioning {
                    self.schedule_indexing();
                }
            }
            Err(error) => self.message(Err(error)),
        }
    }

    fn update_media_map(&self) {
        if let Ok(mut files) = self.media.write() {
            files.clear();
            if let Some(preview) = &self.audio_test.preview {
                files.insert(
                    format!("audio-test/{}", self.audio_test.id),
                    preview.path.clone(),
                );
            }
            let thumbs = self.thumbnail_dir();
            for (id, clip) in &self.clips {
                files.insert(id.to_string(), clip.path.clone());
                if let Some(name) = self
                    .index
                    .current(clip)
                    .and_then(|entry| entry.thumbnail.as_ref())
                {
                    files.insert(format!("thumb/{id}"), thumbs.join(name));
                }
            }
        }
    }

    fn proxy_slot() -> &'static RwLock<Option<EventLoopProxy<AppEvent>>> {
        static SLOT: RwLock<Option<EventLoopProxy<AppEvent>>> = RwLock::new(None);
        &SLOT
    }

    /// Probes durations and poster frames for clips the index does not know yet.
    fn schedule_indexing(&mut self) {
        if self.indexing {
            return;
        }
        let Some(ffmpeg) = self.ffmpeg.clone() else {
            return;
        };
        let Some(proxy) = Self::proxy_slot().read().ok().and_then(|p| p.clone()) else {
            return;
        };
        let pending: Vec<(String, PathBuf, u64, u64, String)> = self
            .clips
            .iter()
            .filter(|(_, clip)| self.index.needs_work(clip))
            .map(|(_, clip)| {
                (
                    library::index_key(&clip.path),
                    clip.path.clone(),
                    clip.bytes,
                    library::modified_seconds(clip),
                    library::thumbnail_name(clip),
                )
            })
            .collect();
        if pending.is_empty() {
            return;
        }
        self.indexing = true;
        let thumbs = self.thumbnail_dir();
        std::thread::spawn(move || {
            for (key, path, bytes, modified, thumbnail) in pending {
                let entry = match recorder::probe_media(&ffmpeg, &path) {
                    Ok(info) => {
                        let at = (info.duration * 0.15)
                            .clamp(0.0, 3.0)
                            .min((info.duration - 0.1).max(0.0));
                        let thumbnail_path = thumbs.join(&thumbnail);
                        let has_thumbnail = thumbnail_path.is_file()
                            || recorder::make_thumbnail(&ffmpeg, &path, &thumbnail_path, at)
                                .is_ok();
                        IndexEntry {
                            bytes,
                            modified,
                            info,
                            thumbnail: has_thumbnail.then_some(thumbnail),
                            failed: false,
                            // Filled in by the event loop, which knows them.
                            markers: Vec::new(),
                        }
                    }
                    Err(_) => IndexEntry {
                        bytes,
                        modified,
                        info: Default::default(),
                        thumbnail: None,
                        failed: true,
                        markers: Vec::new(),
                    },
                };
                if proxy.send_event(AppEvent::Indexed(key, entry)).is_err() {
                    return;
                }
            }
            let _ = proxy.send_event(AppEvent::IndexingDone);
        });
    }

    fn annotation(&self, path: &Path) -> library::Annotation {
        self.index
            .annotations
            .get(&library::index_key(path))
            .cloned()
            .unwrap_or_else(|| {
                let name = path.file_stem().unwrap_or_default().to_string_lossy();
                static BUILTIN: std::sync::OnceLock<Vec<GameEntry>> = std::sync::OnceLock::new();
                let game = BUILTIN
                    .get_or_init(games::builtin_games)
                    .iter()
                    .chain(&self.settings.custom_games)
                    .filter(|g| {
                        name.strip_prefix(g.name.as_str())
                            .is_some_and(|tail| tail.starts_with([' ', '_', '-']))
                    })
                    .max_by_key(|g| g.name.len())
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                library::Annotation {
                    game,
                    tags: Vec::new(),
                }
            })
    }

    fn clip_locked(&self, path: &Path) -> bool {
        self.exports.uses(path) || self.dragging.as_ref().is_some_and(|(p, _)| p == path)
    }

    fn clip_json(&self, id: u64, clip: &Clip) -> Value {
        let entry = self.index.current(clip);
        let annotation = self.annotation(&clip.path);
        json!({
            "id": id,
            "name": clip.name,
            "bytes": clip.bytes,
            "extension": clip.path.extension().unwrap_or_default().to_string_lossy(),
            "created": clip.created.and_then(|v| v.duration_since(UNIX_EPOCH).ok()).map(|v| v.as_secs()),
            "src": format!("http://clip.localhost/{id}"),
            "thumb": entry.and_then(|e| e.thumbnail.as_ref()).map(|_| format!("http://clip.localhost/thumb/{id}")),
            "duration": entry.map(|e| e.info.duration),
            "width": entry.map(|e| e.info.width),
            "height": entry.map(|e| e.info.height),
            "audio": entry.map(|e| e.info.audio),
            // Marked moments are kept by path, so they show before the clip is indexed.
            "markers": self.markers_of(&clip.path),
            "favorite": self.index.favorites.contains(&library::index_key(&clip.path)),
            "locked": self.clip_locked(&clip.path),
            "game": annotation.game,
            "tags": annotation.tags,
            "pending": entry.is_none(),
            "path": clip.path,
        })
    }

    fn sync(&self, view: &WebView) {
        let clips: Vec<Value> = self
            .clips
            .iter()
            .map(|(id, clip)| self.clip_json(*id, clip))
            .collect();
        let displays: Vec<Value> = self
            .displays
            .iter()
            .map(|d| {
                json!({"name": d.name, "label": d.label(), "primary": d.primary, "width": d.width, "height": d.height})
            })
            .collect();
        let encoders: Vec<Value> = Encoder::ALL
            .iter()
            .map(|e| {
                json!({
                    "id": e.id(),
                    "label": e.label(),
                    "available": self.capabilities.encoders.contains(e) || !self.capabilities.probed,
                })
            })
            .collect();
        let selected_encoder =
            capture::select_encoder(self.settings.encoder, &self.capabilities.encoders);
        let state = json!({
            "clips": clips,
            "recording": self.recording,
            "elapsed": self.elapsed,
            "busy": self.busy || self.capture.transitioning,
            "capture_busy": self.capture.transitioning,
            "exports": self.exports.views(),
            "audio_test": self.audio_test.state(),
            "dragging": self.dragging.is_some(),
            "export_limit": jobs::LIMIT,
            "status": self.status,
            "error": self.error,
            "ffmpeg_available": self.ffmpeg.is_some(),
            "ffmpeg_path": self.ffmpeg,
            "clips_dir": self.settings.clips_dir,
            "settings": self.settings,
            "runtime_settings": {
                "quality": self.runtime_settings().quality,
                "fps": self.runtime_settings().fps,
                "replay_seconds": self.runtime_settings().replay_seconds,
                "microphone": self.runtime_settings().microphone,
                "system_audio": self.runtime_settings().system_audio,
                "capture_window": self.runtime_settings().capture_window,
                "game_audio": self.runtime_settings().game_audio,
            },
            "autostart": self.autostart,
            "active_game": self.active_game,
            "active_game_display": self.active_game_display,
            "capture_display": self.capture_display().map(|d| d.name.clone()),
            "auto_started": self.auto_started,
            "apps": self.apps,
            "builtin_games": games::builtin_games(),
            "displays": displays,
            "microphones": self.microphones,
            "default_output": self.default_output,
            "capabilities": {
                "probed": self.capabilities.probed,
                "ddagrab": self.capabilities.ddagrab,
                "gfxcapture": self.capabilities.gfxcapture,
                "encoders": self.capabilities.encoders.iter().map(|e| e.id()).collect::<Vec<_>>(),
            },
            "encoders": encoders,
            "selected_encoder": selected_encoder.label(),
            "session": self.session,
            "diagnostics": self.recorder.as_ref().and_then(|r| r.diagnostics()),
            "markers": self.marker_json(),
            "buffer_bytes": self.buffer_bytes,
            "free_bytes": self.free_bytes,
            "clips_bytes": self.clips_bytes,
            "max_volume": settings::MAX_VOLUME,
            "notice": {
                "sequence": self.notice.sequence,
                "text": self.notice.text,
                "error": self.notice.error,
                "clip": self.notice.clip.as_ref().and_then(|p| self.clips.iter().find(|(_, c)| &c.path == p)).map(|(id, _)| id),
            },
            "replay_presets": mechanics::REPLAY_PRESETS,
            "replay_min": mechanics::MIN_REPLAY_SECONDS,
            "replay_max": mechanics::MAX_REPLAY_SECONDS,
            "editor_min_seconds": mechanics::MIN_EDITOR_SECONDS,
            "editor_tolerance": mechanics::EDITOR_TOLERANCE_SECONDS,
            "fps_choices": FPS_CHOICES,
            "hotkeys": {
                "replay": self.settings.hotkey_replay.label(),
                "replay_long": self.settings.hotkey_replay_long.label(),
                "marker": self.settings.hotkey_marker.label(),
                "record": self.settings.hotkey_record.label(),
                "failed": self.hotkey_failures,
            },
            "tray": self.tray.is_some(),
            "version": env!("CARGO_PKG_VERSION"),
            "recovered": self.recovered,
        });
        let _ = view.evaluate_script(&format!("window.setState && window.setState({state});"));
    }

    /// Markers with what the buffer can still deliver for each of them:
    /// `ready` once the whole window is written, `seconds` how much of the
    /// wanted window is left before the ring deletes it.
    fn marker_json(&self) -> Vec<Value> {
        let before = f64::from(self.settings.marker_before);
        let after = f64::from(self.settings.marker_after);
        self.markers
            .iter()
            .map(|marker| {
                let (ready, available) = marker_state(marker.at, before, after, self.buffered);
                json!({
                    "id": marker.id,
                    "at": marker.at.round() as u64,
                    "ready": ready,
                    "seconds": available.round() as u64,
                    "wanted": (before + after).round() as u64,
                })
            })
            .collect()
    }

    fn clip_path(&self, command: &Value) -> Result<PathBuf, String> {
        let id = command["id"]
            .as_u64()
            .ok_or("Bitte einen Clip auswählen.")?;
        self.clips
            .iter()
            .find(|(key, _)| *key == id)
            .map(|(_, c)| c.path.clone())
            .ok_or_else(|| "Der Clip ist nicht mehr in der Bibliothek. Bitte aktualisieren.".into())
    }

    fn session_config(&mut self) -> Result<SessionConfig, String> {
        let exe = self
            .active_game
            .as_ref()
            .map(|g| g.exe.as_str())
            .unwrap_or(&self.settings.target_exe);
        let settings = self.settings.for_game(exe);
        let mut display = self
            .capture_display()
            .cloned()
            .ok_or("Es wurde kein Bildschirm gefunden.")?;
        let target = if settings.capture_window || (settings.system_audio && settings.game_audio) {
            Some(games::visible_apps().into_iter().find(|app| self.active_game.as_ref().map_or(app.exe == settings.target_exe, |game| game.matches(app)))
                .ok_or("Starte das Spiel oder wähle unter Aufnahme ein laufendes Zielprogramm aus.")?)
        } else {
            None
        };
        if settings.capture_window {
            let window = target.as_ref().ok_or("Kein Aufnahmefenster gewählt.")?;
            let (w, h) = games::window_size(window.hwnd, window.pid)
                .ok_or("Das Aufnahmefenster ist geschlossen oder minimiert.")?;
            display.width = w;
            display.height = h;
        }
        let config = SessionConfig {
            window: target.clone().filter(|_| settings.capture_window),
            audio_process: target
                .as_ref()
                .filter(|_| settings.system_audio && settings.game_audio)
                .map(|app| app.pid),
            display,
            encoder: capture::select_encoder(self.settings.encoder, &self.capabilities.encoders),
            capabilities: self.capabilities.clone(),
            fps: settings.fps,
            quality: settings.quality,
            show_cursor: self.settings.show_cursor,
            system_audio: settings.system_audio,
            microphone: settings
                .microphone
                .then(|| self.settings.microphone_device.clone()),
            microphone_separate_track: self.settings.microphone_separate_track,
            system_gain: settings::gain(self.settings.system_volume),
            microphone_gain: settings::gain(self.settings.microphone_volume),
            replay_seconds: settings.buffer_seconds(),
            keep_full_session: self.settings.keep_full_session,
            min_free_bytes: self.settings.min_free_bytes(),
            precise_clips: self.settings.precise_clips,
            clips_dir: self.settings.clips_dir.clone(),
        };
        self.session_settings = Some(settings);
        Ok(config)
    }

    fn recording_job(&mut self, action: &str, proxy: EventLoopProxy<AppEvent>) {
        if action == "start" && self.audio_test.active.is_some() {
            self.message(Err("Beende zuerst den Audio-Test.".into()));
            return;
        }
        if matches!(action, "start" | "stop") {
            self.capture.request(action == "start");
            self.advance_capture(proxy);
            return;
        }
        if self.busy || self.capture.transitioning {
            return;
        }
        let seconds = if action == "replay_long" && self.settings.replay_seconds_long > 0 {
            self.settings.replay_seconds_long
        } else {
            self.runtime_settings().replay_seconds
        };
        let prefix = self.clip_prefix();
        let export = self
            .recorder
            .as_mut()
            .ok_or_else(|| "Bitte zuerst FFmpeg auswählen.".to_string())
            .and_then(|recorder| recorder.prepare_replay(seconds, &prefix));
        let export = match export {
            Ok(export) => export,
            Err(error) => {
                self.message(Err(error));
                return;
            }
        };
        let marks: Vec<f64> = self.markers.iter().map(|marker| marker.at).collect();
        let events = proxy.clone();
        self.file_job(proxy, "Moment wird gesichert…", move || {
            export.run().map(|saved| {
                send_markers(
                    &events,
                    &saved.path,
                    &marks,
                    saved.start,
                    f64::from(seconds),
                );
                JobOutcome {
                    message: "Moment gesichert.".into(),
                    saved: Some(saved.path),
                    session: None,
                    replay: true,
                }
            })
        });
    }

    /// Capture transitions are independent of file jobs. A stop requested while
    /// starting is retained; probing also cannot resurrect a cancelled start.
    fn advance_capture(&mut self, proxy: EventLoopProxy<AppEvent>) {
        if self.capture.wants_start() && !self.capabilities.probed && self.ffmpeg.is_some() {
            self.status = "FFmpeg wird geprüft, die Aufnahme startet gleich…".into();
            return;
        }
        let Some(start) = self.capture.next(self.recording) else {
            return;
        };
        let config = if start {
            match self.session_config() {
                Ok(config) => Some(config),
                Err(error) => {
                    self.message(Err(error));
                    return;
                }
            }
        } else {
            None
        };
        let Some(mut recorder) = self.recorder.take() else {
            self.message(Err(
                "Bitte zuerst FFmpeg in den Einstellungen auswählen.".into()
            ));
            return;
        };
        if start {
            self.markers.clear();
            self.buffered = None;
        }
        self.capture.transitioning = true;
        self.error = false;
        self.status = if start {
            "Aufnahme wird gestartet…"
        } else {
            "Aufnahme wird beendet…"
        }
        .into();
        let marks: Vec<f64> = self.markers.iter().map(|marker| marker.at).collect();
        let elapsed = self.elapsed as f64;
        std::thread::spawn(move || {
            let result = if let Some(config) = config {
                recorder.start(config).map(|info| JobOutcome {
                    message: format!("Aufnahme läuft über {}.", info.strategy),
                    saved: None,
                    session: Some(info),
                    replay: false,
                })
            } else {
                recorder.stop().map(|saved| {
                    if let Some(path) = &saved {
                        send_markers(&proxy, path, &marks, 0.0, elapsed + 5.0);
                    }
                    JobOutcome {
                        message: if saved.is_some() {
                            "Aufnahme beendet und gespeichert."
                        } else {
                            "Aufnahme beendet."
                        }
                        .into(),
                        saved,
                        session: None,
                        replay: false,
                    }
                })
            };
            let _ = proxy.send_event(AppEvent::Finished(JobResult {
                recorder: Some(recorder),
                restore_recorder: true,
                result,
            }));
        });
    }

    /// Remembers the current moment so its clip can be saved later. Costs
    /// nothing while playing: no file is written and the recording runs on.
    fn mark(&mut self) {
        let Some(recorder) = self.recorder.as_ref() else {
            self.message(Err(
                "Bitte zuerst FFmpeg in den Einstellungen auswählen.".into()
            ));
            return;
        };
        if !recorder.is_recording() {
            self.notify(
                "Es läuft keine Aufnahme. Starte zuerst die Aufnahme.",
                true,
                None,
            );
            return;
        }
        let at = recorder.elapsed().as_secs_f64();
        let id = self.next_marker;
        self.next_marker += 1;
        self.markers.push(Marker {
            id,
            at,
            prefix: self.clip_prefix(),
        });
        let after = self.settings.marker_after;
        let text = if after > 0 {
            format!("Markierung gesetzt. In {after} Sekunden ist der Moment vollständig.")
        } else {
            "Markierung gesetzt.".into()
        };
        self.banner("Markierung gesetzt", false);
        self.notify(&text, false, None);
        self.status = text;
        self.error = false;
    }

    /// The marked moments stored for a clip, in seconds from its start.
    fn markers_of(&self, path: &Path) -> Vec<f64> {
        self.index
            .entries
            .get(&library::index_key(path))
            .map(|entry| entry.markers.clone())
            .unwrap_or_default()
    }

    /// Keeps the marked moments of a finished clip with that clip, so they are
    /// still there after a restart.
    fn remember_markers(&mut self, path: &Path, offsets: Vec<f64>) {
        if offsets.is_empty() {
            return;
        }
        let entry = self
            .index
            .entries
            .entry(library::index_key(path))
            .or_default();
        entry.markers = offsets;
        self.index.save(&self.config);
    }

    /// Writes the clip around a marked moment.
    fn marker_job(&mut self, id: u64, proxy: EventLoopProxy<AppEvent>) {
        let Some(marker) = self.markers.iter().find(|m| m.id == id).cloned() else {
            self.message(Err("Die Markierung gibt es nicht mehr.".into()));
            return;
        };
        let before = f64::from(self.settings.marker_before);
        let after = f64::from(self.settings.marker_after);
        let export = self
            .recorder
            .as_mut()
            .ok_or_else(|| "Bitte zuerst FFmpeg auswählen.".to_string())
            .and_then(|recorder| recorder.prepare_marker(marker.at, before, after, &marker.prefix));
        let export = match export {
            Ok(export) => export,
            Err(error) => {
                self.message(Err(error));
                return;
            }
        };
        self.pending_marker = Some(id);
        let marks: Vec<f64> = self.markers.iter().map(|marker| marker.at).collect();
        let events = proxy.clone();
        self.file_job(proxy, "Markierter Moment wird gesichert…", move || {
            export.run().map(|saved| {
                send_markers(&events, &saved.path, &marks, saved.start, before + after);
                JobOutcome {
                    message: "Markierter Moment gesichert.".into(),
                    saved: Some(saved.path),
                    session: None,
                    replay: true,
                }
            })
        });
    }

    /// Reacts to something the recorder reported by itself: a capture that died
    /// is started again a few times, a nearly full drive ends the recording.
    fn handle_issue(&mut self, issue: recorder::PollIssue, proxy: EventLoopProxy<AppEvent>) {
        self.message(Err(issue.message.clone()));
        self.notify(&issue.message, true, None);
        if issue.stop_requested {
            self.banner("Aufnahme wird beendet", true);
            if self.recording {
                self.auto_started = false;
                self.recording_job("stop", proxy);
            }
            return;
        }
        self.refresh();
        if !issue.capture_lost || self.closing || self.ffmpeg.is_none() {
            return;
        }
        let now = Instant::now();
        self.restarts
            .retain(|at| now.duration_since(*at) < RESTART_WINDOW);
        if self.restarts.len() >= MAX_RESTARTS {
            self.notify(
                "Die Aufnahme ist mehrfach abgebrochen und wird nicht weiter neu gestartet.",
                true,
                None,
            );
            return;
        }
        self.restarts.push(now);
        self.notify("Die Aufnahme wird automatisch neu gestartet.", false, None);
        self.recording_job("start", proxy);
    }

    fn file_job(
        &mut self,
        proxy: EventLoopProxy<AppEvent>,
        status: &str,
        task: impl FnOnce() -> Result<JobOutcome, String> + Send + 'static,
    ) {
        self.busy = true;
        self.error = false;
        self.status = status.into();
        std::thread::spawn(move || {
            let _ = proxy.send_event(AppEvent::Finished(JobResult {
                recorder: None,
                restore_recorder: false,
                result: task(),
            }));
        });
    }

    fn cancel_background(&mut self) {
        self.exports.cancel_all();
        self.audio_test.cancel();
        if let Some((_, flag)) = &self.dragging {
            flag.cancel();
        }
    }

    fn start_audio_test(
        &mut self,
        command: &Value,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Result<(), String> {
        if self.audio_test.active.is_some() || self.recording || self.capture.transitioning {
            return Err("Beende zuerst die Aufnahme oder den laufenden Audio-Test.".into());
        }
        let ffmpeg = self
            .ffmpeg
            .clone()
            .ok_or("Bitte zuerst FFmpeg auswählen.")?;
        let mut prefs: Settings = serde_json::from_value(command["settings"].clone())
            .map_err(|e| format!("Ungültige Audio-Einstellungen: {e}"))?;
        prefs = prefs.sanitized();
        let exe = self
            .active_game
            .as_ref()
            .map(|g| g.exe.as_str())
            .unwrap_or(&prefs.target_exe);
        let prefs = prefs.for_game(exe);
        let mut sources = Vec::new();
        if prefs.system_audio {
            let source = if prefs.game_audio {
                let target = games::visible_apps()
                    .into_iter()
                    .find(|app| {
                        self.active_game
                            .as_ref()
                            .map_or(app.exe == prefs.target_exe, |game| game.matches(app))
                    })
                    .ok_or("Starte das Zielspiel für den Spielton-Test.")?;
                audio::AudioSource::Process { pid: target.pid }
            } else {
                audio::AudioSource::Loopback
            };
            sources.push((source, settings::gain(prefs.system_volume)));
        }
        if prefs.microphone {
            sources.push((
                audio::AudioSource::Microphone {
                    device_id: prefs.microphone_device,
                },
                settings::gain(prefs.microphone_volume),
            ));
        }
        if sources.is_empty() {
            return Err("Aktiviere Systemton oder Mikrofon für den Audio-Test.".into());
        }
        self.audio_test.id += 1;
        let id = self.audio_test.id;
        self.audio_test.preview = None;
        self.audio_test.error = None;
        self.audio_test.levels.clear();
        self.audio_test.meters.clear();
        self.update_media_map();
        let control = jobs::Control::new();
        self.audio_test.active = Some(control.clone());
        let root = self.config.join("previews");
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                audio_test::record(&ffmpeg, &root, sources, control, |meters| {
                    let _ = proxy.send_event(AppEvent::AudioMeters(id, meters));
                })
            }))
            .unwrap_or_else(|_| Err("Der Audio-Test wurde unerwartet beendet.".into()));
            let _ = proxy.send_event(AppEvent::AudioFinished(id, result));
        });
        Ok(())
    }

    fn queue_export(
        &mut self,
        label: String,
        sources: Vec<PathBuf>,
        task: jobs::Work,
        proxy: EventLoopProxy<AppEvent>,
    ) {
        match self.exports.push(label, sources, task) {
            Ok(()) => {
                self.message(Ok("Export zur Warteschlange hinzugefügt.".into()));
                self.start_exports(proxy);
            }
            Err(error) => self.message(Err(error)),
        }
    }

    fn start_exports(&mut self, proxy: EventLoopProxy<AppEvent>) {
        if self.busy || self.closing {
            return;
        }
        let Some((job, control)) = self.exports.start_next() else {
            return;
        };
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (job.work)(control.clone())
            }))
            .unwrap_or_else(|_| Err("Der Export wurde unerwartet beendet.".into()));
            if result.is_ok() {
                control.progress(1.0, "Gespeichert");
            }
            let _ = proxy.send_event(AppEvent::ExportFinished(job.id, result));
        });
    }

    fn finish_export(
        &mut self,
        id: u64,
        result: Result<JobOutcome, String>,
        proxy: EventLoopProxy<AppEvent>,
    ) {
        if !self.exports.finish(id) {
            return;
        }
        match result {
            Ok(outcome) => {
                self.message(Ok(outcome.message.clone()));
                self.notify(&outcome.message, false, outcome.saved);
            }
            Err(error) => {
                let cancelled = error == jobs::CANCELLED;
                self.message(if cancelled {
                    Ok(error.clone())
                } else {
                    Err(error.clone())
                });
                self.notify(&error, !cancelled, None);
            }
        }
        self.start_exports(proxy);
        self.refresh();
    }

    fn apply_hotkeys(&mut self) {
        let mut bindings = Vec::new();
        if let Some(key) = self.settings.hotkey_replay.virtual_key() {
            bindings.push((HOTKEY_REPLAY, self.settings.hotkey_replay.modifiers(), key));
        }
        if self.settings.replay_seconds_long > 0 {
            if let Some(key) = self.settings.hotkey_replay_long.virtual_key() {
                bindings.push((
                    HOTKEY_REPLAY_LONG,
                    self.settings.hotkey_replay_long.modifiers(),
                    key,
                ));
            }
        }
        if let Some(key) = self.settings.hotkey_marker.virtual_key() {
            bindings.push((HOTKEY_MARKER, self.settings.hotkey_marker.modifiers(), key));
        }
        if let Some(key) = self.settings.hotkey_record.virtual_key() {
            bindings.push((HOTKEY_RECORD, self.settings.hotkey_record.modifiers(), key));
        }
        if let Some(hotkeys) = &self.hotkeys {
            hotkeys.apply(bindings);
        }
    }

    fn save_settings(&mut self, command: &Value) {
        let mut settings: Settings = match serde_json::from_value(command["settings"].clone()) {
            Ok(settings) => settings,
            Err(error) => {
                self.message(Err(format!("Die Einstellungen sind ungültig: {error}")));
                return;
            }
        };
        if settings.clips_dir.as_os_str().is_empty() {
            settings.clips_dir = self.settings.clips_dir.clone();
        }
        let settings = settings.sanitized();
        if !self.exports.is_empty() && settings.clips_dir != self.settings.clips_dir {
            self.message(Err(
                "Beende zuerst die Exportliste, bevor du den Clip-Ordner änderst.".into(),
            ));
            return;
        }
        let restricted = self.recording && self.settings.capture_changed(&settings);
        if restricted {
            self.message(Err(
                "Aufnahme-Einstellungen gelten erst nach dem Beenden der laufenden Aufnahme. Bitte zuerst die Aufnahme beenden.".into(),
            ));
            return;
        }
        let autostart = command["autostart"].as_bool().unwrap_or(self.autostart);
        if autostart != self.autostart {
            match platform::set_autostart(autostart) {
                Ok(()) => self.autostart = autostart,
                Err(error) => {
                    self.message(Err(error));
                    return;
                }
            }
        }
        match settings::save_settings(&self.config, &settings) {
            Ok(()) => {
                let hotkeys_changed = settings.hotkey_replay != self.settings.hotkey_replay
                    || settings.hotkey_replay_long != self.settings.hotkey_replay_long
                    || settings.replay_seconds_long != self.settings.replay_seconds_long
                    || settings.hotkey_marker != self.settings.hotkey_marker
                    || settings.hotkey_record != self.settings.hotkey_record;
                let folder_changed = settings.clips_dir != self.settings.clips_dir;
                let limit_changed = settings.storage_limit_gb != self.settings.storage_limit_gb;
                self.settings = settings;
                if hotkeys_changed {
                    self.apply_hotkeys();
                }
                self.apply_games();
                self.message(Ok("Einstellungen gespeichert.".into()));
                if folder_changed || limit_changed {
                    self.refresh();
                }
            }
            Err(error) => self.message(Err(error)),
        }
    }

    fn action(&mut self, command: Value, hwnd: isize, proxy: EventLoopProxy<AppEvent>) {
        let action = command["action"].as_str().unwrap_or("");
        if action == "cancel_export" {
            if let Some(id) = command["id"].as_u64() {
                self.exports.cancel(id);
            }
            return;
        }
        if action == "cancel_audio_test" {
            self.audio_test.cancel();
            return;
        }
        if self.closing {
            return;
        }
        if action == "ready" {
            return;
        }
        match action {
            "start" | "stop" => {
                self.recording_job(action, proxy);
                return;
            }
            "toggle" => {
                let next = if self.recording || self.capture.transitioning {
                    "stop"
                } else {
                    "start"
                };
                self.recording_job(next, proxy);
                return;
            }
            "mark" if !self.capture.transitioning => {
                self.mark();
                return;
            }
            _ => {}
        }
        if self.busy || self.capture.transitioning {
            self.message(Err("Einen Moment, der letzte Vorgang läuft noch.".into()));
            return;
        }
        if matches!(action, "rename" | "delete")
            && self
                .clip_path(&command)
                .ok()
                .is_some_and(|path| self.clip_locked(&path))
        {
            self.message(Err("Dieser Clip wird noch für einen Export benötigt. Beende oder entferne den Auftrag zuerst.".into()));
            return;
        }
        if matches!(action, "choose_folder" | "choose_ffmpeg")
            && (!self.exports.is_empty()
                || self.audio_test.active.is_some()
                || self.dragging.is_some())
        {
            self.message(Err("Beende zuerst die Exportliste.".into()));
            return;
        }
        match action {
            "audio_test" => {
                if let Err(error) = self.start_audio_test(&command, proxy) {
                    self.message(Err(error.clone()));
                    self.audio_test.error = Some(error);
                }
            }
            "drag_clip" => {
                if self.dragging.is_none() {
                    match self.clip_path(&command) {
                        Ok(path) => {
                            let cancel = drag::Control::default();
                            self.dragging = Some((path.clone(), cancel.clone()));
                            std::thread::spawn(move || {
                                let _ = proxy
                                    .send_event(AppEvent::DragFinished(drag::file(&path, cancel)));
                            });
                        }
                        Err(e) => self.message(Err(e)),
                    }
                }
            }
            "annotate" => {
                let result = self.clip_path(&command).and_then(|path| {
                    let tags = command["tags"]
                        .as_array()
                        .ok_or("Tags fehlen.")?
                        .iter()
                        .map(|t| t.as_str().map(str::to_owned).ok_or("Ungültiger Tag."))
                        .collect::<Result<Vec<_>, _>>()?;
                    let value = library::Annotation::normalized(
                        command["game"].as_str().unwrap_or(""),
                        &tags,
                    )?;
                    self.index
                        .annotate(library::index_key(&path), value, &self.config)
                });
                self.message(result.map(|_| "Spiel und Tags gespeichert.".into()));
            }
            "refresh" => self.refresh(),
            "rescan" => {
                self.displays = capture::displays();
                self.microphones = audio::capture_devices();
                self.default_output = audio::default_output_name();
                self.refresh();
            }
            "start" | "stop" | "replay" | "replay_long" => self.recording_job(action, proxy),
            "mark" => self.mark(),
            "save_marker" => {
                if let Some(id) = command["id"].as_u64() {
                    self.marker_job(id, proxy);
                }
            }
            "drop_marker" => {
                if let Some(id) = command["id"].as_u64() {
                    self.markers.retain(|marker| marker.id != id);
                    self.message(Ok("Markierung verworfen.".into()));
                }
            }
            "toggle" => {
                let next = if self.recording { "stop" } else { "start" };
                self.recording_job(next, proxy);
            }
            "import" => {
                if let Some(source) = platform::pick_file(hwnd, false) {
                    let dir = self.settings.clips_dir.clone();
                    self.file_job(proxy, "Clip wird importiert…", move || {
                        library::import_clip(&source, &dir).map(|path| JobOutcome {
                            message: "Clip importiert.".into(),
                            saved: Some(path),
                            session: None,
                            replay: false,
                        })
                    });
                }
            }
            "open_folder" => {
                let result = fs::create_dir_all(&self.settings.clips_dir)
                    .map_err(|e| e.to_string())
                    .and_then(|_| platform::open_folder(&self.settings.clips_dir));
                if let Err(error) = result {
                    self.message(Err(error));
                }
            }
            "choose_folder" if !self.recording => {
                if let Some(dir) = platform::pick_folder(hwnd) {
                    let mut settings = self.settings.clone();
                    settings.clips_dir = dir;
                    match settings::save_settings(&self.config, &settings) {
                        Ok(()) => {
                            self.settings = settings;
                            self.message(Ok(
                                "Clip-Ordner geändert. Bisherige Clips bleiben im alten Ordner."
                                    .into(),
                            ));
                            self.refresh();
                        }
                        Err(error) => self.message(Err(error)),
                    }
                }
            }
            "choose_ffmpeg" if !self.recording => {
                if let Some(path) = platform::pick_file(hwnd, true) {
                    let result = fs::create_dir_all(&self.config).and_then(|_| {
                        fs::write(
                            self.config.join("ffmpeg-path.txt"),
                            path.to_string_lossy().as_bytes(),
                        )
                    });
                    match result {
                        Ok(()) => {
                            self.recorder = Some(Recorder::new(path.clone()));
                            self.ffmpeg = Some(path);
                            self.capabilities = Capabilities::default();
                            self.probe_capabilities(proxy);
                            self.message(Ok(
                                "FFmpeg-Pfad gespeichert. Encoder werden geprüft…".into()
                            ));
                            self.refresh();
                        }
                        Err(error) => self.message(Err(error.to_string())),
                    }
                }
            }
            "reprobe" => {
                if self.ffmpeg.is_some() {
                    self.capabilities = Capabilities::default();
                    self.probe_capabilities(proxy);
                    self.message(Ok("Encoder werden erneut geprüft…".into()));
                }
            }
            "save_settings" => self.save_settings(&command),
            "list_apps" => {
                self.apps = games::candidate_apps();
            }
            "add_game" => {
                let entry = GameEntry {
                    exe: command["exe"].as_str().unwrap_or("").to_owned(),
                    name: command["name"].as_str().unwrap_or("").to_owned(),
                    title: String::new(),
                }
                .normalized();
                self.add_game(entry);
            }
            "pick_game" => {
                if let Some(path) = platform::pick_program(hwnd) {
                    let exe = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let name = games::display_name(&games::normalize_exe(&exe));
                    self.add_game(GameEntry::new(&exe, &name));
                }
            }
            "remove_game" => {
                let exe = games::normalize_exe(command["exe"].as_str().unwrap_or(""));
                let mut settings = self.settings.clone();
                settings.custom_games.retain(|g| g.exe != exe);
                match settings::save_settings(&self.config, &settings) {
                    Ok(()) => {
                        self.settings = settings;
                        self.apply_games();
                        self.message(Ok("Spiel aus der Liste entfernt.".into()));
                    }
                    Err(error) => self.message(Err(error)),
                }
            }
            "rename" => {
                let path = self.clip_path(&command);
                let name = command["name"].as_str().unwrap_or("").to_owned();
                match path {
                    Ok(path) => {
                        // So the marked moments follow the clip to its new name.
                        self.pending_rename = Some(library::index_key(&path));
                        self.file_job(proxy, "Clip wird umbenannt…", move || {
                            library::rename_clip(&path, &name).map(|path| JobOutcome {
                                message: "Clip umbenannt.".into(),
                                saved: Some(path),
                                session: None,
                                replay: false,
                            })
                        });
                    }
                    Err(error) => self.message(Err(error)),
                }
            }
            "delete" => match self.clip_path(&command) {
                Ok(path) => self.file_job(
                    proxy,
                    "Clip wird in den Papierkorb verschoben…",
                    move || {
                        platform::recycle(&path).map(|_| JobOutcome {
                            message: "Clip in den Papierkorb verschoben.".into(),
                            saved: None,
                            session: None,
                            replay: false,
                        })
                    },
                ),
                Err(error) => self.message(Err(error)),
            },
            "reveal" => match self.clip_path(&command) {
                Ok(path) => {
                    if let Err(error) = platform::reveal_in_explorer(&path) {
                        self.message(Err(error));
                    }
                }
                Err(error) => self.message(Err(error)),
            },
            "open_clip" => match self.clip_path(&command) {
                Ok(path) => {
                    if let Err(error) = platform::open_file(&path) {
                        self.message(Err(error));
                    }
                }
                Err(error) => self.message(Err(error)),
            },
            "highlights" => {
                let result = command["pieces"]
                    .as_array()
                    .ok_or("Wähle zuerst Ausschnitte aus.".to_string())
                    .and_then(|items| {
                        if items.is_empty() || items.len() > highlights::MAX_PIECES {
                            return Err("Ein Highlight darf 1 bis 20 Ausschnitte enthalten.".into());
                        }
                        items
                            .iter()
                            .map(|item| {
                                let path = self.clip_path(item)?;
                                Ok(highlights::Piece {
                                    crop: parse_crop(&item["crop"])?,
                                    markers: self.markers_of(&path),
                                    path,
                                    start: item["start"].as_f64().unwrap_or(f64::NAN),
                                    end: item["end"].as_f64().unwrap_or(f64::NAN),
                                })
                            })
                            .collect::<Result<Vec<_>, String>>()
                    });
                match (result, self.ffmpeg.clone()) {
                    (Ok(pieces), Some(ffmpeg)) => {
                        let profile = discord::Profile::from_id(
                            command["mode"].as_str().unwrap_or("precise"),
                        );
                        let width = match command["portrait_width"].as_u64() {
                            Some(720) => Some(720),
                            Some(1080) => Some(1080),
                            _ if command["portrait_width"].is_null() => None,
                            _ => {
                                self.message(Err("Ungültiges Hochkantformat.".into()));
                                return;
                            }
                        };
                        let dir = self.settings.clips_dir.clone();
                        let sources = pieces.iter().map(|p| p.path.clone()).collect();
                        let events = proxy.clone();
                        self.queue_export(format!("Highlights · {} Ausschnitte", pieces.len()), sources, Box::new(move |control| {
                            let output = unique_trim_path(&dir, "Highlights", profile.map_or("Mix", |p| p.suffix()));
                            let markers = highlights::export_controlled(&ffmpeg, &pieces, &output, profile, width, &control)?;
                            let _ = events.send_event(AppEvent::ClipMarkers(output.clone(), markers));
                            Ok(JobOutcome { message: "Highlight-Clip gespeichert. Die Originale bleiben erhalten.".into(), saved: Some(output), session: None, replay: false })
                        }), proxy);
                    }
                    (Err(error), _) => self.message(Err(error)),
                    (_, None) => self.message(Err("Bitte zuerst FFmpeg auswählen.".into())),
                }
            }
            "favorite" => match self.clip_path(&command) {
                Ok(path) => {
                    let key = library::index_key(&path);
                    if command["favorite"].as_bool().unwrap_or(false) {
                        self.index.favorites.insert(key);
                    } else {
                        self.index.favorites.remove(&key);
                    }
                    self.index.save(&self.config);
                }
                Err(error) => self.message(Err(error)),
            },
            "trim" => match (self.clip_path(&command), self.ffmpeg.clone()) {
                (Ok(path), Some(ffmpeg)) => {
                    let start = command["start"].as_f64().unwrap_or(f64::NAN);
                    let end = command["end"].as_f64().unwrap_or(f64::NAN);
                    let crop = match parse_crop(&command["crop"]) {
                        Ok(crop) => crop,
                        Err(error) => {
                            self.message(Err(error));
                            return;
                        }
                    };
                    let profile =
                        discord::Profile::from_id(command["mode"].as_str().unwrap_or("precise"));
                    let mode = if command["mode"].as_str() == Some("fast") {
                        TrimMode::Fast
                    } else {
                        TrimMode::Precise
                    };
                    let stem = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let dir = self.settings.clips_dir.clone();
                    let marks = self.markers_of(&path);
                    let annotation = self.annotation(&path);
                    let events = proxy.clone();
                    let label = format!(
                        "{} · {}",
                        stem,
                        profile.map_or(
                            if crop.is_some() {
                                "Hochkant"
                            } else {
                                "Schnitt"
                            },
                            |p| p.label()
                        )
                    );
                    self.queue_export(label, vec![path.clone()], Box::new(move |control| {
                        control.check()?;
                        let duration = recorder::probe_duration(&ffmpeg, &path)?;
                        let (start, end) = mechanics::editor_range(start, end, duration).ok_or("Der Ausschnitt muss mindestens zwei Sekunden lang sein und innerhalb des Videos liegen.")?;
                        let suffix = profile.map_or(if crop.is_some() { "Hochkant" } else { "Schnitt" }, |p| p.suffix());
                        let output = unique_trim_path(&dir, &stem, suffix);
                        let workspace = export::Snapshot::new(&dir, &[])?;
                        let staged = workspace.dir.join("clip.mp4");
                        let (from, message) = if let Some(profile) = profile {
                            let bytes = discord::export_controlled(&ffmpeg, &path, &staged, start, end, profile, crop, &control.part(0.0, 0.99))?;
                            (start, format!("{}: {:.2} MB gespeichert und Dateilimit geprüft.", profile.label(), bytes as f64 / 1_000_000.0))
                        } else {
                            let from = recorder::trim_controlled(&ffmpeg, &path, &staged, start, end, mode, crop, &control.part(0.0, 0.99))?;
                            (from, "Neuer Clip gespeichert. Das Original bleibt erhalten.".into())
                        };
                        control.check()?; workspace.publish(&staged, &output)?;
                        send_markers(&events, &output, &marks, from, end - from);
                        let _ = events.send_event(AppEvent::ClipAnnotation(output.clone(), annotation));
                        Ok(JobOutcome { message, saved: Some(output), session: None, replay: false })
                    }), proxy);
                }
                (Err(error), _) => self.message(Err(error)),
                (_, None) => self.message(Err("Bitte zuerst FFmpeg auswählen.".into())),
            },
            "choose_folder" | "choose_ffmpeg" => {
                self.message(Err("Bitte zuerst die laufende Aufnahme beenden.".into()))
            }
            _ => (),
        }
    }

    fn add_game(&mut self, entry: GameEntry) {
        if !entry.is_valid() {
            self.message(Err("Bitte ein Programm mit Dateinamen wählen.".into()));
            return;
        }
        let mut settings = self.settings.clone();
        if settings.custom_games.iter().any(|g| g.exe == entry.exe) {
            self.message(Err(format!("{} steht bereits in der Liste.", entry.name)));
            return;
        }
        let name = entry.name.clone();
        settings.custom_games.push(entry);
        match settings::save_settings(&self.config, &settings) {
            Ok(()) => {
                self.settings = settings;
                self.apply_games();
                self.message(Ok(format!("{name} wird jetzt erkannt.")));
            }
            Err(error) => self.message(Err(error)),
        }
    }

    fn probe_capabilities(&mut self, proxy: EventLoopProxy<AppEvent>) {
        let Some(ffmpeg) = self.ffmpeg.clone() else {
            return;
        };
        let config = self.config.clone();
        std::thread::spawn(move || {
            let capabilities = capture::probe_capabilities(&ffmpeg, &config);
            let _ = proxy.send_event(AppEvent::Capabilities(capabilities));
        });
    }

    fn finish(&mut self, result: JobResult, proxy: EventLoopProxy<AppEvent>) {
        let capture_result = result.restore_recorder;
        if capture_result {
            self.recorder = result.recorder;
            self.capture.transitioning = false;
        } else {
            self.busy = false;
        }
        if let Some(recorder) = &self.recorder {
            self.recording = recorder.is_recording();
            self.session = recorder.info().cloned();
            self.elapsed = recorder.elapsed().as_secs();
        }
        let marked = if capture_result {
            None
        } else {
            self.pending_marker.take()
        };
        let renamed = if capture_result {
            None
        } else {
            self.pending_rename.take()
        };
        // Markers belong to one session: once it ends, the buffer behind them
        // is gone, so they go with it.
        if !self.recording && !self.capture.transitioning && !self.capture.wants_start() {
            self.session_settings = None;
            self.markers.clear();
            self.buffered = None;
        }
        match result.result {
            Ok(outcome) => {
                if let Some(id) = marked {
                    self.markers.retain(|marker| marker.id != id);
                }
                // A renamed clip keeps its marked moments under the new name.
                if let (Some(old), Some(path)) = (renamed, outcome.saved.as_ref()) {
                    if self.index.favorites.remove(&old) {
                        self.index.favorites.insert(library::index_key(path));
                        self.index.save(&self.config);
                    }
                    let annotation = self
                        .index
                        .annotations
                        .remove(&old)
                        .unwrap_or_else(|| self.annotation(Path::new(&old)));
                    self.index
                        .annotations
                        .insert(library::index_key(path), annotation);
                    let moved = self
                        .index
                        .entries
                        .get(&old)
                        .map(|entry| entry.markers.clone())
                        .unwrap_or_default();
                    self.remember_markers(path, moved);
                    self.index.save(&self.config);
                }

                let has_warnings = outcome
                    .session
                    .as_ref()
                    .is_some_and(|s| !s.warnings.is_empty());
                let mut message = outcome.message.clone();
                if let Some(session) = &outcome.session {
                    if !session.warnings.is_empty() {
                        message = format!("{message} Hinweis: {}", session.warnings.join(" "));
                    }
                }
                self.message(Ok(message.clone()));
                self.notify(&message, has_warnings, outcome.saved.clone());
                if outcome.replay {
                    let name = outcome
                        .saved
                        .as_ref()
                        .and_then(|p| p.file_stem())
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    self.banner(&format!("Moment gesichert · {name}"), false);
                    if self.settings.clip_sound {
                        overlay::play_chime();
                    }
                } else if outcome.session.is_some() {
                    let text = match &self.active_game {
                        Some(game) => format!("Aufnahme läuft · {}", game.name),
                        None => "Aufnahme läuft".to_owned(),
                    };
                    self.banner(&text, false);
                }
                if outcome.session.is_some() {
                    self.session = outcome.session;
                }
            }
            Err(error) => {
                self.message(Err(error.clone()));
                self.notify(&error, true, None);
                if self.recording || self.active_game.is_some() {
                    self.banner("Stillclip: Vorgang fehlgeschlagen", true);
                }
            }
        }
        if !self.recording && !self.capture.transitioning && !self.capture.wants_start() {
            self.auto_started = false;
        }
        self.refresh();
        self.update_tray();
        if self.closing {
            self.capture.request(false);
        }
        self.advance_capture(proxy);
    }

    fn update_tray(&self) {
        if let Some(tray) = &self.tray {
            let tooltip = if self.recording {
                match &self.active_game {
                    Some(game) => format!(
                        "Stillclip · {} · Aufnahme läuft ({})",
                        game.name,
                        self.settings.hotkey_replay.label()
                    ),
                    None => format!(
                        "Stillclip · Aufnahme läuft ({})",
                        self.settings.hotkey_replay.label()
                    ),
                }
            } else {
                "Stillclip · bereit".to_owned()
            };
            tray.update(self.recording, &tooltip);
        }
    }
}

/// Marked moments that fall inside a saved clip, as seconds from its start.
fn marker_offsets(marks: &[f64], start: f64, length: f64) -> Vec<f64> {
    let mut offsets: Vec<f64> = marks
        .iter()
        .filter(|at| **at >= start - 0.05 && **at <= start + length + 0.05)
        .map(|at| (at - start).clamp(0.0, length))
        .collect();
    offsets.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    offsets
}

/// Hands the marked moments of a finished clip to the event loop, which keeps
/// them with the clip so the editor can show them.
fn send_markers(
    proxy: &EventLoopProxy<AppEvent>,
    path: &Path,
    marks: &[f64],
    start: f64,
    length: f64,
) {
    let offsets = marker_offsets(marks, start, length);
    if !offsets.is_empty() {
        let _ = proxy.send_event(AppEvent::ClipMarkers(path.to_path_buf(), offsets));
    }
}

/// What the buffer can still deliver for a marked moment: whether its whole
/// window has been written, and how many seconds of the wanted window are
/// left. `buffered` is the finished part of the recording, `(from, to)`.
fn marker_state(at: f64, before: f64, after: f64, buffered: Option<(f64, f64)>) -> (bool, f64) {
    let Some((from, to)) = buffered else {
        return (false, 0.0);
    };
    let ready = at + after <= to + 0.001;
    let available = ((at + after).min(to) - (at - before).max(from)).max(0.0);
    (ready, available)
}

fn parse_crop(value: &Value) -> Result<Option<portrait::Crop>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let crop: portrait::Crop =
        serde_json::from_value(value.clone()).map_err(|_| "Ungültiger Hochkant-Ausschnitt.")?;
    crop.validate().map(Some)
}

fn unique_trim_path(dir: &Path, stem: &str, suffix: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{stem} - {suffix}.mp4"));
    let mut number = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} - {suffix} {number}.mp4"));
        number += 1;
    }
    candidate
}

fn asset(path: &str) -> Option<(&'static [u8], &'static str)> {
    Some(match path {
        "/" | "/index.html" => (
            include_bytes!("../ui/index.html"),
            "text/html; charset=utf-8",
        ),
        "/app.css" => (include_bytes!("../ui/app.css"), "text/css; charset=utf-8"),
        "/app.js" => (
            include_bytes!("../ui/app.js"),
            "text/javascript; charset=utf-8",
        ),
        _ => return None,
    })
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(_instance) = platform::InstanceGuard::acquire()? else {
        return Ok(());
    };
    let mut screenshot_path = None;
    let mut screenshot_page = String::from("library");
    let mut start_hidden = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--screenshot" {
            screenshot_path = Some(PathBuf::from(
                args.next()
                    .ok_or("--screenshot benötigt einen PNG-Dateipfad")?,
            ));
        } else if arg == "--page" {
            screenshot_page = args
                .next()
                .ok_or("--page benötigt einen Namen (library, settings, editor)")?
                .to_string_lossy()
                .into_owned();
        } else if arg == "--minimized" {
            start_hidden = true;
        } else {
            return Err("Unbekanntes Argument. Unterstützt: --screenshot <file.png> [--page name], --minimized".into());
        }
    }
    let checking = screenshot_path.is_some();
    if checking {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(45));
            eprintln!("Stillclip: WebView-Fotoprüfung nach 45 Sekunden abgebrochen.");
            std::process::exit(1);
        });
    }
    eprintln!("Stillclip: loading local settings");
    let mut app = App::new();
    start_hidden = start_hidden || (app.settings.start_minimized && !checking);
    eprintln!("Stillclip: creating event loop");
    fs::create_dir_all(&app.config)?;
    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    if let Ok(mut slot) = App::proxy_slot().write() {
        *slot = Some(proxy.clone());
    }
    app.schedule_indexing();
    let ipc_proxy = proxy.clone();
    let window_icon = Icon::from_rgba(icon::rgba(icon::SIZE), icon::SIZE, icon::SIZE).ok();
    let window = WindowBuilder::new()
        .with_title("Stillclip")
        .with_focused(!checking && !start_hidden)
        .with_always_on_bottom(checking)
        .with_visible(!start_hidden)
        .with_window_icon(window_icon)
        .with_inner_size(LogicalSize::new(1280.0, 840.0))
        .with_min_inner_size(LogicalSize::new(720.0, 600.0))
        .build(&event_loop)?;
    app.window_visible = !start_hidden;
    platform::set_window_icon(window.hwnd() as isize);
    platform::style_window(window.hwnd() as isize);
    eprintln!("Stillclip: creating WebView");
    let mut context = WebContext::new(Some(app.config.join("webview")));
    let media = app.media.clone();
    let view = WebViewBuilder::new_with_web_context(&mut context)
        .with_background_color((9, 9, 9, 255))
        .with_custom_protocol("app".into(), |_, request| {
            match asset(request.uri().path()) {
                Some((bytes, mime)) => Response::builder()
                    .status(200)
                    .header("Content-Type", mime)
                    .body(Cow::Borrowed(bytes))
                    .unwrap(),
                None => Response::builder()
                    .status(404)
                    .header("Content-Type", "text/plain")
                    .body(Cow::Borrowed(b"Not found".as_slice()))
                    .unwrap(),
            }
        })
        .with_asynchronous_custom_protocol("clip".into(), move |_, request, responder| {
            let key = request.uri().path().trim_start_matches('/').to_owned();
            let path = media.read().ok().and_then(|files| files.get(&key).cloned());
            std::thread::spawn(move || {
                responder.respond(match path {
                    Some(path) => media::serve(&path, &request),
                    None => Response::builder().status(404).body(Vec::new()).unwrap(),
                });
            });
        })
        .with_navigation_handler(|url| {
            matches!(
                url.as_str(),
                "http://app.localhost/"
                    | "http://app.localhost/index.html"
                    | "app://localhost/index.html"
            )
        })
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_ipc_handler(move |request| {
            if matches!(
                request.uri().host(),
                Some("app.localhost") | Some("localhost")
            ) {
                if let Ok(value) = serde_json::from_str(request.body()) {
                    let _ = ipc_proxy.send_event(AppEvent::Action(value));
                }
            }
        })
        .with_devtools(cfg!(debug_assertions))
        .with_url("app://localhost/index.html")
        .build(&window)?;
    eprintln!("Stillclip: ready");
    if !checking {
        app.hotkeys = Hotkeys::start(proxy.clone());
        app.apply_hotkeys();
        app.tray = Tray::start(proxy.clone());
        app.update_tray();
        app.overlay = Overlay::start();
        app.watcher = Some(GameWatcher::start(proxy.clone()));
        app.apply_games();
        if app.ffmpeg.is_some() && !app.capabilities.probed {
            app.probe_capabilities(proxy.clone());
        }
    }
    let mut ready_at = None;
    let mut screenshot_requested = false;
    let mut launched = false;
    let mut next_tick = Instant::now() + Duration::from_millis(500);
    let mut ticks: u64 = 0;
    let show_window = |window: &tao::window::Window, app: &mut App| {
        window.set_visible(true);
        window.set_minimized(false);
        window.set_focus();
        app.window_visible = true;
    };
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(next_tick);
        match event {
            Event::UserEvent(AppEvent::Action(command)) => {
                if command["action"] == "ready" {
                    ready_at = Some(Instant::now());
                    if checking {
                        app.sync(&view);
                        let _ = view.evaluate_script(&format!(
                            "window.showPage && window.showPage({});",
                            serde_json::Value::String(screenshot_page.clone())
                        ));
                    }
                    if !launched && !checking {
                        launched = true;
                        if let Some(path) = app.recovered.take() {
                            app.notify(
                                "Eine unterbrochene Aufnahme wurde wiederhergestellt.",
                                false,
                                Some(path),
                            );
                        }
                        if app.settings.record_on_launch && app.ffmpeg.is_some() {
                            app.recording_job("start", proxy.clone());
                        }
                    }
                }
                if !checking {
                    app.action(command, window.hwnd() as isize, proxy.clone());
                }
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::ScreenshotFinished(result)) => {
                if let Err(error) = result {
                    eprintln!("Stillclip: {error}");
                    std::process::exit(1);
                }
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(AppEvent::Notice(message)) => {
                app.message(Err(message));
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::Hotkey(id)) => {
                match id {
                    HOTKEY_REPLAY if app.recording && !app.busy && !app.capture.transitioning => {
                        app.recording_job("replay", proxy.clone());
                    }
                    HOTKEY_REPLAY_LONG if app.recording && !app.busy && !app.capture.transitioning => {
                        app.recording_job("replay_long", proxy.clone());
                    }
                    HOTKEY_MARKER if !app.capture.transitioning => app.mark(),
                    HOTKEY_REPLAY | HOTKEY_REPLAY_LONG if !app.recording && !app.busy && !app.capture.transitioning => {
                        app.notify("Es läuft keine Aufnahme. Starte zuerst die Aufnahme.", true, None);
                    }
                    HOTKEY_RECORD => {
                        let next = if app.recording || app.capture.transitioning { "stop" } else { "start" };
                        app.recording_job(next, proxy.clone());
                    }
                    _ => {}
                }
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::HotkeysApplied(failed)) => {
                app.hotkey_failures = failed.clone();
                if !failed.is_empty() {
                    let names: Vec<String> = failed
                        .iter()
                        .map(|id| match *id {
                            HOTKEY_REPLAY => app.settings.hotkey_replay.label(),
                            HOTKEY_REPLAY_LONG => app.settings.hotkey_replay_long.label(),
                            HOTKEY_MARKER => app.settings.hotkey_marker.label(),
                            _ => app.settings.hotkey_record.label(),
                        })
                        .collect();
                    app.notify(
                        &format!(
                            "Tastenkürzel {} ist bereits von einem anderen Programm belegt.",
                            names.join(", ")
                        ),
                        true,
                        None,
                    );
                }
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::Tray(action)) => {
                match action {
                    TrayAction::Show => show_window(&window, &mut app),
                    TrayAction::ToggleRecording => {
                        let next = if app.recording || app.capture.transitioning { "stop" } else { "start" };
                        app.recording_job(next, proxy.clone());
                    }
                    TrayAction::SaveReplay if app.recording && !app.busy && !app.capture.transitioning => {
                        app.recording_job("replay", proxy.clone());
                    }
                    TrayAction::Quit => {
                        app.closing = true;
                        app.cancel_background();
                        if app.busy || app.capture.transitioning || !app.exports.is_empty() || app.audio_test.active.is_some() || app.dragging.is_some() {
                            app.capture.request(false);
                            app.advance_capture(proxy.clone());
                            app.status = "Vorgang wird abgeschlossen, danach schließt Stillclip…".into();
                        } else if app.recording {
                            app.recording_job("stop", proxy.clone());
                        } else {
                            *control_flow = ControlFlow::Exit;
                        }
                    }
                    _ => {}
                }
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::Capabilities(capabilities)) => {
                app.capabilities = capabilities;
                app.advance_capture(proxy.clone());
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::ClipMarkers(path, offsets)) => {
                app.remember_markers(&path, offsets);
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::Indexed(key, mut entry)) => {
                // Probing knows nothing about marked moments; keep the ones we have.
                if let Some(known) = app.index.entries.get(&key) {
                    entry.markers = known.markers.clone();
                }
                app.index.entries.insert(key, entry);
                app.update_media_map();
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::IndexingDone) => {
                app.indexing = false;
                app.index.save(&app.config);
                app.schedule_indexing();
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::GameStarted(game, monitor)) => {
                let name = game.name.clone();
                app.active_game = Some(game);
                app.active_game_display = (!monitor.is_empty()).then_some(monitor);
                if app.settings.auto_record_games && app.ffmpeg.is_some() && !app.closing {
                    if !app.recording || app.auto_started {
                        app.auto_started = true;
                        app.restarts.clear();
                        app.notify(&format!("{name} erkannt – die Aufnahme startet."), false, None);
                        app.recording_job("start", proxy.clone());
                    } else {
                        app.notify(&format!("{name} erkannt – Clips werden danach benannt."), false, None);
                    }
                }
                app.update_tray();
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::GameStopped(game)) => {
                let name = game.name.clone();
                if app.active_game.as_ref().is_some_and(|g| g.exe == game.exe) {
                    app.active_game = None;
                    app.active_game_display = None;
                }
                if app.auto_started {
                    app.auto_started = false;
                    app.notify(&format!("{name} beendet – die Aufnahme wird gestoppt."), false, None);
                    app.recording_job("stop", proxy.clone());
                }
                app.update_tray();
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::AudioMeters(id, meters)) => { if id == app.audio_test.id { app.audio_test.meters = meters; } }
            Event::UserEvent(AppEvent::AudioFinished(id, result)) => {
                if id == app.audio_test.id {
                    app.audio_test.active = None; app.audio_test.meters.clear(); app.audio_test.levels.clear();
                    match result {
                        Ok(preview) => { app.audio_test.preview = Some(preview); app.audio_test.error = None; }
                        Err(e) => { app.audio_test.error = (e != jobs::CANCELLED).then_some(e); }
                    }
                    app.update_media_map(); app.sync(&view);
                }
            }
            Event::UserEvent(AppEvent::DragFinished(result)) => {
                app.dragging = None;
                if let Err(e) = result { app.message(Err(e)); }
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::ClipAnnotation(path, annotation)) => {
                app.index.annotations.insert(library::index_key(&path), annotation); app.index.save(&app.config);
            }
            Event::UserEvent(AppEvent::ExportFinished(id, result)) => {
                app.finish_export(id, result, proxy.clone()); app.sync(&view);
                if app.closing && !app.recording && !app.busy && !app.capture.transitioning && app.exports.is_empty() && app.audio_test.active.is_none() && app.dragging.is_none() { *control_flow = ControlFlow::Exit; }
            }
            Event::UserEvent(AppEvent::Finished(result)) => {
                app.finish(result, proxy.clone());
                app.sync(&view);
                if app.closing && !app.recording && !app.busy && !app.capture.transitioning && app.exports.is_empty() && app.audio_test.active.is_none() && app.dragging.is_none() {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if app.settings.close_to_tray && app.tray.is_some() && !app.closing {
                    window.set_visible(false);
                    app.window_visible = false;
                    if !app.tray_hint_shown {
                        app.tray_hint_shown = true;
                        if let Some(tray) = &app.tray {
                            tray.notify(
                                "Stillclip läuft weiter",
                                "Die Aufnahme und dein Tastenkürzel bleiben aktiv. Rechtsklick auf das Symbol zum Beenden.",
                            );
                        }
                    }
                } else {
                    app.closing = true;
                    app.cancel_background();
                    if app.busy || app.capture.transitioning || !app.exports.is_empty() || app.audio_test.active.is_some() || app.dragging.is_some() {
                        app.capture.request(false);
                        app.advance_capture(proxy.clone());
                        app.status = "Vorgang wird abgeschlossen, danach schließt Stillclip…".into();
                        app.sync(&view);
                    } else if app.recording {
                        app.recording_job("stop", proxy.clone());
                        app.sync(&view);
                    } else {
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            Event::NewEvents(_) if Instant::now() >= next_tick => {
                next_tick = Instant::now() + Duration::from_millis(500);
                ticks = ticks.wrapping_add(1);
                // Free space changes slowly; asking the drive every ten seconds
                // is plenty and keeps the warning in the UI current.
                if ticks % 20 == 1 && !checking {
                    let free = platform::free_bytes(&app.settings.clips_dir).unwrap_or(0);
                    if free != app.free_bytes {
                        app.free_bytes = free;
                        app.sync(&view);
                    }
                }
                if checking
                    && !screenshot_requested
                    && ready_at.is_some_and(|t| t.elapsed() >= Duration::from_secs(2))
                {
                    screenshot_requested = true;
                    if let Err(error) =
                        screenshot::capture(&view, screenshot_path.clone().unwrap(), proxy.clone())
                    {
                        eprintln!("Stillclip: {error}");
                        std::process::exit(1);
                    }
                }
                if app.closing && !app.recording && !app.busy && !app.capture.transitioning && app.exports.is_empty() && app.audio_test.active.is_none() && app.dragging.is_none() { *control_flow = ControlFlow::Exit; }
                if app.audio_test.active.is_some() { app.audio_test.poll(); app.sync(&view); }
                app.start_exports(proxy.clone());
                if !app.exports.is_empty() { app.sync(&view); }
                if let Some(recorder) = app.recorder.as_mut() {
                    let issue = recorder.poll();
                    let elapsed = recorder.elapsed().as_secs();
                    let recording = recorder.is_recording();
                    let buffer_bytes = if recording { recorder.buffer_bytes() } else { 0 };
                    // Only interesting while markers wait for their clip.
                    let buffered = if recording && !app.markers.is_empty() {
                        recorder.buffered_span()
                    } else {
                        None
                    };
                    let changed = elapsed != app.elapsed
                        || recording != app.recording
                        || issue.is_some()
                        || buffer_bytes != app.buffer_bytes
                        || buffered != app.buffered;
                    app.elapsed = elapsed;
                    app.buffer_bytes = buffer_bytes;
                    app.buffered = buffered;
                    if recording != app.recording {
                        app.recording = recording;
                        app.session = recorder.info().cloned();
                        app.update_tray();
                    }
                    if let Some(issue) = issue {
                        app.handle_issue(issue, proxy.clone());
                    }
                    if changed {
                        app.sync(&view);
                    }
                }
            }
            Event::LoopDestroyed => {
                app.cancel_background();
                app.index.save(&app.config);
                app.recorder.take();
                app.watcher.take();
                app.overlay.take();
                app.tray.take();
                app.hotkeys.take();
            }
            _ => (),
        }
    });
}

fn main() {
    if let Err(error) = run() {
        if std::env::args_os().any(|arg| arg == "--screenshot") {
            eprintln!("Stillclip: {error}");
            std::process::exit(1);
        }
        let text: Vec<u16> = format!(
            "Stillclip konnte nicht gestartet werden:\n{error}\n\nBenötigt wird die Microsoft Edge WebView2-Laufzeit."
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let title: Vec<u16> = "Stillclip".encode_utf16().chain(Some(0)).collect();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                0x10,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_moments_become_positions_inside_the_saved_clip() {
        let round = |values: Vec<f64>| -> Vec<f64> {
            values
                .iter()
                .map(|v| (v * 1000.0).round() / 1000.0)
                .collect()
        };
        // A thirty second replay saved at session time 100..130.
        let marks = [40.0, 101.5, 112.0, 129.9, 200.0];
        assert_eq!(
            round(marker_offsets(&marks, 100.0, 30.0)),
            [1.5, 12.0, 29.9]
        );
        // Nothing inside the window: no marks for this clip.
        assert!(marker_offsets(&marks, 150.0, 30.0).is_empty());
        // The keyframe cut starts the clip a moment early; the mark moves with it.
        assert_eq!(round(marker_offsets(&[20.0], 19.6, 30.0)), [0.4]);
        // A mark right on either edge still counts and stays in range.
        assert_eq!(
            round(marker_offsets(&[100.0, 130.0], 100.0, 30.0)),
            [0.0, 30.0]
        );
        // The clip around a marked moment has it exactly after the lead.
        assert_eq!(round(marker_offsets(&[60.0], 40.0, 30.0)), [20.0]);
    }

    #[test]
    fn marked_moments_report_what_the_buffer_still_holds() {
        // Nothing finished yet: the moment is not ready and nothing is there.
        assert_eq!(marker_state(3.0, 20.0, 10.0, None), (false, 0.0));
        // Marked three seconds ago, ten seconds of tail wanted: still running.
        let (ready, available) = marker_state(100.0, 20.0, 10.0, Some((40.0, 103.0)));
        assert!(!ready);
        assert_eq!(available, 23.0);
        // The whole window is written: ready, and the full thirty seconds.
        let (ready, available) = marker_state(100.0, 20.0, 10.0, Some((40.0, 115.0)));
        assert!(ready);
        assert_eq!(available, 30.0);
        // The ring has eaten into the lead: ready, but shorter than wanted.
        let (ready, available) = marker_state(100.0, 20.0, 10.0, Some((95.0, 150.0)));
        assert!(ready);
        assert_eq!(available, 15.0);
        // Long gone: nothing left to cut.
        let (ready, available) = marker_state(100.0, 20.0, 10.0, Some((200.0, 260.0)));
        assert!(ready);
        assert_eq!(available, 0.0);
        // A tail of zero seconds is ready as soon as the mark itself is written.
        assert_eq!(
            marker_state(50.0, 15.0, 0.0, Some((0.0, 50.0))),
            (true, 15.0)
        );
    }
}
