//! Media sources: local image, video, and audio files.
//!
//! A [`MediaItem`] describes one playable file. The module also owns the
//! `media://` scheme registry: every item that is being shown (or queued) is
//! registered under a numeric id. The registry doubles as an allow-list for
//! the renderer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Kind of media a source can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

/// How a media element fills the output display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectFit {
    /// Crop to cover the whole screen.
    #[default]
    Cover,
    /// Shrink to fit the whole media inside the screen, letterboxing.
    Contain,
}

/// A single playable media file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    /// Absolute path to the file on disk.
    pub path: String,
    /// File name without the directory component.
    pub name: String,
    /// Whether the file is an image, video, or audio.
    pub kind: MediaKind,
    /// How the media fills the output display.
    pub fit: ObjectFit,
    /// Display the item prefers to play on; empty inherits the caller's display.
    pub display: String,
    pub audio_device: String,
}

impl MediaItem {
    /// Builds a [`MediaItem`] from a path, without touching the disk.
    pub fn from_path(path: impl Into<String>) -> MediaItem {
        let path = path.into();
        let name = Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let kind = kind_from_path(&path);
        MediaItem {
            path,
            name,
            kind,
            fit: ObjectFit::default(),
            display: String::new(),
            audio_device: String::new(),
        }
    }
}

/// One queued item as supplied by the UI: a path, its fit mode, the display it
/// should play on (empty to follow the master display), and the output device
/// its audio should go to (empty to follow the preferred output).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
    pub path: String,
    #[serde(default)]
    pub fit: ObjectFit,
    #[serde(default)]
    pub display: String,
    #[serde(default)]
    pub audio_device: String,
    /// False hides the item from “Play ticked” and from saved cues. Older
    /// saved queues have no such field, which reads as ticked.
    #[serde(default = "ticked_by_default")]
    pub selected: bool,
}

/// Hand-written so that a default entry is ticked, matching the serde default:
/// a queue saved before ticking existed should keep playing in full rather than
/// coming back empty.
impl Default for PlaylistEntry {
    fn default() -> Self {
        Self {
            path: String::new(),
            fit: ObjectFit::default(),
            display: String::new(),
            audio_device: String::new(),
            selected: ticked_by_default(),
        }
    }
}

/// Serde default for [`PlaylistEntry::selected`].
fn ticked_by_default() -> bool {
    true
}

/// File extensions treated as images.
pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "ico"];

/// File extensions treated as videos.
pub const VIDEO_EXTS: &[&str] = &["mp4", "webm", "mov", "mkv", "avi", "ogv", "m4v"];

/// File extensions treated as audio.
pub const AUDIO_EXTS: &[&str] = &[
    "mp3", "m4a", "aac", "wav", "flac", "ogg", "oga", "opus", "wma",
];

/// Recognizes an extension as playable media, or `None` for anything else.
pub fn classify(path: &str) -> Option<MediaKind> {
    let ext = Path::new(path)
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some(ext) if IMAGE_EXTS.contains(&ext) => Some(MediaKind::Image),
        Some(ext) if VIDEO_EXTS.contains(&ext) => Some(MediaKind::Video),
        Some(ext) if AUDIO_EXTS.contains(&ext) => Some(MediaKind::Audio),
        _ => None,
    }
}

/// Guesses the media kind from a file extension.
///
/// Unsupported extensions are treated as images so that a non-media file
/// still flows to the renderer and is shown with its natural `content-type`
/// (e.g. text); the renderer is the last arbiter of what it can display.
pub fn kind_from_path(path: &str) -> MediaKind {
    classify(path).unwrap_or(MediaKind::Image)
}

/// Returns the MIME type used when serving a media id over `media://`.
pub fn mime_for(kind: MediaKind, path: &str) -> String {
    let ext = Path::new(path)
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match (kind, ext.as_str()) {
        (MediaKind::Video, "webm") => "video/webm".into(),
        (MediaKind::Video, "mov" | "m4v" | "mp4") => "video/mp4".into(),
        (MediaKind::Video, "mkv") => "video/x-matroska".into(),
        (MediaKind::Video, "avi") => "video/x-msvideo".into(),
        (MediaKind::Video, "ogv") => "video/ogg".into(),
        (MediaKind::Video, _) => "video/mp4".into(),
        (MediaKind::Audio, "mp3") => "audio/mpeg".into(),
        (MediaKind::Audio, "m4a") => "audio/mp4".into(),
        (MediaKind::Audio, "aac") => "audio/aac".into(),
        (MediaKind::Audio, "wav") => "audio/wav".into(),
        (MediaKind::Audio, "flac") => "audio/flac".into(),
        (MediaKind::Audio, "ogg" | "oga" | "opus") => "audio/ogg".into(),
        (MediaKind::Audio, "wma") => "audio/x-ms-wma".into(),
        (MediaKind::Audio, _) => "audio/mpeg".into(),
        (MediaKind::Image, "jpg" | "jpeg") => "image/jpeg".into(),
        (MediaKind::Image, "png") => "image/png".into(),
        (MediaKind::Image, "gif") => "image/gif".into(),
        (MediaKind::Image, "webp") => "image/webp".into(),
        (MediaKind::Image, "bmp") => "image/bmp".into(),
        (MediaKind::Image, "svg") => "image/svg+xml".into(),
        (MediaKind::Image, "ico") => "image/x-icon".into(),
        (MediaKind::Image, _) => "application/octet-stream".into(),
    }
}

/// Registry of files currently allowed to be served to the renderer.
///
/// Managed as application state; hidden behind an opaque interface because
/// the id space is the only contract the renderer window and the protocol
/// handler share.
///
/// NOTE: For long-running sessions, consider implementing an LRU eviction
/// policy to prevent unbounded growth. Currently relies on explicit
/// `release()` calls from scheduler when items are no longer needed.
#[derive(Default)]
pub struct Registry(Mutex<RegistryInner>);

#[derive(Default)]
struct RegistryInner {
    next_id: u64,
    by_id: HashMap<u64, MediaItem>,
}

impl Registry {
    /// Returns the URL for a file, registering it if needed.
    pub fn register(&self, item: MediaItem) -> String {
        let mut inner = self.0.lock().unwrap();
        let id = match inner
            .by_id
            .iter()
            .find(|(_, entry)| entry.path == item.path)
        {
            Some((id, _)) => *id,
            None => {
                let id = inner.next_id;
                inner.next_id += 1;
                inner.by_id.insert(id, item);
                id
            }
        };
        register_url(id)
    }

    /// Looks up a registered item by its numeric id.
    pub fn get(&self, id: u64) -> Option<MediaItem> {
        self.0.lock().unwrap().by_id.get(&id).cloned()
    }

    /// Forgets a file that is no longer displayed or queued.
    pub fn release(&self, id: u64) {
        self.0.lock().unwrap().by_id.remove(&id);
    }
}

/// Builds the `media://localhost/<id>` URL.
fn register_url(id: u64) -> String {
    format!("media://localhost/{id}")
}

/// Parses the numeric id from a `media://` URL.
pub fn id_from_url(url: &str) -> Option<u64> {
    let rest = url.strip_prefix("media://")?;
    let token = rest
        .rsplit('/')
        .map(|part| {
            part.split('?')
                .next()
                .unwrap_or("")
                .split('#')
                .next()
                .unwrap_or("")
        })
        .find(|part| !part.is_empty())
        .unwrap_or("");
    token.parse().ok()
}

/// Absolute path of a file, or `None` when it exists but is a directory.
pub fn canonical_path(path: &Path) -> Option<PathBuf> {
    if !path.is_file() {
        return None;
    }
    path.canonicalize().ok()
}

/// Diagnostics for a `media://` URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaProbe {
    pub url: String,
    pub id: Option<u64>,
    pub registered: bool,
    pub path: Option<String>,
    pub exists: bool,
    pub size: Option<u64>,
    pub kind: Option<String>,
    pub mime: Option<String>,
}

/// Collects everything needed to serve `url`.
pub fn probe(url: &str, registry: &Registry) -> MediaProbe {
    let id = id_from_url(url);
    let item = id.and_then(|id| registry.get(id));
    let path = item.as_ref().map(|item| item.path.clone());
    let exists = path.as_ref().is_some_and(|path| Path::new(path).is_file());
    let size = path
        .as_ref()
        .and_then(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len());
    let kind = item.as_ref().map(|item| match item.kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
    });
    let mime = item.as_ref().map(|item| mime_for(item.kind, &item.path));
    MediaProbe {
        url: url.to_string(),
        id,
        registered: item.is_some(),
        path,
        exists,
        size,
        kind: kind.map(str::to_string),
        mime,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<MediaItem> {
        vec![
            MediaItem::from_path("/media/logo.png"),
            MediaItem::from_path("C:\\media\\advert.mp4"),
            MediaItem::from_path("/media/shell.txt"),
        ]
    }

    #[test]
    fn guesses_kind_from_extension() {
        assert_eq!(kind_from_path("/x/a.png"), MediaKind::Image);
        assert_eq!(kind_from_path("/x/a.jpeg"), MediaKind::Image);
        assert_eq!(kind_from_path("/x/a.mp4"), MediaKind::Video);
        assert_eq!(kind_from_path("/x/a.MKV"), MediaKind::Video);
        assert_eq!(kind_from_path("/x/a.webm"), MediaKind::Video);
        assert_eq!(kind_from_path("/x/a.mp3"), MediaKind::Audio);
        assert_eq!(kind_from_path("/x/a.WAV"), MediaKind::Audio);
        assert_eq!(kind_from_path("/x/a.flac"), MediaKind::Audio);
        assert_eq!(kind_from_path("/x/a.bin"), MediaKind::Image);
    }

    #[test]
    fn classifies_only_recognized_extensions() {
        assert_eq!(classify("/x/a.png"), Some(MediaKind::Image));
        assert_eq!(classify("/x/a.gif"), Some(MediaKind::Image));
        assert_eq!(classify("/x/a.mp4"), Some(MediaKind::Video));
        assert_eq!(classify("/x/a.MKV"), Some(MediaKind::Video));
        assert_eq!(classify("/x/a.mp3"), Some(MediaKind::Audio));
        assert_eq!(classify("/x/a.opus"), Some(MediaKind::Audio));
        assert_eq!(classify("/x/a.ogg"), Some(MediaKind::Audio));
        assert_eq!(classify("/x/a.txt"), None);
        assert_eq!(classify("/x/no-ext"), None);
        assert_eq!(classify("/x/archive.gz"), None);
    }

    #[test]
    fn maps_content_types() {
        assert_eq!(mime_for(MediaKind::Image, "a.jpg"), "image/jpeg");
        assert_eq!(mime_for(MediaKind::Image, "a.png"), "image/png");
        assert_eq!(mime_for(MediaKind::Video, "a.mp4"), "video/mp4");
        assert_eq!(mime_for(MediaKind::Video, "a.webm"), "video/webm");
        assert_eq!(mime_for(MediaKind::Audio, "a.mp3"), "audio/mpeg");
        assert_eq!(mime_for(MediaKind::Audio, "a.wav"), "audio/wav");
        assert_eq!(mime_for(MediaKind::Audio, "a.flac"), "audio/flac");
        assert_eq!(mime_for(MediaKind::Audio, "a.ogg"), "audio/ogg");
    }

    #[test]
    fn registers_and_resolves_ids() {
        let registry = Registry::default();
        let url = registry.register(items()[0].clone());
        assert!(url.starts_with("media://localhost/"));
        let id = id_from_url(&url).expect("id parses");
        assert_eq!(registry.get(id), Some(items()[0].clone()));

        let same = registry.register(items()[0].clone());
        assert_eq!(same, url);
    }

    #[test]
    fn parsing_handles_path_and_query_forms() {
        assert_eq!(id_from_url("media://localhost/0"), Some(0));
        assert_eq!(id_from_url("media://localhost/42"), Some(42));
        assert_eq!(id_from_url("media://7"), Some(7));
        assert_eq!(id_from_url("media://localhost/3?range=yes"), Some(3));
        assert_eq!(id_from_url("media://localhost/3#frag"), Some(3));
        assert_eq!(id_from_url("media://localhost/"), None);
        assert_eq!(id_from_url("media://"), None);
        assert_eq!(id_from_url("not-media://1"), None);
    }

    #[test]
    fn refuses_unknown_ids_and_releases() {
        let registry = Registry::default();
        let url = registry.register(items()[1].clone());
        let id = id_from_url(&url).unwrap();
        registry.release(id);
        assert_eq!(registry.get(id), None);
        assert_eq!(registry.get(999), None);
        assert_eq!(id_from_url("media://localhost/"), None);
        assert_eq!(id_from_url("http://x"), None);
    }

    #[test]
    fn builds_item_name_and_kind_from_path() {
        let item = MediaItem::from_path("/somewhere/logo.png");
        assert_eq!(item.name, "logo.png");
        assert_eq!(item.kind, MediaKind::Image);
        assert_eq!(item.path, "/somewhere/logo.png");

        let audio = MediaItem::from_path("/somewhere/theme.mp3");
        assert_eq!(audio.audio_device, "");
    }

    #[test]
    fn probe_reports_registry_and_disk_state() {
        let dir = std::env::temp_dir().join(format!("hmc-probe-{}", std::process::id()));
        std::fs::write(&dir, "fake-image-bytes").unwrap();
        let registry = Registry::default();
        let url = registry.register(MediaItem::from_path(dir.to_string_lossy().into_owned()));

        let info = probe(&url, &registry);
        assert_eq!(info.id, id_from_url(&url));
        assert!(info.registered);
        assert_eq!(info.kind.as_deref(), Some("image"));
        assert!(info.exists);
        assert_eq!(info.size, Some(16));
        assert_eq!(info.mime.as_deref(), Some("application/octet-stream"));

        let missing_info = probe("media://localhost/999", &registry);
        assert!(!missing_info.registered);
        assert!(!missing_info.exists);

        let _ = std::fs::remove_file(&dir);
    }
}
