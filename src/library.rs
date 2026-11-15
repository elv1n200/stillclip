//! The clip library: scanning, importing, renaming, deleting and the cached
//! media index (durations, sizes, poster frames).
use crate::recorder::MediaInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct Clip {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
}

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mp4" | "mkv" | "mov" | "webm" | "avi" | "m4v"
            )
        })
}

pub fn scan_clips(dir: &Path) -> Result<Vec<Clip>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Clip-Ordner konnte nicht gelesen werden: {error}")),
    };
    let mut clips = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("Clip konnte nicht gelesen werden: {error}"))?;
        let path = entry.path();
        if !is_video(&path) {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => metadata,
            // A recorder or the user may move a file while the list refreshes.
            Ok(_) | Err(_) => continue,
        };
        clips.push(Clip {
            name: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path,
            bytes: metadata.len(),
            created: metadata.created().or_else(|_| metadata.modified()).ok(),
            modified: metadata.modified().ok(),
        });
    }
    clips.sort_by(|left, right| {
        right
            .created
            .cmp(&left.created)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(clips)
}

/// The oldest clips that have to go so the folder fits into `limit_bytes`.
/// A limit of zero switches the cap off, and the newest clip is never on the
/// list – a limit smaller than a single clip must not empty the folder.
pub fn over_limit(clips: &[Clip], limit_bytes: u64) -> Vec<Clip> {
    if limit_bytes == 0 || clips.len() < 2 {
        return Vec::new();
    }
    let mut total: u64 = clips.iter().map(|clip| clip.bytes).sum();
    if total <= limit_bytes {
        return Vec::new();
    }
    let mut oldest: Vec<&Clip> = clips.iter().collect();
    oldest.sort_by_key(|clip| clip.created.or(clip.modified).unwrap_or(UNIX_EPOCH));
    oldest.pop();
    let mut doomed = Vec::new();
    for clip in oldest {
        if total <= limit_bytes {
            break;
        }
        total = total.saturating_sub(clip.bytes);
        doomed.push(clip.clone());
    }
    doomed
}

// Reserve the destination before copying: fs::copy alone can overwrite a clip.
fn copy_new(source: &Path, destination: &Path) -> io::Result<()> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let result = io::copy(&mut input, &mut output).and_then(|_| output.sync_all());
    drop(output);
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}

pub fn import_clip(source: &Path, dir: &Path) -> Result<PathBuf, String> {
    if !source.is_file() || !is_video(source) {
        return Err(
            "Bitte eine Videodatei im Format MP4, MKV, MOV, WebM, AVI oder M4V wählen.".into(),
        );
    }
    let filename = source
        .file_name()
        .ok_or_else(|| "Der Dateiname fehlt.".to_string())?;
    fs::create_dir_all(dir)
        .map_err(|error| format!("Clip-Ordner konnte nicht erstellt werden: {error}"))?;
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let extension = source.extension().unwrap_or_default().to_string_lossy();
    for number in 0..10_000 {
        let destination = if number == 0 {
            dir.join(filename)
        } else {
            dir.join(format!("{stem} ({number}).{extension}"))
        };
        match copy_new(source, &destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Clip konnte nicht importiert werden: {error}")),
        }
    }
    Err("Zu viele Clips mit diesem Dateinamen vorhanden.".into())
}

fn valid_stem(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || name.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
        })
    {
        return Err("Bitte einen gültigen Namen ohne Pfad oder Sonderzeichen eingeben.".into());
    }
    let base = name.split('.').next().unwrap_or_default().trim_end();
    let upper = base.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            upper.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    if reserved {
        return Err("Dieser Name ist für Windows reserviert.".into());
    }
    Ok(name)
}

pub fn rename_clip(path: &Path, name: &str) -> Result<PathBuf, String> {
    let name = valid_stem(name)?;
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return Err("Der Clip wurde nicht gefunden.".into());
    }
    let mut filename = std::ffi::OsString::from(name);
    if let Some(extension) = path.extension() {
        filename.push(".");
        filename.push(extension);
    }
    if filename.to_string_lossy().encode_utf16().count() > 255 {
        return Err("Der Clip-Name ist zu lang.".into());
    }
    let destination = path.with_file_name(filename);
    if destination == path {
        return Ok(destination);
    }
    // A hard link is fast even for large clips and fails if the target exists.
    // The copy fallback also supports volumes without hard-link support.
    let result = match fs::hard_link(path, &destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(error),
        Err(_) => copy_new(path, &destination),
    };
    result.map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            "Ein Clip mit diesem Namen existiert bereits.".to_string()
        } else {
            format!("Clip konnte nicht umbenannt werden: {error}")
        }
    })?;
    if let Err(error) = fs::remove_file(path) {
        let _ = fs::remove_file(&destination);
        return Err(format!("Clip konnte nicht umbenannt werden: {error}"));
    }
    Ok(destination)
}

/// Cached facts about one clip file, keyed by path and invalidated by size/mtime.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub bytes: u64,
    pub modified: u64,
    pub info: MediaInfo,
    /// Poster frame file name inside the thumbnail directory, when generated.
    pub thumbnail: Option<String>,
    /// Probing failed; do not retry until the file changes.
    pub failed: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub entries: HashMap<String, IndexEntry>,
}

pub fn index_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn modified_seconds(clip: &Clip) -> u64 {
    clip.modified
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

impl Index {
    pub fn load(config_dir: &Path) -> Index {
        crate::settings::read_json(&config_dir.join("library-index.json")).unwrap_or_default()
    }

    pub fn save(&self, config_dir: &Path) {
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = fs::create_dir_all(config_dir);
            let _ = crate::settings::write_atomically(config_dir, "library-index.json", &bytes);
        }
    }

    /// The entry for a clip when it still matches the file on disk.
    pub fn current(&self, clip: &Clip) -> Option<&IndexEntry> {
        self.entries
            .get(&index_key(&clip.path))
            .filter(|entry| entry.bytes == clip.bytes && entry.modified == modified_seconds(clip))
    }

    pub fn needs_work(&self, clip: &Clip) -> bool {
        self.current(clip).is_none()
    }

    /// Removes entries (and poster frames) of clips that no longer exist.
    pub fn prune(&mut self, clips: &[Clip], thumbnail_dir: &Path) {
        let keep: std::collections::HashSet<String> =
            clips.iter().map(|c| index_key(&c.path)).collect();
        let stale: Vec<String> = self
            .entries
            .keys()
            .filter(|key| !keep.contains(*key))
            .cloned()
            .collect();
        for key in stale {
            if let Some(entry) = self.entries.remove(&key) {
                if let Some(thumbnail) = entry.thumbnail {
                    let _ = fs::remove_file(thumbnail_dir.join(thumbnail));
                }
            }
        }
    }
}

/// A stable file name for a clip's poster frame.
pub fn thumbnail_name(clip: &Clip) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in clip
        .path
        .to_string_lossy()
        .bytes()
        .chain(clip.bytes.to_le_bytes())
        .chain(modified_seconds(clip).to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}.jpg")
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
            let path =
                std::env::temp_dir().join(format!("stillclip-test-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn clip(name: &str, gigabytes: u64, age_days: u64) -> Clip {
        Clip {
            path: PathBuf::from(format!("C:/clips/{name}.mp4")),
            name: name.into(),
            bytes: gigabytes * 1024 * 1024 * 1024,
            created: Some(UNIX_EPOCH + std::time::Duration::from_secs(86_400 * (400 - age_days))),
            modified: None,
        }
    }

    #[test]
    fn storage_limit_removes_the_oldest_clips_only() {
        let clips = vec![
            clip("neu", 3, 1),
            clip("mittel", 4, 20),
            clip("alt", 5, 90),
        ];
        // Off by default and quiet while the folder fits.
        assert!(over_limit(&clips, 0).is_empty());
        assert!(over_limit(&clips, 12 * 1024 * 1024 * 1024).is_empty());
        // 10 GB: the oldest clip goes, the rest stays.
        let doomed = over_limit(&clips, 10 * 1024 * 1024 * 1024);
        assert_eq!(doomed.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["alt"]);
        // 3 GB: everything but the newest clip, which is never deleted.
        let doomed = over_limit(&clips, 3 * 1024 * 1024 * 1024);
        assert_eq!(
            doomed.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["alt", "mittel"]
        );
        assert!(over_limit(&clips[..1], 1).is_empty());
    }

    #[test]
    fn rejects_paths_and_windows_device_names() {
        for name in [
            "",
            " ",
            ".",
            "..",
            "../escape",
            "..\\escape",
            "C:\\escape",
            "bad:name",
            "bad\nname",
            "name.",
            "NUL",
            "con.txt",
            "COM1",
            "LPT²",
            "AUX .txt",
        ] {
            assert!(valid_stem(name).is_err(), "accepted {name:?}");
        }
        assert_eq!(
            valid_stem("  Mein schöner Clip  ").unwrap(),
            "Mein schöner Clip"
        );
        assert!(valid_stem("highlight.2026").is_ok());
    }

    #[test]
    fn import_and_rename_never_overwrite_existing_clips() {
        let directory = TestDir::new();
        let source = directory.0.join("clip.mp4");
        fs::write(&source, b"original").unwrap();
        let imported = import_clip(&source, &directory.0).unwrap();
        assert_eq!(imported.file_name().unwrap(), "clip (1).mp4");
        fs::write(&imported, b"another clip").unwrap();
        assert!(rename_clip(&source, "clip (1)").is_err());
        assert_eq!(fs::read(&source).unwrap(), b"original");
        assert_eq!(fs::read(&imported).unwrap(), b"another clip");
        assert!(rename_clip(&source, "../escaped").is_err());
        let renamed = rename_clip(&source, "Highlight").unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(renamed).unwrap(), b"original");
    }

    #[test]
    fn index_tracks_file_identity_and_prunes_missing_clips() {
        let directory = TestDir::new();
        let path = directory.0.join("clip.mp4");
        fs::write(&path, b"video").unwrap();
        let clips = scan_clips(&directory.0).unwrap();
        assert_eq!(clips.len(), 1);
        let clip = &clips[0];
        let mut index = Index::default();
        assert!(index.needs_work(clip));
        let thumbs = directory.0.join("thumbs");
        fs::create_dir_all(&thumbs).unwrap();
        let thumbnail = thumbnail_name(clip);
        fs::write(thumbs.join(&thumbnail), b"jpg").unwrap();
        index.entries.insert(
            index_key(&clip.path),
            IndexEntry {
                bytes: clip.bytes,
                modified: modified_seconds(clip),
                info: MediaInfo {
                    duration: 3.5,
                    width: 320,
                    height: 180,
                    audio: true,
                },
                thumbnail: Some(thumbnail.clone()),
                failed: false,
            },
        );
        assert!(!index.needs_work(clip));
        index.save(&directory.0);
        let loaded = Index::load(&directory.0);
        assert_eq!(loaded.current(clip).unwrap().info.duration, 3.5);
        fs::write(&path, b"changed video").unwrap();
        let changed = scan_clips(&directory.0).unwrap();
        assert!(index.needs_work(&changed[0]));
        index.prune(&[], &thumbs);
        assert!(index.entries.is_empty());
        assert!(!thumbs.join(thumbnail).exists());
    }
}
