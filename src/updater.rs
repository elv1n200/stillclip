//! Explicit, bounded downloads from this repository's published releases.
//! Hashes establish consistency with GitHub's release metadata; they are not
//! a substitute for a Windows publisher signature.
use crate::{i18n::text, jobs::Control};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const REPOSITORY: &str = "https://github.com/elv1n200/stillclip";
const API: &str = "https://api.github.com/repos/elv1n200/stillclip/releases/latest";
const MAX_DOWNLOAD: u64 = 600 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub name: String,
    pub download: String,
    pub sha256: String,
    pub bytes: u64,
    pub portable: bool,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(30))
        .https_only(true)
        .build()
}

fn limited_json(mut input: impl Read) -> Result<serde_json::Value, String> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Release metadata is too large.".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub fn parse_release(
    value: &serde_json::Value,
    current: &str,
    portable: bool,
) -> Result<Option<Release>, String> {
    let tag = value["tag_name"]
        .as_str()
        .ok_or("Missing release version.")?;
    let version =
        Version::parse(tag.strip_prefix('v').unwrap_or(tag)).map_err(|e| e.to_string())?;
    let installed = Version::parse(current).map_err(|e| e.to_string())?;
    if value["draft"] != false
        || value["prerelease"] != false
        || !version.pre.is_empty()
        || version <= installed
    {
        return Ok(None);
    }
    // Restrict every URL before using it, including the version's URL path.
    if tag != format!("v{version}") {
        return Err("Unexpected release tag.".into());
    }
    let url = format!("{REPOSITORY}/releases/tag/{tag}");
    if value["html_url"] != url {
        return Err("Unexpected release repository.".into());
    }
    let name = if portable {
        "stillclip-windows-x64.zip"
    } else {
        "stillclip-setup-online-x64.exe"
    };
    let download = format!("{REPOSITORY}/releases/download/{tag}/{name}");
    let asset = value["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|a| a["name"] == name))
        .ok_or_else(|| {
            text(
                "Das passende Update-Paket fehlt.",
                "The matching update package is missing.",
            )
            .to_owned()
        })?;
    if asset["browser_download_url"] != download {
        return Err("Unexpected release asset URL.".into());
    }
    let digest = asset["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| {
            text(
                "Das Update hat keine gültige SHA-256-Prüfsumme.",
                "The update has no valid SHA-256 checksum.",
            )
            .to_owned()
        })?;
    let bytes = asset["size"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= MAX_DOWNLOAD)
        .ok_or("Invalid asset size.")?;
    Ok(Some(Release {
        version: version.to_string(),
        notes: value["body"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(30000)
            .collect(),
        url,
        name: name.into(),
        download,
        sha256: digest.to_ascii_lowercase(),
        bytes,
        portable,
    }))
}

pub fn check(portable: bool) -> Result<Option<Release>, String> {
    let response = agent()
        .get(API)
        .set(
            "User-Agent",
            concat!("Stillclip/", env!("CARGO_PKG_VERSION")),
        )
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    parse_release(
        &limited_json(response.into_reader())?,
        env!("CARGO_PKG_VERSION"),
        portable,
    )
}

pub fn download(release: &Release, root: &Path, control: &Control) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let final_path = root.join(format!("{}-{}", release.version, release.name));
    let temporary = final_path.with_extension("download");
    let result = (|| {
        let response = agent()
            .get(&release.download)
            .set("User-Agent", "Stillclip updater")
            .call()
            .map_err(|e| e.to_string())?;
        let mut input = response.into_reader();
        let mut output = fs::File::create(&temporary).map_err(|e| e.to_string())?;
        let mut digest = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            control.check()?;
            let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > release.bytes {
                return Err("Update exceeds its published size.".into());
            }
            output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            digest.update(&buffer[..n]);
            control.progress(
                total as f64 / release.bytes as f64,
                text("Update herunterladen", "Downloading update"),
            );
        }
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        control.check()?;
        if total != release.bytes || format!("{:x}", digest.finalize()) != release.sha256 {
            return Err(text(
                "Die Update-Prüfsumme stimmt nicht. Die Datei wurde verworfen.",
                "Update checksum mismatch. The file was discarded.",
            )
            .into());
        }
        if final_path.exists() {
            fs::remove_file(&final_path).map_err(|e| e.to_string())?;
        }
        fs::rename(&temporary, &final_path).map_err(|e| e.to_string())?;
        Ok(final_path)
    })();
    let _ = fs::remove_file(temporary);
    result
}

/// Recheck at use time, so a downloaded file cannot be replaced unnoticed.
pub fn verify_file(path: &Path, release: &Release) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() != release.bytes {
        return Err("Update size mismatch.".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if format!("{:x}", hash.finalize()) != release.sha256 {
        return Err("Update checksum mismatch.".into());
    }
    Ok(())
}

pub fn verify_publisher(path: &Path) -> Result<(), String> {
    // A signed installation never accepts an unsigned update or another
    // publisher. Unsigned releases can upgrade through the verified GitHub
    // channel; an invalid/tampered Authenticode signature is always rejected.
    let script = r#"$ErrorActionPreference='Stop';
      $current=Get-AuthenticodeSignature -LiteralPath $env:STILLCLIP_CURRENT_EXE;
      $next=Get-AuthenticodeSignature -LiteralPath $env:STILLCLIP_UPDATE_FILE;
      if ($next.Status -notin @('Valid','NotSigned')) { exit 10 };
      if ($current.Status -eq 'Valid' -and ($next.Status -ne 'Valid' -or $current.SignerCertificate.Subject -ne $next.SignerCertificate.Subject)) { exit 11 };
      if ($current.Status -notin @('Valid','NotSigned')) { exit 12 }"#;
    let output = crate::capture::command(Path::new("powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env(
            "STILLCLIP_CURRENT_EXE",
            std::env::current_exe().map_err(|e| e.to_string())?,
        )
        .env("STILLCLIP_UPDATE_FILE", path)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(text(
            "Die Windows-Herausgeberprüfung ist fehlgeschlagen. Das Update wird nicht gestartet.",
            "Windows publisher verification failed. The update will not be launched.",
        )
        .into());
    }
    Ok(())
}

pub fn launch_installer(path: &Path) -> Result<(), String> {
    // Constant script; paths travel as environment values, never shell code.
    let script = r#"$ErrorActionPreference='Stop';
      Wait-Process -Id ([int]$env:STILLCLIP_PARENT_PID) -Timeout 45 -ErrorAction SilentlyContinue;
      if (Get-Process -Id ([int]$env:STILLCLIP_PARENT_PID) -ErrorAction SilentlyContinue) { exit 1 };
      Start-Process -FilePath $env:STILLCLIP_UPDATE_FILE"#;
    crate::capture::command(Path::new("powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("STILLCLIP_PARENT_PID", std::process::id().to_string())
        .env("STILLCLIP_UPDATE_FILE", path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn release() -> serde_json::Value {
        json!({"tag_name":"v0.9.0","draft":false,"prerelease":false,
            "html_url":format!("{REPOSITORY}/releases/tag/v0.9.0"),"body":"release notes",
            "assets":[{"name":"stillclip-setup-online-x64.exe","size":4000,"digest":format!("sha256:{}", "a".repeat(64)),
            "browser_download_url":format!("{REPOSITORY}/releases/download/v0.9.0/stillclip-setup-online-x64.exe")}]})
    }
    #[test]
    fn updater_rejects_foreign_unhashed_prerelease_and_downgrade_assets() {
        let value = release();
        assert!(parse_release(&value, "0.8.0", false).unwrap().is_some());
        assert!(parse_release(&value, "1.0.0", false).unwrap().is_none());
        assert!(parse_release(&value, "0.8.0", true).is_err());
        let mut bad = value.clone();
        bad["assets"][0]["browser_download_url"] = json!("https://example.com/malware.exe");
        assert!(parse_release(&bad, "0.8.0", false).is_err());
        let mut bad = value.clone();
        bad["assets"][0]["digest"] = serde_json::Value::Null;
        assert!(parse_release(&bad, "0.8.0", false).is_err());
        let mut bad = value;
        bad["prerelease"] = json!(true);
        assert!(parse_release(&bad, "0.8.0", false).unwrap().is_none());
    }

    #[test]
    fn update_file_is_rechecked_for_same_size_tampering() {
        let mut info = parse_release(&release(), "0.8.0", false).unwrap().unwrap();
        info.bytes = 3;
        info.sha256 = format!("{:x}", Sha256::digest(b"abc"));
        let root =
            std::env::temp_dir().join(format!("stillclip-update-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("update.exe");
        fs::write(&file, b"abc").unwrap();
        assert!(verify_file(&file, &info).is_ok());
        fs::write(&file, b"abd").unwrap();
        assert!(verify_file(&file, &info).is_err());
        fs::write(&file, b"ab").unwrap();
        assert!(verify_file(&file, &info).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
