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
            return "Nicht belegt".into();
        }
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Strg");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Umschalt");
        }
        if self.win {
            parts.push("Win");
        }
        parts.push(self.key.as_str());
        parts.join(" + ")
    }
    pub fn modifiers(&self) -> u32 {
        // MOD_ALT 1, MOD_CONTROL 2, MOD_SHIFT 4, MOD_WIN 8
        (self.alt as u32) | ((self.ctrl as u32) << 1) | ((self.shift as u32) << 2) | ((self.win as u32) << 3)
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
        if upper.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
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
pub struct Settings {
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
    pub system_audio: bool,
    pub microphone: bool,
    /// WASAPI endpoint id; empty uses the default capture device.
    pub microphone_device: String,
    /// Keep every segment and save the whole session as one video on stop.
    pub keep_full_session: bool,
    #[serde(deserialize_with = "lenient")]
    pub hotkey_replay: Hotkey,
    #[serde(deserialize_with = "lenient")]
    pub hotkey_record: Hotkey,
    pub close_to_tray: bool,
    pub record_on_launch: bool,
    pub start_minimized: bool,
    /// Keep the microphone as a second audio track in addition to the mix.
    pub microphone_separate_track: bool,
    /// Re-encode replay clips so they are exactly as long as requested.
    pub precise_clips: bool,
    pub clip_sound: bool,
    pub overlay_notifications: bool,
    /// Start recording automatically when a watched game window appears.
    pub auto_record_games: bool,
    pub builtin_games: bool,
    pub custom_games: Vec<GameEntry>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            clips_dir: application_dir().join("clips"),
            replay_seconds: mechanics::DEFAULT_REPLAY_SECONDS,
            fps: mechanics::DEFAULT_FPS,
            quality: Quality::P720,
            encoder: EncoderChoice::Auto,
            display: String::new(),
            show_cursor: false,
            system_audio: true,
            microphone: false,
            microphone_device: String::new(),
            keep_full_session: false,
            hotkey_replay: Hotkey::key("F8"),
            hotkey_record: Hotkey::none(),
            close_to_tray: true,
            record_on_launch: false,
            start_minimized: false,
            microphone_separate_track: false,
            precise_clips: false,
            clip_sound: true,
            overlay_notifications: true,
            auto_record_games: true,
            builtin_games: true,
            custom_games: Vec::new(),
        }
    }
}

impl Settings {
    /// Clamps every numeric value into its supported range.
    pub fn sanitized(mut self) -> Self {
        if self.clips_dir.as_os_str().is_empty() {
            self.clips_dir = Settings::default().clips_dir;
        }
        self.replay_seconds = mechanics::replay_seconds(u64::from(self.replay_seconds));
        self.fps = mechanics::fps(u64::from(self.fps));
        self.hotkey_replay = self.hotkey_replay.normalized();
        self.hotkey_record = self.hotkey_record.normalized();
        if self.hotkey_replay.is_set() && self.hotkey_replay == self.hotkey_record {
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
        self
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
    application_dir().join("settings")
}

pub fn load_settings(config_dir: &Path) -> Settings {
    let settings: Settings = fs::read(config_dir.join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    settings.sanitized()
}

pub fn save_settings(config_dir: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(config_dir)
        .map_err(|error| format!("Einstellungsordner konnte nicht erstellt werden: {error}"))?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Einstellungen konnten nicht gespeichert werden: {error}"))?;
    write_atomically(config_dir, "settings.json", &bytes)
        .map_err(|error| format!("Einstellungen konnten nicht gespeichert werden: {error}"))
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
    fn quality_presets_round_trip() {
        for quality in [Quality::P720, Quality::P1080, Quality::P1440, Quality::Native] {
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
