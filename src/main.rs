#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod capture;
mod encoding;
mod games;
mod icon;
mod library;
mod mechanics;
mod media;
mod overlay;
mod platform;
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
/// How often a capture that died on its own is restarted, and within which span.
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(300);
const FPS_CHOICES: [u32; 5] = [24, 30, 60, 120, 144];

pub enum AppEvent {
    Action(Value),
    Finished(JobResult),
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
    ffmpeg: Option<PathBuf>,
    recorder: Option<Recorder>,
    clips: Vec<(u64, Clip)>,
    media: MediaFiles,
    next_id: u64,
    index: Index,
    indexing: bool,
    busy: bool,
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
    pending_start: bool,
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
        let recovered = ffmpeg
            .as_deref()
            .and_then(|f| recorder::recover_leftovers(f, &settings.clips_dir).ok().flatten());
        let free_bytes = platform::free_bytes(&settings.clips_dir).unwrap_or(0);
        let mut app = Self {
            index: Index::load(&config),
            config,
            settings,
            ffmpeg,
            recorder,
            clips: Vec::new(),
            media: Arc::new(RwLock::new(HashMap::new())),
            next_id: 1,
            indexing: false,
            busy: false,
            recording: false,
            elapsed: 0,
            status: "Bereit für deinen nächsten Moment.".into(),
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
            pending_start: false,
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
            Some(d) => (d.x + d.width as i32 - 28, d.y + 28, (d.width as i32 / 2).max(320)),
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
                tray.notify(if error { "Stillclip – Hinweis" } else { "Stillclip" }, text);
            }
        }
    }

    /// Deletes the oldest clips when the folder outgrew its limit. Only runs
    /// when the user set one; the newest clip always survives.
    fn enforce_storage_limit(&mut self, clips: &mut Vec<Clip>) {
        let doomed = library::over_limit(clips, self.settings.storage_limit_bytes());
        if doomed.is_empty() {
            return;
        }
        let mut freed = 0;
        let mut count = 0;
        for clip in &doomed {
            if fs::remove_file(&clip.path).is_ok() {
                freed += clip.bytes;
                count += 1;
            }
        }
        if count == 0 {
            return;
        }
        clips.retain(|clip| !doomed.iter().any(|gone| gone.path == clip.path));
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
        self.notify(&text, false, None);
    }

    fn refresh(&mut self) {
        match library::scan_clips(&self.settings.clips_dir) {
            Ok(mut clips) => {
                self.enforce_storage_limit(&mut clips);
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
                self.index.prune(&plain, &thumbs);
                self.update_media_map();
                self.schedule_indexing();
            }
            Err(error) => self.message(Err(error)),
        }
    }

    fn update_media_map(&self) {
        if let Ok(mut files) = self.media.write() {
            files.clear();
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
                        let at = (info.duration * 0.15).clamp(0.0, 3.0).min((info.duration - 0.1).max(0.0));
                        let thumbnail_path = thumbs.join(&thumbnail);
                        let has_thumbnail = thumbnail_path.is_file()
                            || recorder::make_thumbnail(&ffmpeg, &path, &thumbnail_path, at).is_ok();
                        IndexEntry {
                            bytes,
                            modified,
                            info,
                            thumbnail: has_thumbnail.then_some(thumbnail),
                            failed: false,
                        }
                    }
                    Err(_) => IndexEntry {
                        bytes,
                        modified,
                        info: Default::default(),
                        thumbnail: None,
                        failed: true,
                    },
                };
                if proxy.send_event(AppEvent::Indexed(key, entry)).is_err() {
                    return;
                }
            }
            let _ = proxy.send_event(AppEvent::IndexingDone);
        });
    }

    fn clip_json(&self, id: u64, clip: &Clip) -> Value {
        let entry = self.index.current(clip);
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
        let selected_encoder = capture::select_encoder(self.settings.encoder, &self.capabilities.encoders);
        let state = json!({
            "clips": clips,
            "recording": self.recording,
            "elapsed": self.elapsed,
            "busy": self.busy,
            "status": self.status,
            "error": self.error,
            "ffmpeg_available": self.ffmpeg.is_some(),
            "ffmpeg_path": self.ffmpeg,
            "clips_dir": self.settings.clips_dir,
            "settings": self.settings,
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
                "encoders": self.capabilities.encoders.iter().map(|e| e.id()).collect::<Vec<_>>(),
            },
            "encoders": encoders,
            "selected_encoder": selected_encoder.label(),
            "session": self.session,
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
                "record": self.settings.hotkey_record.label(),
                "failed": self.hotkey_failures,
            },
            "tray": self.tray.is_some(),
            "version": env!("CARGO_PKG_VERSION"),
            "recovered": self.recovered,
        });
        let _ = view.evaluate_script(&format!("window.setState && window.setState({state});"));
    }

    fn clip_path(&self, command: &Value) -> Result<PathBuf, String> {
        let id = command["id"].as_u64().ok_or("Bitte einen Clip auswählen.")?;
        self.clips
            .iter()
            .find(|(key, _)| *key == id)
            .map(|(_, c)| c.path.clone())
            .ok_or_else(|| "Der Clip ist nicht mehr in der Bibliothek. Bitte aktualisieren.".into())
    }

    fn session_config(&self) -> Result<SessionConfig, String> {
        let display = self
            .capture_display()
            .cloned()
            .ok_or("Es wurde kein Bildschirm gefunden.")?;
        Ok(SessionConfig {
            display,
            encoder: capture::select_encoder(self.settings.encoder, &self.capabilities.encoders),
            capabilities: self.capabilities.clone(),
            fps: self.settings.fps,
            quality: self.settings.quality,
            show_cursor: self.settings.show_cursor,
            system_audio: self.settings.system_audio,
            microphone: self
                .settings
                .microphone
                .then(|| self.settings.microphone_device.clone()),
            microphone_separate_track: self.settings.microphone_separate_track,
            system_gain: settings::gain(self.settings.system_volume),
            microphone_gain: settings::gain(self.settings.microphone_volume),
            replay_seconds: self.settings.buffer_seconds(),
            keep_full_session: self.settings.keep_full_session,
            min_free_bytes: self.settings.min_free_bytes(),
            precise_clips: self.settings.precise_clips,
            clips_dir: self.settings.clips_dir.clone(),
        })
    }

    fn recording_job(&mut self, action: &str, proxy: EventLoopProxy<AppEvent>) {
        if action == "start" && !self.capabilities.probed && self.ffmpeg.is_some() {
            self.pending_start = true;
            self.status = "FFmpeg wird geprüft, die Aufnahme startet gleich…".into();
            self.error = false;
            return;
        }
        let Some(mut recorder) = self.recorder.take() else {
            self.message(Err("Bitte zuerst FFmpeg in den Einstellungen auswählen.".into()));
            return;
        };
        let config = match self.session_config() {
            Ok(config) => config,
            Err(error) => {
                self.recorder = Some(recorder);
                self.message(Err(error));
                return;
            }
        };
        self.busy = true;
        self.error = false;
        self.status = match action {
            "start" => "Aufnahme wird gestartet…",
            "stop" if self.settings.keep_full_session => "Aufnahme wird gespeichert…",
            "stop" => "Aufnahme wird beendet…",
            _ => "Moment wird gesichert…",
        }
        .into();
        let action = action.to_owned();
        let replay_seconds = if action == "replay_long" && self.settings.replay_seconds_long > 0 {
            self.settings.replay_seconds_long
        } else {
            self.settings.replay_seconds
        };
        let prefix = self.clip_prefix();
        let game = self.active_game.as_ref().map(|g| g.name.clone());
        std::thread::spawn(move || {
            let result = match action.as_str() {
                "start" => recorder.start(config).map(|info| JobOutcome {
                    message: match &game {
                        Some(game) => format!(
                            "{game} läuft – Aufnahme über {}. Drücke deine Taste, um die letzten {replay_seconds} Sekunden zu sichern.",
                            info.strategy
                        ),
                        None => format!(
                            "Aufnahme läuft über {}. Drücke deine Taste, um die letzten {replay_seconds} Sekunden zu sichern.",
                            info.strategy
                        ),
                    },
                    saved: None,
                    session: Some(info),
                    replay: false,
                }),
                "stop" => recorder.stop().map(|saved| JobOutcome {
                    message: match &saved {
                        Some(_) => "Aufnahme beendet und als Video gespeichert.".into(),
                        None => "Aufnahme beendet.".into(),
                    },
                    saved,
                    session: None,
                    replay: false,
                }),
                _ => recorder.save_replay(replay_seconds, &prefix).map(|path| JobOutcome {
                    message: format!("Die letzten {replay_seconds} Sekunden sind gesichert."),
                    saved: Some(path),
                    session: None,
                    replay: true,
                }),
            };
            let result = match (result, recorder.take_warning()) {
                (Ok(mut outcome), Some(warning)) => {
                    outcome.message = format!("{} {warning}", outcome.message);
                    Ok(outcome)
                }
                (other, _) => other,
            };
            let _ = proxy.send_event(AppEvent::Finished(JobResult {
                recorder: Some(recorder),
                restore_recorder: true,
                result,
            }));
        });
    }

    /// Reacts to something the recorder reported by itself: a capture that died
    /// is started again a few times, a nearly full drive ends the recording.
    fn handle_issue(&mut self, issue: recorder::PollIssue, proxy: EventLoopProxy<AppEvent>) {
        self.message(Err(issue.message.clone()));
        self.notify(&issue.message, true, None);
        if issue.low_disk {
            self.banner("Speicherplatz knapp – Aufnahme wird beendet", true);
            if self.recording && !self.busy {
                self.auto_started = false;
                self.recording_job("stop", proxy);
            }
            return;
        }
        self.refresh();
        if !issue.capture_lost || self.busy || self.ffmpeg.is_none() {
            return;
        }
        let now = Instant::now();
        self.restarts.retain(|at| now.duration_since(*at) < RESTART_WINDOW);
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
        let restricted = self.recording
            && (settings.fps != self.settings.fps
                || settings.quality != self.settings.quality
                || settings.encoder != self.settings.encoder
                || settings.display != self.settings.display
                || settings.show_cursor != self.settings.show_cursor
                || settings.system_audio != self.settings.system_audio
                || settings.microphone != self.settings.microphone
                || settings.microphone_device != self.settings.microphone_device
                || settings.system_volume != self.settings.system_volume
                || settings.microphone_volume != self.settings.microphone_volume
                || settings.keep_full_session != self.settings.keep_full_session
                || settings.clips_dir != self.settings.clips_dir);
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
        if action == "ready" {
            return;
        }
        if self.busy {
            if matches!(action, "start" | "stop" | "toggle" | "replay" | "replay_long") {
                self.message(Err("Einen Moment, der letzte Vorgang läuft noch.".into()));
            }
            return;
        }
        match action {
            "refresh" => self.refresh(),
            "rescan" => {
                self.displays = capture::displays();
                self.microphones = audio::capture_devices();
                self.default_output = audio::default_output_name();
                self.refresh();
            }
            "start" | "stop" | "replay" | "replay_long" => self.recording_job(action, proxy),
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
                                "Clip-Ordner geändert. Bisherige Clips bleiben im alten Ordner.".into(),
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
                            self.message(Ok("FFmpeg-Pfad gespeichert. Encoder werden geprüft…".into()));
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
                    let exe = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
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
                    Ok(path) => self.file_job(proxy, "Clip wird umbenannt…", move || {
                        library::rename_clip(&path, &name).map(|path| JobOutcome {
                            message: "Clip umbenannt.".into(),
                            saved: Some(path),
                            session: None,
                            replay: false,
                        })
                    }),
                    Err(error) => self.message(Err(error)),
                }
            }
            "delete" => match self.clip_path(&command) {
                Ok(path) => self.file_job(proxy, "Clip wird in den Papierkorb verschoben…", move || {
                    platform::recycle(&path).map(|_| JobOutcome {
                        message: "Clip in den Papierkorb verschoben.".into(),
                        saved: None,
                        session: None,
                        replay: false,
                    })
                }),
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
            "trim" => match (self.clip_path(&command), self.ffmpeg.clone()) {
                (Ok(path), Some(ffmpeg)) => {
                    let start = command["start"].as_f64().unwrap_or(f64::NAN);
                    let end = command["end"].as_f64().unwrap_or(f64::NAN);
                    let mode = if command["mode"].as_str() == Some("fast") {
                        TrimMode::Fast
                    } else {
                        TrimMode::Precise
                    };
                    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                    let dir = self.settings.clips_dir.clone();
                    self.file_job(proxy, "Dein Clip wird geschnitten…", move || {
                        let duration = recorder::probe_duration(&ffmpeg, &path)?;
                        let Some((start, end)) = mechanics::editor_range(start, end, duration) else {
                            return Err("Der Ausschnitt muss mindestens zwei Sekunden lang sein und innerhalb des Videos liegen. Kürzere Videos können vollständig gespeichert werden.".into());
                        };
                        let output = unique_trim_path(&dir, &stem);
                        recorder::trim_clip(&ffmpeg, &path, &output, start, end, mode).map(|_| JobOutcome {
                            message: "Schnitt als neuer Clip gespeichert. Das Original bleibt erhalten.".into(),
                            saved: Some(output),
                            session: None,
                            replay: false,
                        })
                    });
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
        if result.restore_recorder {
            self.recorder = result.recorder;
        }
        self.busy = false;
        self.recording = self.recorder.as_ref().is_some_and(Recorder::is_recording);
        self.session = self.recorder.as_ref().and_then(|r| r.info().cloned());
        self.elapsed = self.recorder.as_ref().map(|r| r.elapsed().as_secs()).unwrap_or(0);
        match result.result {
            Ok(outcome) => {
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
        if !self.recording {
            self.auto_started = false;
        }
        self.refresh();
        self.update_tray();
        if self.closing {
            if self.recording {
                self.recording_job("stop", proxy);
            }
        }
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

fn unique_trim_path(dir: &Path, stem: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{stem} - Schnitt.mp4"));
    let mut number = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} - Schnitt {number}.mp4"));
        number += 1;
    }
    candidate
}

fn asset(path: &str) -> Option<(&'static [u8], &'static str)> {
    Some(match path {
        "/" | "/index.html" => (include_bytes!("../ui/index.html"), "text/html; charset=utf-8"),
        "/app.css" => (include_bytes!("../ui/app.css"), "text/css; charset=utf-8"),
        "/app.js" => (include_bytes!("../ui/app.js"), "text/javascript; charset=utf-8"),
        _ => return None,
    })
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut screenshot_path = None;
    let mut screenshot_page = String::from("library");
    let mut start_hidden = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--screenshot" {
            screenshot_path = Some(PathBuf::from(
                args.next().ok_or("--screenshot benötigt einen PNG-Dateipfad")?,
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
    eprintln!("Stillclip: creating WebView");
    let mut context = WebContext::new(Some(app.config.join("webview")));
    let media = app.media.clone();
    let view = WebViewBuilder::new_with_web_context(&mut context)
        .with_background_color((17, 18, 23, 255))
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
                    HOTKEY_REPLAY if app.recording && !app.busy => {
                        app.recording_job("replay", proxy.clone());
                    }
                    HOTKEY_REPLAY_LONG if app.recording && !app.busy => {
                        app.recording_job("replay_long", proxy.clone());
                    }
                    HOTKEY_REPLAY | HOTKEY_REPLAY_LONG if !app.recording && !app.busy => {
                        app.notify("Es läuft keine Aufnahme. Starte zuerst die Aufnahme.", true, None);
                    }
                    HOTKEY_RECORD if !app.busy => {
                        let next = if app.recording { "stop" } else { "start" };
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
                    TrayAction::ToggleRecording if !app.busy => {
                        let next = if app.recording { "stop" } else { "start" };
                        app.recording_job(next, proxy.clone());
                    }
                    TrayAction::SaveReplay if app.recording && !app.busy => {
                        app.recording_job("replay", proxy.clone());
                    }
                    TrayAction::Quit => {
                        app.closing = true;
                        if app.busy {
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
                if app.pending_start {
                    app.pending_start = false;
                    app.recording_job("start", proxy.clone());
                }
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::Indexed(key, entry)) => {
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
                if app.settings.auto_record_games && app.ffmpeg.is_some() {
                    if !app.recording && !app.busy {
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
                if app.auto_started && app.recording && !app.busy {
                    app.auto_started = false;
                    app.notify(&format!("{name} beendet – die Aufnahme wird gestoppt."), false, None);
                    app.recording_job("stop", proxy.clone());
                }
                app.update_tray();
                app.sync(&view);
            }
            Event::UserEvent(AppEvent::Finished(result)) => {
                app.finish(result, proxy.clone());
                app.sync(&view);
                if app.closing && !app.recording && !app.busy {
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
                    if app.busy {
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
                if let Some(recorder) = app.recorder.as_mut() {
                    let issue = recorder.poll();
                    let elapsed = recorder.elapsed().as_secs();
                    let recording = recorder.is_recording();
                    let buffer_bytes = if recording { recorder.buffer_bytes() } else { 0 };
                    let changed = elapsed != app.elapsed
                        || recording != app.recording
                        || issue.is_some()
                        || buffer_bytes != app.buffer_bytes;
                    app.elapsed = elapsed;
                    app.buffer_bytes = buffer_bytes;
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
