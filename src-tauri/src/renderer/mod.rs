//! Output rendering: borderless/fullscreen windows placed on a chosen display.
//!
//! Every output is an undecorated webview window that fills the rectangle of
//! the target monitor (via [`crate::display::model::DisplayBounds`]) and
//! plays a single [`crate::media::MediaItem`] sourced from the `media://`
//! registry. The control window stays wherever the user put it; output
//! windows are `always_on_top` + `skip_taskbar` so they behave like a
//! signage panel.
//!
//! Output windows are created with `WebviewWindowBuilder`, which on Windows
//! must never be called from a synchronous command handler (deadlock,
//! see the Tauri docs) — all callers in this app go through async commands
//! or the scheduler's worker thread.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::media::{self, MediaItem, MediaKind, ObjectFit, Registry};

/// One open output window and what it is showing.
pub struct RenderSession {
    payload: RenderPayload,
}

/// What the `render.html` page needs to show media in an output window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderPayload {
    /// `media://<id>` URL to load.
    pub media_url: String,
    /// `media://` poster URL (thumbnail or first frame) shown before load.
    pub poster_url: String,
    /// Media kind — decides which element `render.html` creates.
    pub kind: MediaKind,
    /// Friendly title shown as the window title and element fallback text.
    pub title: String,
    /// Optional caption rendered over the bottom edge of the output.
    pub overlay: String,
    /// How the media fills the output (`cover` or `contain`).
    pub fit: ObjectFit,
}

/// Tracks open output windows, keyed by their webview label.
#[derive(Default)]
pub struct RendererState(Mutex<HashMap<String, RenderSession>>);

impl RendererState {
    /// Returns the payload for the window with the given label, if open.
    pub fn payload(&self, label: &str) -> Option<RenderPayload> {
        self.0
            .lock()
            .unwrap()
            .get(label)
            .map(|session| session.payload.clone())
    }

    /// Labels of every open output window.
    pub fn labels(&self) -> Vec<String> {
        self.0.lock().unwrap().keys().cloned().collect()
    }
}

/// Playback volume shared by every output window, from 0.0 (silent) to 1.0
/// (full). Managed apart from [`RendererState`] so it survives windows
/// opening and closing.
pub struct Volume(Mutex<f32>);

impl Default for Volume {
    fn default() -> Self {
        Self(Mutex::new(1.0))
    }
}

impl Volume {
    /// The level output windows should play at.
    pub fn get(&self) -> f32 {
        *self.0.lock().unwrap()
    }

    /// Stores a new level, clamped to 0.0–1.0.
    pub fn set(&self, level: f32) {
        *self.0.lock().unwrap() = level.clamp(0.0, 1.0);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPayload {
    pub media_url: String,
    pub title: String,
}

#[derive(Default)]
pub struct PreviewState(Mutex<Option<PreviewPayload>>);

impl PreviewState {
    fn set(&self, payload: PreviewPayload) {
        *self.0.lock().unwrap() = Some(payload);
    }

    pub fn payload(&self) -> Option<PreviewPayload> {
        self.0.lock().unwrap().clone()
    }

    fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }
}

pub const PREVIEW_LABEL: &str = "preview";

/// Errors produced while opening or managing output windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererError {
    NotSupportedOnPlatform,
    /// The display has no usable screen bounds to position the window on.
    NoDisplayBounds(String),
    /// Building or closing the window failed (WebView2/webkit error).
    Creation(String),
}

impl std::fmt::Display for RendererError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSupportedOnPlatform => write!(f, "Rendering is only supported on Windows."),
            Self::NoDisplayBounds(name) => {
                write!(f, "Display {name} has no usable screen bounds.")
            }
            Self::Creation(detail) => write!(f, "Could not open output window: {detail}"),
        }
    }
}

impl std::error::Error for RendererError {}

/// Creates a window label for a display, e.g. `output-DISPLAY1-3`.
pub fn label_for(device_name: &str, seq: u64) -> String {
    let name = device_name
        .rsplit('\\')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("display");
    format!("output-{name}-{seq}")
}

/// Opens a borderless fullscreen window on `display` playing `item`.
///
/// The window fills the display's bounds, then is forced into fullscreen.
/// This must run off the main/command thread on Windows (see module docs).
pub fn open(
    app: &AppHandle,
    display: &crate::display::model::DisplayInfo,
    item: &MediaItem,
    label: &str,
    overlay: &str,
) -> Result<(), RendererError> {
    let bounds = display.bounds;
    if bounds.width == 0 || bounds.height == 0 {
        return Err(RendererError::NoDisplayBounds(display.device_name.clone()));
    }

    let media_url = app.state::<Registry>().register(item.clone());

    let payload = RenderPayload {
        media_url: media_url.clone(),
        poster_url: media_url.clone(),
        kind: item.kind,
        title: item.name.clone(),
        overlay: overlay.to_string(),
        fit: item.fit,
    };
    // Register the payload *before* building the window: a newly created
    // webview can start executing render.html (and invoke the token) before
    // open() would otherwise get around to inserting this entry.
    app.state::<RendererState>()
        .0
        .lock()
        .unwrap()
        .insert(label.to_string(), RenderSession { payload });

    let window = match WebviewWindowBuilder::new(
        app,
        label.to_string(),
        WebviewUrl::App("render.html".into()),
    )
    .title(format!("Media \u{2014} {}", item.name))
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    // Allow video autoplay with sound (output windows should start playing
    // immediately). Wry's Windows defaults are re-applied because setting
    // custom args replaces them.
    .additional_browser_args(
        "--autoplay-policy=no-user-gesture-required --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
    )
    .position(bounds.x as f64, bounds.y as f64)
    .inner_size(bounds.width as f64, bounds.height as f64)
    .build()
    {
        Ok(window) => window,
        Err(err) => {
            app.state::<RendererState>().0.lock().unwrap().remove(label);
            return Err(RendererError::Creation(err.to_string()));
        }
    };

    let _ = window.set_fullscreen(true);

    Ok(())
}

pub fn open_preview(app: &AppHandle, item: &MediaItem) -> Result<(), RendererError> {
    let media_url = app.state::<Registry>().register(item.clone());
    let payload = PreviewPayload {
        media_url,
        title: item.name.clone(),
    };
    app.state::<PreviewState>().set(payload.clone());

    if let Some(window) = app.get_webview_window(PREVIEW_LABEL) {
        let _ = app.emit_to(PREVIEW_LABEL, "preview-media", payload);
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let mut builder = WebviewWindowBuilder::new(
        app,
        PREVIEW_LABEL,
        WebviewUrl::App("preview.html".into()),
    )
    .title(format!("Video preview \u{2014} {}", item.name))
    .resizable(true)
    .min_inner_size(640.0, 360.0)
    .inner_size(960.0, 540.0)
    .center()
    .additional_browser_args(
        "--autoplay-policy=no-user-gesture-required --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
    )
    .on_web_resource_request(|_request, response| {
        response.headers_mut().insert(
            tauri::http::header::HeaderName::from_static("permissions-policy"),
            tauri::http::header::HeaderValue::from_static("speaker-selection=(self)"),
        );
    });

    if let Ok(displays) = crate::display::list_displays() {
        if let Some(primary) = displays.iter().find(|display| display.is_primary) {
            let width = 960.0;
            let height = 540.0;
            let x = primary.bounds.x as f64 + (primary.bounds.width as f64 - width).max(0.0) / 2.0;
            let y =
                primary.bounds.y as f64 + (primary.bounds.height as f64 - height).max(0.0) / 2.0;
            builder = builder.position(x, y);
        }
    }

    match builder.build() {
        Ok(_) => Ok(()),
        Err(err) => {
            app.state::<PreviewState>().clear();
            Err(RendererError::Creation(err.to_string()))
        }
    }
}

pub fn close_preview(app: &AppHandle) -> bool {
    app.state::<PreviewState>().clear();
    let Some(window) = app.get_webview_window(PREVIEW_LABEL) else {
        return false;
    };
    let _ = window.close();
    true
}

/// Opens an invisible output window that plays `item` as sound only.
///
/// Audio never appears on any display: the window is hidden, has no
/// decorations, skips the taskbar, and sits far off-screen. The webview still
/// decodes the file and emits `render-finished`/`media-progress`, which the
/// scheduler uses to advance and time the item.
pub fn open_audio(
    app: &AppHandle,
    item: &MediaItem,
    label: &str,
    overlay: &str,
) -> Result<(), RendererError> {
    let media_url = app.state::<Registry>().register(item.clone());

    let payload = RenderPayload {
        media_url: media_url.clone(),
        poster_url: media_url.clone(),
        kind: item.kind,
        title: item.name.clone(),
        overlay: overlay.to_string(),
        fit: item.fit,
    };
    app.state::<RendererState>()
        .0
        .lock()
        .unwrap()
        .insert(label.to_string(), RenderSession { payload });

    let window = match WebviewWindowBuilder::new(
        app,
        label.to_string(),
        WebviewUrl::App("render.html".into()),
    )
    .title(format!("Audio \u{2014} {}", item.name))
    .decorations(false)
    .resizable(false)
    .visible(false)
    .skip_taskbar(true)
    .additional_browser_args(
        "--autoplay-policy=no-user-gesture-required --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
    )
    .position(-32000.0, -32000.0)
    .inner_size(1.0, 1.0)
    .build()
    {
        Ok(window) => window,
        Err(err) => {
            app.state::<RendererState>().0.lock().unwrap().remove(label);
            return Err(RendererError::Creation(err.to_string()));
        }
    };

    let _ = window.set_fullscreen(false);

    Ok(())
}

/// Closes the output with `label`. Registry entries are intentionally kept
/// — the control UI shows live thumbnails for every queued item via
/// `media://`, and the registry deduplicates by path. Returns whether a
/// window was actually open.
pub fn close(app: &AppHandle, label: &str) -> bool {
    let state = app.state::<RendererState>();
    let mut sessions = state.0.lock().unwrap();
    let Some(_session) = sessions.remove(label) else {
        return false;
    };
    drop(sessions);

    if let Some(window) = app.get_webview_window(label) {
        let _ = window.close();
    }
    true
}

/// Closes every open output window; returns how many were closed.
pub fn close_all(app: &AppHandle) -> usize {
    let labels = app.state::<RendererState>().labels();
    labels.into_iter().filter(|label| close(app, label)).count()
}

/// Refreshes the stored payload's fit mode for an open window, so a re-fetch
/// of the render token reflects the change. Returns whether the window exists.
pub fn set_fit(app: &AppHandle, label: &str, fit: ObjectFit) -> bool {
    let state = app.state::<RendererState>();
    let mut sessions = state.0.lock().unwrap();
    let Some(session) = sessions.get_mut(label) else {
        return false;
    };
    session.payload.fit = fit;
    true
}

/// Number of output windows currently open.
pub fn open_count(app: &AppHandle) -> usize {
    app.state::<RendererState>().labels().len()
}

/// Re-exports the media helpers callers need (URL parsing, kind detection).
pub use media::{kind_from_path, mime_for};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_unique_labels_from_device_names() {
        assert_eq!(label_for(r"\\.\DISPLAY1", 1), "output-DISPLAY1-1");
        assert_eq!(label_for(r"\\.\DISPLAY2", 3), "output-DISPLAY2-3");
        assert_eq!(label_for("no-backslashes", 0), "output-no-backslashes-0");
    }

    #[test]
    fn label_contains_only_safe_characters() {
        let label = label_for(r"\\.\DISPLAY3", 42);
        for ch in label.chars() {
            assert!(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
        }
    }
}
