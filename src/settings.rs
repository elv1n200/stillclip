//! Persistent user settings. Every field has a default so older `settings.json`
//! files keep loading; unknown values fall back instead of failing.
use crate::games::GameEntry;
use crate::mechanics;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Output size and nominal bitrate of a recording.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    /// 1280×720 at 15 Mbit/s: the original desktop default.
    #[default]
    P720,
    /// 1920×1080 at 25 Mbit/s.
    P1080,
    /// 2560×1440 at 40 Mbit/s.
    P1440,
    /// The display's own size; the bitrate grows with the pixel area.
    Native,
}

#[cfg_attr(not(test), allow(dead_code))]
impl Quality {
    pub fn id(self) -> &'static str {
        match self {
            Quality::P720 => "p720",
            Quality::P1080 => "p1080",
            Quality::P1440 => "p1440",
            Quality::Native => "native",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "p720" | "720" | "720p" => Quality::P720,
            "p1080" | "1080" | "1080p" => Quality::P1080,
            "p1440" | "1440" | "1440p" => Quality::P1440,
            "native" | "nativ" => Quality::Native,
            _ => return None,
        })
    }
    /// Configured (width, height, Mbit/s) the capture formulas scale from.
    pub fn configured(self) -> (u32, u32, u32) {
        match self {
            Quality::P720 => (
                crate::encoding::CONFIGURED_WIDTH,
                crate::encoding::CONFIGURED_HEIGHT,
                crate::encoding::BITRATE_MBPS,
            ),
            Quality::P1080 => (1920, 1080, 25),
            Quality::P1440 => (2560, 1440, 40),
            // Native keeps the display size; the area formula raises 1080p's rate.
            Quality::Native => (1920, 1080, 25),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum EncoderChoice {
    /// Prefer NVENC, then AMF, then Quick Sync, then the CPU.
    #[default]
    Auto,
    Nvenc,
    Amf,
    Qsv,
    Cpu,
}

#[cfg_attr(not(test), allow(dead_code))]
impl EncoderChoice {
    pub fn id(self) -> &'static str {
        match self {
            EncoderChoice::Auto => "auto",
            EncoderChoice::Nvenc => "nvenc",
            EncoderChoice::Amf => "amf",
            EncoderChoice::Qsv => "qsv",
            EncoderChoice::Cpu => "cpu",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "auto" => EncoderChoice::Auto,
            "nvenc" => EncoderChoice::Nvenc,
            "amf" => EncoderChoice::Amf,
            "qsv" => EncoderChoice::Qsv,
            "cpu" => EncoderChoice::Cpu,
            _ => return None,
        })
    }
}

/// Highest loudness the UI offers, in percent.
pub const MAX_VOLUME: u32 = 200;

/// Percent to an FFmpeg `volume` factor.
pub fn gain(percent: u32) -> f32 {
    percent.min(MAX_VOLUME) as f32 / 100.0
}

/// A global keyboard shortcut. `key` is a stable name such as `F8`, `K` or `Num5`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: String,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self::none()
    }
}

impl Hotkey {
    pub fn none() -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: String::new(),
        }
    }
    pub fn key(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            ..Self::none()
        }
    }
    pub fn is_set(&self) -> bool {
        virtual_key(&self.key).is_some()
    }
    /// Human readable label such as `Strg + Alt + F8`.
    pub fn label(&self) -> String {
        if !self.is_set() {
            return crate::i18n::text("Nicht belegt", "Not assigned").into();
        }
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push(crate::i18n::text("Strg", "Ctrl"));
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push(crate::i18n::text("Umschalt", "Shift"));
        }
        if self.win {
            parts.push("Win");
        }
        parts.push(self.key.as_str());
        parts.join(" + ")
    }
    pub fn modifiers(&self) -> u32 {
        // MOD_ALT 1, MOD_CONTROL 2, MOD_SHIFT 4, MOD_WIN 8
        (self.alt as u32)
            | ((self.ctrl as u32) << 1)
            | ((self.shift as u32) << 2)
            | ((self.win as u32) << 3)
    }
    pub fn virtual_key(&self) -> Option<u32> {
        virtual_key(&self.key)
    }
    /// Normalizes user supplied values: keys are matched case-insensitively.
    pub fn normalized(mut self) -> Self {
        self.key = canonical_key(&self.key).unwrap_or_default();
        self
    }
}

/// Stable key names accepted from the UI and their Windows virtual-key codes.
pub fn virtual_key(name: &str) -> Option<u32> {
    let canonical = canonical_key(name)?;
    let bytes = canonical.as_bytes();
    if let Some(number) = canonical.strip_prefix('F') {
        if let Ok(n) = number.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(0x70 + n - 1);
            }
        }
    }
    if let Some(digit) = canonical.strip_prefix("Num") {
        if let Ok(n) = digit.parse::<u32>() {
            if n <= 9 {
                return Some(0x60 + n);
            }
        }
    }
    if bytes.len() == 1 {
        let c = bytes[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(u32::from(c));
        }
    }
    Some(match canonical.as_str() {
        "Space" => 0x20,
        "Pause" => 0x13,
        "Insert" => 0x2D,
        "Delete" => 0x2E,
        "Home" => 0x24,
        "End" => 0x23,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "ScrollLock" => 0x91,
        "PrintScreen" => 0x2C,
        "NumMultiply" => 0x6A,
        "NumAdd" => 0x6B,
        "NumSubtract" => 0x6D,
        "NumDecimal" => 0x6E,
        "NumDivide" => 0x6F,
        "Backspace" => 0x08,
        "Tab" => 0x09,
        "Enter" => 0x0D,
        "Escape" => 0x1B,
        "Left" => 0x25,
        "Up" => 0x26,
        "Right" => 0x27,
        "Down" => 0x28,
        "Backquote" => 0xC0,
        "Minus" => 0xBD,
        "Equal" => 0xBB,
        "BracketLeft" => 0xDB,
        "BracketRight" => 0xDD,
        "Backslash" => 0xDC,
        "Semicolon" => 0xBA,
        "Quote" => 0xDE,
        "Comma" => 0xBC,
        "Period" => 0xBE,
        "Slash" => 0xBF,
        _ => return None,
    })
}

fn canonical_key(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    const NAMED: [&str; 34] = [
        "Space",
        "Pause",
        "Insert",
        "Delete",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "ScrollLock",
        "PrintScreen",
        "NumMultiply",
        "NumAdd",
        "NumSubtract",
        "NumDecimal",
        "NumDivide",
        "Backspace",
        "Tab",
        "Enter",
        "Escape",
        "Left",
        "Up",
        "Right",
        "Down",
        "Backquote",
        "Minus",
        "Equal",
        "BracketLeft",
        "BracketRight",
        "Backslash",
        "Semicolon",
        "Quote",
        "Comma",
        "Period",
        "Slash",
    ];
    if let Some(named) = NAMED.iter().find(|n| n.to_ascii_lowercase() == lower) {
        return Some((*named).to_owned());
    }
    if let Some(rest) = lower.strip_prefix('f') {
        if rest.parse::<u32>().is_ok() {
            return Some(format!("F{rest}"));
        }
    }
    if let Some(rest) = lower.strip_prefix("num") {
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() {
            return Some(format!("Num{rest}"));
        }
    }
    if trimmed.len() == 1 && trimmed.is_ascii() {
        let upper = trimmed.to_ascii_uppercase();
        if upper
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Some(upper);
        }
    }
    None
}

/// Deserializes a value but falls back to its default instead of rejecting the
/// whole settings file when one field holds an unknown value.
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct GameProfile {
    pub exe: String,
    #[serde(deserialize_with = "lenient")]
    pub quality: Quality,
    pub fps: u32,
    pub replay_seconds: u32,
    pub microphone: bool,
    pub system_audio: bool,
    pub capture_window: bool,
    pub game_audio: bool,
}

impl Default for GameProfile {
    fn default() -> Self {
        Self {
            exe: String::new(),
            quality: Quality::P1080,
            fps: 60,
            replay_seconds: 30,
            microphone: false,
            system_audio: true,
            capture_window: false,
            game_audio: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    #[serde(deserialize_with = "lenient")]
    pub language: crate::i18n::Language,
    #[serde(deserialize_with = "lenient")]
    pub desktop: crate::desktop::Preferences,
    pub clips_dir: PathBuf,
    pub replay_seconds: u32,
    pub fps: u32,
    #[serde(deserialize_with = "lenient")]
    pub quality: Quality,
    #[serde(deserialize_with = "lenient")]
    pub encoder: EncoderChoice,
    /// Display device name such as `\\.\DISPLAY2`; empty selects the primary display.
    pub display: String,
    pub show_cursor: bool,
    /// Record the display the detected game runs on instead of the fixed choice.
    pub follow_game_display: bool,
    pub system_audio: bool,
    pub microphone: bool,
    /// WASAPI endpoint id; empty uses the default capture device.
    pub microphone_device: String,
    /// Loudness of the system mix in percent; 100 keeps the original level.
    pub system_volume: u32,
    /// Loudness of the microphone in percent; 100 keeps the original level.
    pub microphone_volume: u32,
    /// A second, usually longer clip length; 0 switches it off.
    pub replay_seconds_long: u32,
    /// Seconds a marked moment reaches back.
    pub marker_before: u32,
    /// Seconds a marked moment keeps after the mark.
    pub marker_after: u32,
    /// Keep every segment and save the whole session as one video on stop.
    pub keep_full_session: bool,
    /// Stop recording while less than this many gigabytes are free; 0 switches it off.
    pub min_free_gb: u32,
    /// Delete the oldest clips above this folder size in gigabytes; 0 switches it off.
    pub storage_limit_gb: u32,
    #[serde(deserialize_with = "lenient")]
    pub hotkey_replay: Hotkey,
    #[serde(deserialize_with = "lenient")]
    pub hotkey_replay_long: Hotkey,
    #[serde(deserialize_with = "lenient")]
    pub hotkey_marker: Hotkey,
    #[serde(deserialize_with = "lenient")]
    pub hotkey_record: Hotkey,
    pub close_to_tray: bool,
    pub record_on_launch: bool,
    pub start_minimized: bool,
    /// Keep isolated game and microphone tracks in addition to the playback mix.
    pub microphone_separate_track: bool,
    /// Re-encode replay clips so they are exactly as long as requested.
    pub precise_clips: bool,
    pub clip_sound: bool,
    pub overlay_notifications: bool,
    /// Start recording automatically when a watched game window appears.
    pub auto_record_games: bool,
    pub builtin_games: bool,
    pub custom_games: Vec<GameEntry>,
    #[serde(deserialize_with = "lenient")]
    pub game_profiles: Vec<GameProfile>,
    pub capture_window: bool,
    pub game_audio: bool,
    /// Manual target when no watched game is active. Only an executable name.
    pub target_exe: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: crate::i18n::Language::default(),
            desktop: crate::desktop::Preferences::default(),
            clips_dir: storage_paths().1,
            replay_seconds: mechanics::DEFAULT_REPLAY_SECONDS,
            fps: mechanics::DEFAULT_FPS,
            quality: Quality::P720,
            encoder: EncoderChoice::Auto,
            display: String::new(),
            show_cursor: false,
            follow_game_display: true,
            system_audio: true,
            microphone: false,
            microphone_device: String::new(),
            system_volume: 100,
            microphone_volume: 100,
            replay_seconds_long: 0,
            marker_before: 20,
            marker_after: 10,
            keep_full_session: false,
            min_free_gb: 5,
            storage_limit_gb: 0,
            hotkey_replay: Hotkey::key("F8"),
            hotkey_replay_long: Hotkey::none(),
            hotkey_marker: Hotkey::none(),
            hotkey_record: Hotkey::none(),
            close_to_tray: true,
            record_on_launch: false,
            start_minimized: false,
            microphone_separate_track: true,
            precise_clips: false,
            clip_sound: true,
            overlay_notifications: true,
            auto_record_games: true,
            builtin_games: true,
            custom_games: Vec::new(),
            game_profiles: Vec::new(),
            capture_window: false,
            game_audio: false,
            target_exe: String::new(),
        }
    }
}

impl Settings {
    pub fn for_game(&self, exe: &str) -> Self {
        let mut effective = self.clone();
        let exe = crate::games::normalize_exe(exe);
        if let Some(profile) = self.game_profiles.iter().find(|p| p.exe == exe) {
            effective.quality = profile.quality;
            effective.fps = profile.fps;
            effective.replay_seconds = profile.replay_seconds;
            effective.microphone = profile.microphone;
            effective.system_audio = profile.system_audio;
            effective.capture_window = profile.capture_window;
            effective.game_audio = profile.game_audio;
        }
        effective
    }

    /// Values captured by a running session. Keep UI and backend lock policy in sync.
    pub fn capture_changed(&self, next: &Self) -> bool {
        self.clips_dir != next.clips_dir
            || self.game_profiles != next.game_profiles
            || self.capture_window != next.capture_window
            || self.game_audio != next.game_audio
            || self.target_exe != next.target_exe
            || self.fps != next.fps
            || self.quality != next.quality
            || self.encoder != next.encoder
            || self.display != next.display
            || self.follow_game_display != next.follow_game_display
            || self.show_cursor != next.show_cursor
            || self.system_audio != next.system_audio
            || self.microphone != next.microphone
            || self.microphone_device != next.microphone_device
            || self.microphone_separate_track != next.microphone_separate_track
            || self.system_volume != next.system_volume
            || self.microphone_volume != next.microphone_volume
            || self.keep_full_session != next.keep_full_session
            || self.precise_clips != next.precise_clips
            || self.replay_seconds != next.replay_seconds
            || self.replay_seconds_long != next.replay_seconds_long
            || self.marker_before != next.marker_before
            || self.marker_after != next.marker_after
            || self.min_free_gb != next.min_free_gb
    }

    /// Clamps every numeric value into its supported range.
    pub fn sanitized(mut self) -> Self {
        self.desktop.highlights = self.desktop.highlights.clone().sanitized();
        if self.clips_dir.as_os_str().is_empty() {
            self.clips_dir = Settings::default().clips_dir;
        }
        self.replay_seconds = mechanics::replay_seconds(u64::from(self.replay_seconds));
        if self.replay_seconds_long > 0 {
            self.replay_seconds_long =
                mechanics::replay_seconds(u64::from(self.replay_seconds_long));
        }
        self.fps = mechanics::fps(u64::from(self.fps));
        self.marker_before = self
            .marker_before
            .clamp(1, mechanics::MAX_REPLAY_SECONDS as u32);
        self.marker_after = self.marker_after.min(mechanics::MAX_REPLAY_SECONDS as u32);
        self.system_volume = self.system_volume.min(MAX_VOLUME);
        self.microphone_volume = self.microphone_volume.min(MAX_VOLUME);
        self.min_free_gb = self.min_free_gb.min(1024);
        self.storage_limit_gb = self.storage_limit_gb.min(100_000);
        self.hotkey_replay = self.hotkey_replay.normalized();
        self.hotkey_replay_long = self.hotkey_replay_long.normalized();
        self.hotkey_marker = self.hotkey_marker.normalized();
        self.hotkey_record = self.hotkey_record.normalized();
        // Every shortcut must stay unique; later ones give way to earlier ones.
        if self.hotkey_replay.is_set() && self.hotkey_replay == self.hotkey_replay_long {
            self.hotkey_replay_long = Hotkey::none();
        }
        if self.hotkey_marker.is_set()
            && (self.hotkey_marker == self.hotkey_replay
                || self.hotkey_marker == self.hotkey_replay_long)
        {
            self.hotkey_marker = Hotkey::none();
        }
        if self.hotkey_record.is_set()
            && (self.hotkey_record == self.hotkey_replay
                || self.hotkey_record == self.hotkey_replay_long
                || self.hotkey_record == self.hotkey_marker)
        {
            self.hotkey_record = Hotkey::none();
        }
        let mut games: Vec<GameEntry> = Vec::new();
        for game in std::mem::take(&mut self.custom_games) {
            let game = game.normalized();
            if game.is_valid() && !games.iter().any(|g| g.exe == game.exe) {
                games.push(game);
            }
        }
        self.custom_games = games;
        let mut seen = std::collections::HashSet::new();
        self.game_profiles.retain_mut(|profile| {
            profile.exe = crate::games::normalize_exe(&profile.exe);
            profile.fps = mechanics::fps(u64::from(profile.fps));
            profile.replay_seconds = mechanics::replay_seconds(u64::from(profile.replay_seconds));
            !profile.exe.is_empty() && seen.insert(profile.exe.clone())
        });
        self.target_exe = crate::games::normalize_exe(&self.target_exe);
        self
    }

    /// Seconds the segment ring has to hold: the longest clip length in use,
    /// marked moments included, so a mark can still be saved afterwards.
    pub fn buffer_seconds(&self) -> u32 {
        self.replay_seconds
            .max(self.replay_seconds_long)
            .max(self.marker_before + self.marker_after)
            .max(if self.desktop.highlights.enabled { crate::valorant::RETAIN_SECONDS } else { 0 })
    }

    /// Free space the recorder insists on, in bytes.
    pub fn min_free_bytes(&self) -> u64 {
        u64::from(self.min_free_gb) * 1024 * 1024 * 1024
    }

    /// Size the clip folder may reach before old clips go, in bytes.
    pub fn storage_limit_bytes(&self) -> u64 {
        u64::from(self.storage_limit_gb) * 1024 * 1024 * 1024
    }

    /// Every executable the watcher should react to.
    pub fn watched_games(&self) -> Vec<GameEntry> {
        if !self.auto_record_games {
            return Vec::new();
        }
        let mut games = self.custom_games.clone();
        if self.builtin_games {
            for game in crate::games::builtin_games() {
                if !games.iter().any(|g| g.exe == game.exe) {
                    games.push(game);
                }
            }
        }
        games
    }
}

pub fn application_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_config_dir() -> PathBuf {
    storage_paths().0
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("STILLCLIP_DATA_DIR").map(PathBuf::from).unwrap_or_else(default_config_dir)
}

fn storage_paths_for(app: &Path, installed: bool, local: Option<PathBuf>, videos: Option<PathBuf>) -> (PathBuf, PathBuf) {
    if installed {
        if let Some(local) = local {
            let data = local.join("Stillclip");
            return (data.join("settings"), videos.map(|dir| dir.join("Stillclip")).unwrap_or_else(|| data.join("clips")));
        }
    }
    (app.join("settings"), app.join("clips"))
}

fn storage_paths() -> (PathBuf, PathBuf) {
    let app = application_dir();
    let installed = app.join("installed.flag").is_file();
    storage_paths_for(&app, installed,
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        if installed { crate::platform::videos_dir() } else { None })
}

/// Editors such as Notepad write a UTF-8 byte order mark; serde rejects it,
/// which would silently reset every setting.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    let text = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    serde_json::from_slice(text).ok()
}

pub fn load_settings(config_dir: &Path) -> Settings {
    let mut settings: Settings = read_json(&config_dir.join("settings.json")).unwrap_or_else(|| {
        let mut settings = Settings::default();
        if !config_dir.join("settings.json").exists() {
            if let Ok(language) = fs::read_to_string(application_dir().join("initial-language.txt")) {
                settings.language = crate::i18n::Language::parse(language.trim()).unwrap_or_default();
            }
        }
        settings
    });
    // Existing installations do not get a forced onboarding screen.
    if read_json::<serde_json::Value>(&config_dir.join("settings.json")).is_some_and(|v| v.get("desktop").is_none()) {
        settings.desktop.welcome_complete = true;
    }
    settings.sanitized()
}

pub fn save_settings(config_dir: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(config_dir)
        .map_err(|error| crate::tr_format!("Einstellungsordner konnte nicht erstellt werden: {error}", "Could not create settings folder: {error}"))?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| crate::tr_format!("Einstellungen konnten nicht gespeichert werden: {error}", "Could not save settings: {error}"))?;
    write_atomically(config_dir, "settings.json", &bytes)
        .map_err(|error| crate::tr_format!("Einstellungen konnten nicht gespeichert werden: {error}", "Could not save settings: {error}"))
}

/// Writes to a temporary file first so a crash never leaves a half-written file.
pub fn write_atomically(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = dir.join(format!(".{name}-{}-{nonce}.tmp", std::process::id()));
    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, dir.join(name))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_migration_and_desktop_preferences_preserve_existing_capture_settings() {
        let root = std::env::temp_dir().join(format!("stillclip-setup-settings-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let fresh = load_settings(&root); assert!(!fresh.desktop.welcome_complete);
        fs::write(root.join("settings.json"), br#"{"language":"en","fps":120,"quality":"p1080"}"#).unwrap();
        let existing = load_settings(&root);
        assert!(existing.desktop.welcome_complete); assert_eq!(existing.fps, 120); assert_eq!(existing.quality, Quality::P1080);
        let mut changed = existing.clone(); changed.desktop.hud_enabled = !existing.desktop.hud_enabled;
        changed.desktop.player_name = "Elvin".into(); changed.desktop.automatic_updates = true;
        assert!(!existing.capture_changed(&changed));
        save_settings(&root, &changed).unwrap(); assert_eq!(load_settings(&root), changed);
        fs::remove_dir_all(root).unwrap();
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("stillclip-settings-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn automatic_highlights_are_opt_in_persistent_and_keep_enough_video() {
        let old: Settings = serde_json::from_str(r#"{"desktop":{"player_name":"Elvin"}}"#).unwrap();
        assert!(!old.desktop.highlights.enabled);
        assert_eq!(old.buffer_seconds(),30);
        let directory = TestDir::new();
        let mut settings = old;
        settings.desktop.highlights.enabled = true;
        settings.desktop.highlights.min_kills = 3;
        assert_eq!(settings.buffer_seconds(),crate::valorant::RETAIN_SECONDS);
        save_settings(&directory.0,&settings).unwrap();
        assert_eq!(load_settings(&directory.0),settings);
        settings.replay_seconds = 300;
        assert_eq!(settings.buffer_seconds(),300);
        settings.desktop.player_name.clear();
        assert!(settings.desktop.clone().validate().is_err());
        settings.desktop.highlights.enabled = false;
        assert!(settings.desktop.validate().is_ok());
    }

    #[test]
    fn language_persists_and_old_or_unknown_values_keep_german_defaults() {
        use crate::i18n::Language;
        let directory = TestDir::new();
        let mut settings = Settings::default();
        let before = settings.clone();
        settings.language = Language::En;
        assert!(!before.capture_changed(&settings));
        save_settings(&directory.0, &settings).unwrap();
        assert_eq!(load_settings(&directory.0), settings);
        for json in [r#"{}"#, r#"{"language":"fr","fps":30}"#, r#"{"language":12,"fps":30}"#] {
            fs::write(directory.0.join("settings.json"), json).unwrap();
            let loaded = load_settings(&directory.0);
            assert_eq!(loaded.language, Language::De);
            if json != "{}" { assert_eq!(loaded.fps, 30); }
        }
    }

    #[test]
    fn installed_data_is_separate_and_portable_paths_stay_compatible() {
        let app = Path::new("program");
        let local = PathBuf::from("user-data");
        let videos = PathBuf::from("redirected-videos");
        assert_eq!(storage_paths_for(app, false, Some(local.clone()), Some(videos.clone())),
            (app.join("settings"), app.join("clips")));
        assert_eq!(storage_paths_for(app, true, Some(local.clone()), Some(videos)),
            (local.join("Stillclip/settings"), PathBuf::from("redirected-videos/Stillclip")));
        assert_eq!(storage_paths_for(app, true, Some(local.clone()), None),
            (local.join("Stillclip/settings"), local.join("Stillclip/clips")));
    }

    #[test]
    fn settings_survive_repeated_saves_and_partial_files() {
        let directory = TestDir::new();
        let defaults = load_settings(&directory.0);
        assert_eq!(defaults.replay_seconds, 30);
        assert_eq!(defaults.fps, 60);
        assert_eq!(defaults.hotkey_replay.key, "F8");
        let mut settings = Settings::default();
        settings.clips_dir = directory.0.join("videos");
        settings.quality = Quality::P1080;
        settings.encoder = EncoderChoice::Cpu;
        settings.microphone = true;
        save_settings(&directory.0, &settings).unwrap();
        for seconds in [15, 90, 300, 457, 1200] {
            settings.replay_seconds = seconds;
            settings.fps = 24;
            save_settings(&directory.0, &settings).unwrap();
            let loaded = load_settings(&directory.0);
            assert_eq!(loaded, settings);
        }
        // Files written by the previous version only know a few fields.
        fs::write(
            directory.0.join("settings.json"),
            br#"{"clips_dir":"C:\\clips","fps":0,"replay_seconds":99999}"#,
        )
        .unwrap();
        let loaded = load_settings(&directory.0);
        assert_eq!(loaded.fps, 15);
        assert_eq!(loaded.replay_seconds, 1200);
        assert_eq!(loaded.clips_dir, PathBuf::from("C:\\clips"));
        assert_eq!(loaded.quality, Quality::P720);
        assert!(loaded.system_audio);
        fs::write(
            directory.0.join("settings.json"),
            br#"{"replay_seconds":0,"quality":"bogus"}"#,
        )
        .unwrap();
        let loaded = load_settings(&directory.0);
        assert_eq!(loaded.replay_seconds, 15);
        assert_eq!(loaded.fps, 60);
    }

    #[test]
    fn profiles_are_normalized_persistent_and_do_not_change_global_defaults() {
        let mut settings = Settings::default();
        settings.game_profiles.push(GameProfile {
            exe: "VALORANT-Win64-Shipping.EXE".into(),
            quality: Quality::P1080,
            fps: 120,
            replay_seconds: 60,
            microphone: true,
            capture_window: true,
            game_audio: true,
            system_audio: true,
        });
        let settings = settings.sanitized();
        let effective = settings.for_game("valorant-win64-shipping.exe");
        assert_eq!((effective.fps, effective.replay_seconds), (120, 60));
        assert!(effective.capture_window && effective.game_audio && effective.microphone);
        assert_eq!(settings.quality, Quality::P720);
        assert_eq!(settings.replay_seconds, 30);
        assert_eq!(settings.for_game("missing.exe"), settings);
        let dir = TestDir::new();
        save_settings(&dir.0, &settings).unwrap();
        assert_eq!(load_settings(&dir.0), settings);
        let mut changed = settings.clone();
        changed.game_profiles[0].fps = 60;
        assert!(settings.capture_changed(&changed));
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert!(old.game_profiles.is_empty());
    }

    #[test]
    fn settings_with_a_byte_order_mark_still_load() {
        let directory = TestDir::new();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"fps":30,"replay_seconds":45}"#);
        fs::write(directory.0.join("settings.json"), bytes).unwrap();
        let loaded = load_settings(&directory.0);
        assert_eq!(loaded.fps, 30);
        assert_eq!(loaded.replay_seconds, 45);
    }

    #[test]
    fn hotkeys_map_names_to_virtual_keys() {
        assert_eq!(Hotkey::key("F8").virtual_key(), Some(0x77));
        assert_eq!(Hotkey::key("f12").normalized().key, "F12");
        assert_eq!(Hotkey::key("k").virtual_key(), Some(0x4B));
        assert_eq!(Hotkey::key("Num5").virtual_key(), Some(0x65));
        assert_eq!(Hotkey::key("pageup").normalized().key, "PageUp");
        assert_eq!(Hotkey::key("F25").virtual_key(), None);
        assert_eq!(Hotkey::key("ö").virtual_key(), None);
        assert!(!Hotkey::none().is_set());
        let combo = Hotkey {
            ctrl: true,
            alt: true,
            shift: false,
            win: false,
            key: "F8".into(),
        };
        assert_eq!(combo.modifiers(), 3);
        assert_eq!(combo.label(), "Strg + Alt + F8");
        let mut settings = Settings::default();
        settings.hotkey_record = Hotkey::key("F8");
        assert!(!settings.sanitized().hotkey_record.is_set());
    }

    #[test]
    fn running_capture_locks_every_session_setting_but_not_shortcuts() {
        let original = Settings::default();
        for field in [
            "replay_seconds",
            "replay_seconds_long",
            "marker_before",
            "marker_after",
            "min_free_gb",
        ] {
            let mut value = serde_json::to_value(&original).unwrap();
            value[field] = serde_json::json!(600);
            let changed: Settings = serde_json::from_value(value).unwrap();
            assert!(original.capture_changed(&changed), "{field}");
        }
        for field in [
            "precise_clips",
            "microphone_separate_track",
            "follow_game_display",
        ] {
            let mut value = serde_json::to_value(&original).unwrap();
            value[field] = serde_json::json!(!value[field].as_bool().unwrap());
            let changed: Settings = serde_json::from_value(value).unwrap();
            assert!(original.capture_changed(&changed), "{field}");
        }
        let mut changed = original.clone();
        changed.hotkey_replay = Hotkey::key("F9");
        changed.storage_limit_gb = 50;
        assert!(!original.capture_changed(&changed));
    }

    #[test]
    fn loudness_lengths_and_limits_are_clamped() {
        let defaults = Settings::default();
        assert_eq!(defaults.system_volume, 100);
        assert_eq!(defaults.microphone_volume, 100);
        assert_eq!(defaults.replay_seconds_long, 0);
        assert_eq!(defaults.min_free_gb, 5);
        assert_eq!(defaults.storage_limit_gb, 0);
        assert!(defaults.follow_game_display);
        assert_eq!(gain(100), 1.0);
        assert_eq!(gain(0), 0.0);
        assert_eq!(gain(900), 2.0);

        let mut settings = Settings::default();
        settings.system_volume = 5_000;
        settings.microphone_volume = 150;
        settings.replay_seconds_long = 99_999;
        settings.min_free_gb = 9_999;
        let settings = settings.sanitized();
        assert_eq!(settings.system_volume, MAX_VOLUME);
        assert_eq!(settings.microphone_volume, 150);
        assert_eq!(settings.replay_seconds_long, 1200);
        assert_eq!(settings.min_free_gb, 1024);
        // The ring has to hold the longer of the two clip lengths.
        assert_eq!(settings.buffer_seconds(), 1200);
        assert_eq!(settings.min_free_bytes(), 1024 * 1024 * 1024 * 1024);

        // A length of zero switches the second clip off and stays off.
        let mut off = Settings::default();
        off.replay_seconds_long = 0;
        assert_eq!(off.sanitized().replay_seconds_long, 0);
        assert_eq!(Settings::default().buffer_seconds(), 30);

        // No shortcut may be claimed twice.
        let mut clashing = Settings::default();
        clashing.hotkey_replay_long = Hotkey::key("F8");
        clashing.hotkey_record = Hotkey::key("F8");
        let clashing = clashing.sanitized();
        assert_eq!(clashing.hotkey_replay.key, "F8");
        assert!(!clashing.hotkey_replay_long.is_set());
        assert!(!clashing.hotkey_record.is_set());
    }

    #[test]
    fn quality_presets_round_trip() {
        for quality in [
            Quality::P720,
            Quality::P1080,
            Quality::P1440,
            Quality::Native,
        ] {
            assert_eq!(Quality::parse(quality.id()), Some(quality));
        }
        for encoder in [
            EncoderChoice::Auto,
            EncoderChoice::Nvenc,
            EncoderChoice::Amf,
            EncoderChoice::Qsv,
            EncoderChoice::Cpu,
        ] {
            assert_eq!(EncoderChoice::parse(encoder.id()), Some(encoder));
        }
        assert_eq!(Quality::parse("4k"), None);
    }
}
