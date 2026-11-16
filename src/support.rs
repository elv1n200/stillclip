//! Reports are assembled from an allowlist. No raw settings, recordings,
//! process list, window titles, device IDs or account tokens are exported.
use serde_json::{json, Value};
use std::path::Path;

pub fn redact(message: &str, private: &[String]) -> String {
    let mut text: String = message.chars().take(2400).collect();
    let mut values = private.iter().filter(|p| p.len() >= 3).collect::<Vec<_>>();
    values.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for value in values { text = text.replace(value, "[private]"); }
    // Paths can contain spaces and quotes. Removing through the end of the
    // line deliberately favours privacy over retaining an error's suffix.
    let paths = regex::Regex::new(r#"(?i)(?:[a-z]:[\\/]|\\\\|https?://|(?:^|[\s=\"'])/)[^\r\n]*"#).unwrap();
    paths.replace_all(&text, " [redacted]").into_owned()
}

pub fn create(app: &crate::App) -> Value {
    let mut private = ["USERNAME", "USERPROFILE", "LOCALAPPDATA", "APPDATA", "COMPUTERNAME"].iter()
        .filter_map(|name| std::env::var(name).ok()).collect::<Vec<_>>();
    private.extend([app.config.to_string_lossy().into_owned(), app.settings.clips_dir.to_string_lossy().into_owned(),
        app.settings.desktop.player_name.clone()]);
    json!({
        "schema": 1,
        "created_utc": chrono::Utc::now().to_rfc3339(),
        "application": {"name":"Stillclip", "version":env!("CARGO_PKG_VERSION"), "architecture":std::env::consts::ARCH,
            "installed": crate::settings::application_dir().join("installed.flag").is_file()},
        "system": {"os":std::env::consts::OS, "windows_build": crate::platform::windows_build()},
        "media": {"ffmpeg_available":app.ffmpeg.is_some(), "capabilities_probed": app.capabilities.probed,
            "encoders":app.capabilities.encoders.iter().map(|e| e.id()).collect::<Vec<_>>(),
            "desktop_capture":app.capabilities.ddagrab, "window_capture":app.capabilities.gfxcapture},
        "capture": {"recording":app.recording, "quality":app.runtime_settings().quality, "fps":app.runtime_settings().fps,
            "microphone":app.runtime_settings().microphone, "system_audio":app.runtime_settings().system_audio,
            "game_audio":app.runtime_settings().game_audio, "window_capture":app.runtime_settings().capture_window,
            "separate_audio":app.settings.microphone_separate_track,
            "performance":app.recorder.as_ref().and_then(|r| r.diagnostics())},
        "displays": app.displays.iter().map(|d| json!({"width":d.width,"height":d.height,"primary":d.primary})).collect::<Vec<_>>(),
        "storage": {"free_bytes":app.free_bytes,"clips_bytes":app.clips_bytes,"minimum_free_gb":app.settings.min_free_gb},
        "recent_errors": app.desktop.errors.iter().map(|e| redact(e, &private)).collect::<Vec<_>>(),
        "privacy": "Paths, local account identifiers and URLs in errors are redacted. Review this report before sharing. No files are uploaded."
    })
}

pub fn save(report: &Value, directory: &Path) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    let name = format!("stillclip-report-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f"));
    let path = directory.join(name);
    let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(&path).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?;
    file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_hides_paths_with_spaces_unc_urls_and_account_names() {
        let private = vec!["Jane Doe".to_owned(), "gamer#123".to_owned()];
        for raw in [r#"Could not open C:\Users\Jane Doe\Videos\Secret Match.mp4: denied"#,
            r#"Input \\NAS\private share\Secret Match.mp4"#, "decode /home/jane/Secret Match.mp4 failed",
            "request https://example.com/?token=secret", "Jane Doe gamer#123"] {
            let safe = redact(raw, &private);
            for secret in ["Jane Doe", "gamer#123", "Secret Match", "private share", "token=secret"] { assert!(!safe.contains(secret), "{safe}"); }
        }
        assert_eq!(redact("H.264 encoder returned error -22", &[]), "H.264 encoder returned error -22");
    }
}
