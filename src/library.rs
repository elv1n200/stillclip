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
        Err(error) => return Err(crate::tr_format!("Clip-Ordner konnte nicht gelesen werden: {error}", "Could not read clips folder: {error}")),
    };
    let mut clips = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| crate::tr_format!("Clip konnte nicht gelesen werden: {error}", "Could not read clip: {error}"))?;
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
#[cfg(test)]
pub fn over_limit(clips: &[Clip], limit_bytes: u64) -> Vec<Clip> {
    over_limit_except(clips, limit_bytes, &std::collections::HashSet::new())
}

fn over_limit_except(
    clips: &[Clip],
    limit_bytes: u64,
    protected: &std::collections::HashSet<String>,
) -> Vec<Clip> {
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
        if protected.contains(&index_key(&clip.path)) {
            continue;
        }
        if total <= limit_bytes {
            break;
        }
        total = total.saturating_sub(clip.bytes);
        doomed.push(clip.clone());
    }
    doomed
}

/// Keep failed deletions in the library so their index and markers survive.
#[cfg(test)]
pub fn clean_limit(
    clips: &mut Vec<Clip>,
    limit_bytes: u64,
    remove: impl FnMut(&Path) -> io::Result<()>,
) -> (usize, u64, usize) {
    clean_limit_except(
        clips,
        limit_bytes,
        &std::collections::HashSet::new(),
        remove,
    )
}

pub fn clean_limit_except(
    clips: &mut Vec<Clip>,
    limit_bytes: u64,
    protected: &std::collections::HashSet<String>,
    mut remove: impl FnMut(&Path) -> io::Result<()>,
) -> (usize, u64, usize) {
    let mut removed = Vec::new();
    let mut freed = 0;
    let mut failed = 0;
    for clip in over_limit_except(clips, limit_bytes, protected) {
        if remove(&clip.path).is_ok() {
            freed += clip.bytes;
            removed.push(clip.path);
        } else {
            failed += 1;
        }
    }
    clips.retain(|clip| !removed.contains(&clip.path));
    (removed.len(), freed, failed)
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
            crate::i18n::text("Bitte eine Videodatei im Format MP4, MKV, MOV, WebM, AVI oder M4V wählen.", "Choose a video in MP4, MKV, MOV, WebM, AVI, or M4V format.").into(),
        );
    }
    let filename = source
        .file_name()
        .ok_or_else(|| crate::i18n::text("Der Dateiname fehlt.", "The filename is missing.").to_string())?;
    fs::create_dir_all(dir)
        .map_err(|error| crate::tr_format!("Clip-Ordner konnte nicht erstellt werden: {error}", "Could not create clips folder: {error}"))?;
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
            Err(error) => return Err(crate::tr_format!("Clip konnte nicht importiert werden: {error}", "Could not import clip: {error}")),
        }
    }
    Err(crate::i18n::text("Zu viele Clips mit diesem Dateinamen vorhanden.", "Too many clips already use this filename.").into())
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
        return Err(crate::i18n::text("Bitte einen gültigen Namen ohne Pfad oder Sonderzeichen eingeben.", "Enter a valid name without a path or special characters.").into());
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
        return Err(crate::i18n::text("Dieser Name ist für Windows reserviert.", "This name is reserved by Windows.").into());
    }
    Ok(name)
}

pub fn rename_clip(path: &Path, name: &str) -> Result<PathBuf, String> {
    let name = valid_stem(name)?;
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return Err(crate::i18n::text("Der Clip wurde nicht gefunden.", "Clip not found.").into());
    }
    let mut filename = std::ffi::OsString::from(name);
    if let Some(extension) = path.extension() {
        filename.push(".");
        filename.push(extension);
    }
    if filename.to_string_lossy().encode_utf16().count() > 255 {
        return Err(crate::i18n::text("Der Clip-Name ist zu lang.", "The clip name is too long.").into());
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
            crate::i18n::text("Ein Clip mit diesem Namen existiert bereits.", "A clip with this name already exists.").to_string()
        } else {
            crate::tr_format!("Clip konnte nicht umbenannt werden: {error}", "Could not rename clip: {error}")
        }
    })?;
    if let Err(error) = fs::remove_file(path) {
        let _ = fs::remove_file(&destination);
        return Err(crate::tr_format!("Clip konnte nicht umbenannt werden: {error}", "Could not rename clip: {error}"));
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
    /// Moments marked while recording, in seconds from the clip's start.
    #[serde(default)]
    pub markers: Vec<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub entries: HashMap<String, IndexEntry>,
    #[serde(default)]
    pub favorites: std::collections::HashSet<String>,
    #[serde(default)]
    pub annotations: HashMap<String, Annotation>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    #[serde(default)]
    pub game: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Annotation {
    pub fn normalized(game: &str, tags: &[String]) -> Result<Self, String> {
        let game = game.trim();
        if game.chars().count() > 80 || game.chars().any(char::is_control) {
            return Err(crate::i18n::text("Der Spielname darf höchstens 80 Zeichen enthalten.", "Game names must be no longer than 80 characters.").into());
        }
        let mut unique = Vec::<String>::new();
        for tag in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            if tag.chars().count() > 32 || tag.chars().any(char::is_control) || tag.contains(',') {
                return Err(crate::i18n::text("Tags dürfen höchstens 32 Zeichen und keine Kommas enthalten.", "Tags must contain no commas and be no longer than 32 characters.").into());
            }
            if !unique
                .iter()
                .any(|t| t.to_lowercase() == tag.to_lowercase())
            {
                unique.push(tag.into());
            }
        }
        if unique.len() > 12 {
            return Err(crate::i18n::text("Pro Clip sind höchstens 12 Tags möglich.", "A clip can have up to 12 tags.").into());
        }
        Ok(Self {
            game: game.into(),
            tags: unique,
        })
    }
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
        let _ = self.try_save(config_dir);
    }

    pub fn try_save(&self, config_dir: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
        crate::settings::write_atomically(config_dir, "library-index.json", &bytes)
            .map_err(|e| e.to_string())
    }

    pub fn annotate(
        &mut self,
        key: String,
        value: Annotation,
        config: &Path,
    ) -> Result<(), String> {
        let old = self.annotations.insert(key.clone(), value);
        if let Err(error) = self.try_save(config) {
            if let Some(value) = old {
                self.annotations.insert(key, value);
            } else {
                self.annotations.remove(&key);
            }
            return Err(crate::tr_format!("Tags konnten nicht gespeichert werden: {error}", "Could not save tags: {error}"));
        }
        Ok(())
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
        self.favorites.retain(|key| keep.contains(key));
        self.annotations.retain(|key, _| keep.contains(key));
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

    #[test]
    fn annotations_normalize_persist_and_roll_back_on_write_failure() {
        let dir = TestDir::new();
        let value = Annotation::normalized(
            " VALORANT ",
            &[" Ace ".into(), "ace".into(), "Clutch".into(), " ".into()],
        )
        .unwrap();
        assert_eq!(value.tags, ["Ace", "Clutch"]);
        assert_eq!(value.game, "VALORANT");
        assert!(Annotation::normalized("Game", &["x".repeat(33)]).is_err());
        assert!(Annotation::normalized(
            "Game",
            &(0..13).map(|i| i.to_string()).collect::<Vec<_>>()
        )
        .is_err());
        let mut index = Index::default();
        index
            .annotate("clip.mp4".into(), value.clone(), &dir.0)
            .unwrap();
        assert_eq!(Index::load(&dir.0).annotations["clip.mp4"], value);
        let blocked = dir.0.join("file");
        fs::write(&blocked, "blocked").unwrap();
        assert!(index
            .annotate("clip.mp4".into(), Annotation::default(), &blocked)
            .is_err());
        assert_eq!(index.annotations["clip.mp4"], value);
        let old: Index = serde_json::from_str(r#"{"entries":{}}"#).unwrap();
        assert!(old.annotations.is_empty());
    }

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
    fn failed_cleanup_keeps_the_clip_and_its_markers_in_the_index() {
        let mut clips = vec![clip("old", 2, 3), clip("locked", 2, 2), clip("new", 2, 1)];
        let locked = index_key(&clips[1].path);
        let mut index = Index::default();
        index.entries.insert(
            locked.clone(),
            IndexEntry {
                markers: vec![2.5],
                ..Default::default()
            },
        );
        let (count, _, failed) = clean_limit(&mut clips, 2 * 1024 * 1024 * 1024, |path| {
            if index_key(path) == locked {
                Err(io::ErrorKind::PermissionDenied.into())
            } else {
                Ok(())
            }
        });
        assert_eq!((count, failed, clips.len()), (1, 1, 2));
        index.prune(&clips, Path::new("unused-thumbnails"));
        assert_eq!(index.entries[&locked].markers, vec![2.5]);
    }

    #[test]
    fn favorites_survive_cleanup_and_round_trip_without_blocking_other_deletions() {
        let mut clips = vec![
            clip("favorite", 2, 4),
            clip("old", 2, 3),
            clip("middle", 2, 2),
            clip("new", 2, 1),
        ];
        let favorite = index_key(&clips[0].path);
        let mut index = Index::default();
        index.favorites.insert(favorite.clone());
        let (count, _, failed) = clean_limit_except(
            &mut clips,
            4 * 1024 * 1024 * 1024,
            &index.favorites,
            |path| {
                assert_ne!(index_key(path), favorite);
                Ok(())
            },
        );
        assert_eq!((count, failed), (2, 0));
        assert_eq!(clips.len(), 2);
        index.prune(&clips, Path::new("unused"));
        let dir = TestDir::new();
        index.save(&dir.0);
        assert!(Index::load(&dir.0).favorites.contains(&favorite));
        let legacy: Index = serde_json::from_str("{\"entries\":{}}").unwrap();
        assert!(legacy.favorites.is_empty());
    }

    #[test]
    fn storage_limit_removes_the_oldest_clips_only() {
        let clips = vec![clip("neu", 3, 1), clip("mittel", 4, 20), clip("alt", 5, 90)];
        // Off by default and quiet while the folder fits.
        assert!(over_limit(&clips, 0).is_empty());
        assert!(over_limit(&clips, 12 * 1024 * 1024 * 1024).is_empty());
        // 10 GB: the oldest clip goes, the rest stays.
        let doomed = over_limit(&clips, 10 * 1024 * 1024 * 1024);
        assert_eq!(
            doomed.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["alt"]
        );
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
                markers: vec![1.5],
            },
        );
        assert!(!index.needs_work(clip));
        index.save(&directory.0);
        let loaded = Index::load(&directory.0);
        assert_eq!(loaded.current(clip).unwrap().info.duration, 3.5);
        // Marked moments survive a round trip through the saved index.
        assert_eq!(loaded.current(clip).unwrap().markers, vec![1.5]);
        fs::write(&path, b"changed video").unwrap();
        let changed = scan_clips(&directory.0).unwrap();
        assert!(index.needs_work(&changed[0]));
        index.prune(&[], &thumbs);
        assert!(index.entries.is_empty());
        assert!(!thumbs.join(thumbnail).exists());
    }
}
