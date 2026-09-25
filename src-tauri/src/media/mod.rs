//! Media sources: local image, video, and audio files.
//!
//! A [`MediaItem`] describes one playable file. The module also owns the
//! `media://` scheme registry backing the renderer: every item that is
//! being shown (or queued) is registered under a numeric id, and the
//! `serve` protocol handler streams the file bytes to the output window.
//!
//! The registry doubles as an allow-list — the protocol refuses ids that
//! were never registered, and refuse anything but registered files, so the
//! renderer cannot be tricked into reading arbitrary paths from the disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::logging;

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
#[derive(Default)]
pub struct Registry(Mutex<RegistryInner>);

#[derive(Default)]
struct RegistryInner {
    next_id: u64,
    by_id: HashMap<u64, MediaItem>,
}

impl Registry {
    /// Returns the URL for a file, registering it if needed. WebView2 (the
    /// Windows webview) does not support real custom URI schemes — Tauri only
    /// routes requests to a registered handler through `http://<scheme>.localhost`.
    /// So on Windows the id lives in the *path* of an http URL; on other
    /// platforms the scheme itself is delivered to the handler.
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

/// Builds the `media://localhost/<id>` URL (or `http://media.localhost/<id>`
/// on Windows, where WebView2/Tauri has no real custom scheme support).
fn register_url(id: u64) -> String {
    if cfg!(target_os = "windows") {
        format!("http://media.localhost/{id}")
    } else {
        format!("media://localhost/{id}")
    }
}

/// Parses the numeric id from a `media://` URL (`media://localhost/<id>`,
/// legacy `media://<id>`) or the Windows `http://media.localhost/<id>` form.
pub fn id_from_url(url: &str) -> Option<u64> {
    let rest = url.strip_prefix("media://").or_else(|| {
        url.strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))?
            .strip_prefix("media.localhost/")
    })?;
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

/// Extracts the numeric id from a media request's URI. Tolerates webviews
/// that deliver the id as a bare path (`media://localhost/N`, `/N`) and the
/// legacy authority form (`media://N`).
fn id_from_request(uri: &tauri::http::Uri) -> Option<u64> {
    id_from_url(&uri.to_string()).or_else(|| {
        uri.path()
            .trim_start_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("")
            .parse()
            .ok()
    })
}

/// Absolute path of a file, or `None` when it exists but is a directory.
pub fn canonical_path(path: &Path) -> Option<PathBuf> {
    if !path.is_file() {
        return None;
    }
    path.canonicalize().ok()
}

/// Serves registered media files over the `media://` custom URI scheme.
///
/// Registered ids are streamed with their natural `Content-Type`; video
/// requests honor byte ranges so the renderer can seek. Anything that was
/// never registered (arbitrary ids) is refused with 404 — the registry is
/// the allow-list.
pub fn serve(
    ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
    use tauri::http::{Response, StatusCode};
    use tauri::Manager;

    let id = id_from_request(request.uri());
    let Some(item) = id.and_then(|id| ctx.app_handle().state::<Registry>().get(id)) else {
        logging::error(&format!("media:// serve 404 for uri {}", request.uri()));
        return not_found();
    };

    let Ok(bytes) = std::fs::read(&item.path) else {
        logging::error(&format!(
            "media:// serve read failed for {:?} (id {:?})",
            item.path, id
        ));
        return not_found();
    };
    let total = bytes.len();
    let mime = mime_for(item.kind, &item.path);

    // Honor a single `bytes=start-end` or `bytes=start-` range (the webview
    // uses these while scrubbing video); fall back to serving the whole file.
    let requested = request
        .headers()
        .get(RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_range);

    match requested {
        Some((start, end)) if start < total => {
            let end = end.min(total - 1);
            let body = bytes[start..=end].to_vec();
            logging::info(&format!(
                "media:// serve {} -> 206 {}bytes {}",
                request.uri(),
                body.len(),
                mime
            ));
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(CONTENT_TYPE, mime)
                .header(ACCEPT_RANGES, "bytes")
                .header(CONTENT_LENGTH, body.len())
                .header(CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
                .body(body)
                .unwrap_or_else(|_| not_found())
        }
        _ => {
            logging::info(&format!(
                "media:// serve {} -> 200 {}bytes {}",
                request.uri(),
                total,
                mime
            ));
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, mime)
                .header(ACCEPT_RANGES, "bytes")
                .header(CONTENT_LENGTH, total)
                .body(bytes)
                .unwrap_or_else(|_| not_found())
        }
    }
}

/// Parses a `Range` header value into an inclusive byte window.
fn parse_range(value: &str) -> Option<(usize, usize)> {
    let spec = value.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start = start.parse::<usize>().ok()?;
    let end = if end.is_empty() {
        None
    } else {
        end.parse::<usize>().ok()
    };
    match end {
        Some(end) => Some((start, end)),
        None => Some((start, usize::MAX)),
    }
}

fn not_found() -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(tauri::http::StatusCode::NOT_FOUND)
        .body(Vec::new())
        .unwrap_or_else(|_| tauri::http::Response::default())
}

/// Diagnostics for a single `media://` URL, shown on the renderer when a
/// load fails so the cause (id routing, registry, missing file) is visible.
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

/// Collects everything the protocol handler would need to serve `url`.
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
        assert!(
            url.starts_with("media://localhost/") || url.starts_with("http://media.localhost/")
        );
        let id = id_from_url(&url).expect("id parses");
        assert_eq!(registry.get(id), Some(items()[0].clone()));

        let same = registry.register(items()[0].clone());
        assert_eq!(same, url);
    }

    #[test]
    fn parsing_handles_path_authority_and_query_forms() {
        assert_eq!(id_from_url("media://localhost/0"), Some(0));
        assert_eq!(id_from_url("media://localhost/42"), Some(42));
        assert_eq!(id_from_url("media://7"), Some(7));
        assert_eq!(id_from_url("media://localhost/3?range=yes"), Some(3));
        assert_eq!(id_from_url("media://localhost/3#frag"), Some(3));
        assert_eq!(id_from_url("media://localhost/"), None);
        assert_eq!(id_from_url("media://"), None);
        assert_eq!(id_from_url("not-media://1"), None);
        assert_eq!(id_from_url("http://media.localhost/0"), Some(0));
        assert_eq!(id_from_url("http://media.localhost/3?range=yes"), Some(3));
        assert_eq!(id_from_url("https://media.localhost/5"), Some(5));
        assert_eq!(id_from_url("http://media.localhost/"), None);
        assert_eq!(id_from_url("http://example.com/0"), None);

        // Bare-path requests (some webviews deliver just `/N`).
        let uri: tauri::http::Uri = "/9".parse().unwrap();
        assert_eq!(id_from_request(&uri), Some(9));
        let uri: tauri::http::Uri = "media://localhost/11".parse().unwrap();
        assert_eq!(id_from_request(&uri), Some(11));
        let uri: tauri::http::Uri = "media://13".parse().unwrap();
        assert_eq!(id_from_request(&uri), Some(13));
        let uri: tauri::http::Uri = "http://media.localhost/17".parse().unwrap();
        assert_eq!(id_from_request(&uri), Some(17));
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

    #[test]
    fn parses_byte_ranges() {
        assert_eq!(parse_range("bytes=0-1023"), Some((0, 1023)));
        assert_eq!(parse_range("bytes=1024-"), Some((1024, usize::MAX)));
        assert_eq!(parse_range("bytes=5-10"), Some((5, 10)));
        assert_eq!(parse_range("bytes=0-"), Some((0, usize::MAX)));
        assert_eq!(parse_range("bytes=-100"), None);
        assert_eq!(parse_range("items=1-2"), None);
        assert_eq!(parse_range("bytes=abc"), None);
    }
}
