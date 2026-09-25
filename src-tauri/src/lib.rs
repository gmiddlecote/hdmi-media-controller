//! HDMI Media Controller — backend entry point.
//!
//! The application is split into independent modules so that display
//! management, media playback, playlists, scheduling, and output rendering
//! can be developed in parallel:
//!
//! - [`display`] — enumerates and manages Windows displays.
//! - [`media`] — loads and serves local image/video files.
//! - [`playlist`] — ordered playback of media files.
//! - [`scheduler`] — time- and event-based playback scheduling.
//! - [`renderer`] — fullscreen/borderless output on the selected display.

pub mod display;
pub mod logging;
pub mod media;
pub mod playlist;
pub mod renderer;
pub mod scheduler;

use std::fs;
use std::path::Path;

use display::model::{DisplayInfo, DisplayMode, DisplayModes};
use media::{MediaItem, ObjectFit};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

#[cfg(target_os = "windows")]
use std::time::Duration;

/// Returns the displays detected on the current system, including their
/// connector type (e.g. HDMI) when the display config can be queried.
#[tauri::command]
fn list_displays() -> Result<Vec<DisplayInfo>, String> {
    display::list_displays().map_err(|err| err.to_string())
}

/// Returns the modes supported by a display, plus the mode in effect now.
#[tauri::command]
fn get_display_modes(device_name: String) -> Result<DisplayModes, String> {
    display::get_display_modes(&device_name).map_err(|err| err.to_string())
}

/// Applies the requested resolution/refresh rate to a display.
#[tauri::command]
fn set_display_mode(
    device_name: String,
    width: u32,
    height: u32,
    refresh_rate: u32,
) -> Result<(), String> {
    let mode = DisplayMode {
        width,
        height,
        refresh_rate,
        bit_depth: 0,
    };
    display::set_display_mode(&device_name, &mode).map_err(|err| err.to_string())
}

/// A single entry in a directory listing.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryEntry {
    name: String,
    path: String,
    kind: EntryKind,
}

/// Coarse type of a directory entry, for the browsing UI.
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum EntryKind {
    Dir,
    Image,
    Video,
    Other,
}

/// Lists the contents of a directory so the UI can browse media files.
/// Folders sort first, then files alphabetically; hidden files are skipped.
#[tauri::command]
fn list_directory(directory: String) -> Result<Vec<DirectoryEntry>, String> {
    let canonical = Path::new(&directory)
        .canonicalize()
        .map_err(|_| format!("Directory not found: {directory}"))?;
    if canonical.is_dir() {
        let mut entries: Vec<DirectoryEntry> = Vec::new();
        for entry in fs::read_dir(&canonical).map_err(|err| err.to_string())? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let kind = if path.is_dir() {
                EntryKind::Dir
            } else {
                match media::classify(&path.to_string_lossy()) {
                    Some(media::MediaKind::Image) => EntryKind::Image,
                    Some(media::MediaKind::Video) => EntryKind::Video,
                    None => EntryKind::Other,
                }
            };
            entries.push(DirectoryEntry {
                name,
                path: path.to_string_lossy().into_owned(),
                kind,
            });
        }
        entries.sort_by(|a, b| match (&a.kind, &b.kind) {
            (EntryKind::Dir, _) => std::cmp::Ordering::Less,
            (_, EntryKind::Dir) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });
        Ok(entries)
    } else {
        Err(format!("Not a directory: {directory}"))
    }
}

/// One queued item as supplied by the UI: a path, its fit mode, and the
/// display it should play on (empty to follow the master display).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistEntry {
    path: String,
    #[serde(default)]
    fit: ObjectFit,
    #[serde(default)]
    display: String,
}

/// Replaces the playback queue from a list of file paths. Files that do not
/// exist (or are directories) are skipped. Returns how many were queued.
#[tauri::command]
fn scheduler_set_playlist(
    state: tauri::State<'_, scheduler::State>,
    entries: Vec<PlaylistEntry>,
) -> Result<usize, String> {
    let items: Vec<MediaItem> = entries
        .iter()
        .filter_map(|entry| {
            media::canonical_path(Path::new(&entry.path)).map(|path| {
                let mut item = MediaItem::from_path(path.to_string_lossy().into_owned());
                item.fit = entry.fit;
                item.display = entry.display.clone();
                item
            })
        })
        .collect();
    state.send(scheduler::Command::SetPlaylist(items.clone()))?;
    Ok(items.len())
}

/// Starts stepping through the playlist on the named display.
#[tauri::command]
fn scheduler_play(
    state: tauri::State<'_, scheduler::State>,
    device_name: String,
) -> Result<(), String> {
    state.send(scheduler::Command::Play {
        display: device_name,
    })
}

/// Plays one specific file on the named display with the requested mode
/// (`once`, `loop`, or `timed`). `seconds` applies to `timed`.
#[tauri::command]
fn scheduler_play_item(
    state: tauri::State<'_, scheduler::State>,
    path: String,
    device_name: String,
    mode: scheduler::PlayMode,
    seconds: u64,
    fit: ObjectFit,
) -> Result<(), String> {
    let canonical = media::canonical_path(Path::new(&path))
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or_else(|| format!("File not found: {path}"))?;
    state.send(scheduler::Command::PlayItem {
        path: canonical,
        display: device_name,
        mode,
        seconds,
        fit,
    })
}

/// Registers a file and returns its `media://` url, so the queue can show
/// live thumbnails. Existing files reuse their id.
#[tauri::command]
fn media_register(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let canonical = media::canonical_path(Path::new(&path))
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or_else(|| format!("File not found: {path}"))?;
    let item = MediaItem::from_path(canonical);
    Ok(app.state::<media::Registry>().register(item))
}

/// Returns diagnostics for a `media://` URL so a failing renderer can show
/// whether the id parsed, the file is registered, and is present on disk.
#[tauri::command]
fn media_probe(app: tauri::AppHandle, url: String) -> media::MediaProbe {
    media::probe(&url, &app.state::<media::Registry>())
}

/// Stops playback and closes the output window.
#[tauri::command]
fn scheduler_stop(state: tauri::State<'_, scheduler::State>) -> Result<(), String> {
    state.send(scheduler::Command::Stop)
}

#[tauri::command]
fn scheduler_pause(state: tauri::State<'_, scheduler::State>) -> Result<(), String> {
    state.send(scheduler::Command::Pause)
}

#[tauri::command]
fn scheduler_resume(state: tauri::State<'_, scheduler::State>) -> Result<(), String> {
    state.send(scheduler::Command::Resume)
}

#[tauri::command]
fn scheduler_next(state: tauri::State<'_, scheduler::State>) -> Result<(), String> {
    state.send(scheduler::Command::Next)
}

#[tauri::command]
fn scheduler_prev(state: tauri::State<'_, scheduler::State>) -> Result<(), String> {
    state.send(scheduler::Command::Prev)
}

/// Sets how long a still image stays on screen, in milliseconds.
#[tauri::command]
fn scheduler_set_dwell(
    state: tauri::State<'_, scheduler::State>,
    millis: u64,
) -> Result<(), String> {
    state.set_dwell_ms(millis);
    Ok(())
}

/// Sets the caption rendered over output windows. Existing outputs update
/// immediately; new ones use it from the start. Empty clears the caption.
#[tauri::command]
fn scheduler_set_overlay(
    app: tauri::AppHandle,
    state: tauri::State<'_, scheduler::State>,
    text: String,
) -> Result<(), String> {
    state.set_overlay(text);
    for label in app.state::<renderer::RendererState>().labels() {
        let _ = app.emit_to(label, "overlay-text", state.overlay());
    }
    Ok(())
}

/// Sets the fit mode (`cover` or `contain`) for an item, both in the queued
/// playlist and, when it is currently on screen, on the live output window.
#[tauri::command]
fn scheduler_set_fit(
    state: tauri::State<'_, scheduler::State>,
    path: String,
    fit: ObjectFit,
) -> Result<(), String> {
    state.send(scheduler::Command::SetFit { path, fit })
}

/// Returns the current playback status for the control UI.
#[tauri::command]
fn scheduler_status(state: tauri::State<'_, scheduler::State>) -> scheduler::Snapshot {
    state.snapshot()
}

/// Returns the media payload an output window should render.
///
/// Called by `render.html` with its own window label.
#[tauri::command]
fn log_js_error(message: String) {
    logging::error(&message);
}

#[tauri::command]
fn renderer_render_token(
    app: tauri::AppHandle,
    label: String,
) -> Result<renderer::RenderPayload, String> {
    app.state::<renderer::RendererState>()
        .payload(&label)
        .ok_or_else(|| "No active output session for this window.".into())
}

/// Closes the output window with the given label.
#[tauri::command]
fn renderer_close(app: tauri::AppHandle, label: String) -> Result<(), String> {
    if renderer::close(&app, &label) {
        Ok(())
    } else {
        Err("No active output session with that label.".into())
    }
}

/// Closes every open output window.
#[tauri::command]
fn renderer_close_all(app: tauri::AppHandle) -> Result<usize, String> {
    Ok(renderer::close_all(&app))
}

/// Polls the connected-display set and emits a `displays-changed` Tauri
/// event when it changes, so the UI re-enumerates on hotplug.
#[cfg(target_os = "windows")]
fn begin_display_watch(app: tauri::AppHandle) {
    use tauri::Emitter;

    std::thread::spawn(move || {
        let mut previous: Vec<String> = Vec::new();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            match display::list_displays() {
                Ok(displays) => {
                    let current = display::hotplug::display_ids(&displays);
                    if display::hotplug::snapshots_differ(&previous, &current) {
                        previous = current;
                        let _ = app.emit("displays-changed", ());
                    }
                }
                Err(_) => previous.clear(),
            }
        }
    });
}

pub fn run() {
    logging::init();
    let (command_tx, command_rx) = std::sync::mpsc::channel::<scheduler::Command>();
    let scheduler_state = scheduler::State::new(command_tx);

    logging::info("initialising tauri builder");
    let result = tauri::Builder::default()
        .manage(media::Registry::default())
        .manage(renderer::RendererState::default())
        .manage(scheduler_state.clone())
        .setup(move |app| -> Result<(), Box<dyn std::error::Error>> {
            #[cfg(target_os = "windows")]
            begin_display_watch(app.handle().clone());
            scheduler::spawn(app.handle().clone(), command_rx, scheduler_state.clone());
            Ok(())
        })
        .register_uri_scheme_protocol("media", media::serve)
        .invoke_handler(tauri::generate_handler![
            list_displays,
            list_directory,
            get_display_modes,
            set_display_mode,
            scheduler_set_playlist,
            scheduler_play,
            scheduler_play_item,
            media_register,
            media_probe,
            log_js_error,
            scheduler_stop,
            scheduler_pause,
            scheduler_resume,
            scheduler_next,
            scheduler_prev,
            scheduler_set_dwell,
            scheduler_set_overlay,
            scheduler_set_fit,
            scheduler_status,
            renderer_render_token,
            renderer_close,
            renderer_close_all,
        ])
        .run(tauri::generate_context!());

    match result {
        Ok(()) => logging::info("application exited normally"),
        Err(e) => {
            let msg = format!("tauri application error: {e}");
            logging::error(&msg);
            eprintln!("{msg}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_directories_before_files_and_skips_hidden() {
        let dir = std::env::temp_dir().join(format!("hmc-browse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join(".hidden.png"), "x").unwrap();
        std::fs::write(dir.join("b.txt"), "x").unwrap();
        std::fs::write(dir.join("a.jpg"), "x").unwrap();
        std::fs::write(dir.join("clip.mp4"), "x").unwrap();

        let entries = list_directory(dir.to_string_lossy().into_owned()).unwrap();
        let kinds: Vec<(String, String)> = entries
            .iter()
            .map(|entry| {
                let kind = serde_json::to_value(&entry.kind)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string();
                (entry.name.clone(), kind)
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("sub".into(), "dir".into()),
                ("a.jpg".into(), "image".into()),
                ("b.txt".into(), "other".into()),
                ("clip.mp4".into(), "video".into()),
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_missing_or_non_directory_paths() {
        let missing =
            std::env::temp_dir().join(format!("hmc-browse-missing-{}", std::process::id()));
        assert!(list_directory(missing.to_string_lossy().into_owned()).is_err());

        let file = std::env::temp_dir().join(format!("hmc-browse-file-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        assert!(list_directory(file.to_string_lossy().into_owned()).is_err());
        let _ = std::fs::remove_file(&file);
    }
}
