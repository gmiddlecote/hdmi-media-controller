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
pub mod media;
pub mod playlist;
pub mod renderer;
pub mod scheduler;

use std::path::Path;

use display::model::{DisplayInfo, DisplayMode, DisplayModes};
use media::MediaItem;
use tauri::Manager;

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

/// Replaces the playback queue from a list of file paths. Files that do not
/// exist (or are directories) are skipped. Returns how many were queued.
#[tauri::command]
fn scheduler_set_playlist(
    state: tauri::State<'_, scheduler::State>,
    paths: Vec<String>,
) -> Result<usize, String> {
    let items: Vec<MediaItem> = paths
        .iter()
        .filter_map(|path| {
            media::canonical_path(Path::new(path))
                .map(|path| MediaItem::from_path(path.to_string_lossy().into_owned()))
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
) -> Result<(), String> {
    let canonical = media::canonical_path(Path::new(&path))
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or_else(|| format!("File not found: {path}"))?;
    state.send(scheduler::Command::PlayItem {
        path: canonical,
        display: device_name,
        mode,
        seconds,
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

/// Returns the current playback status for the control UI.
#[tauri::command]
fn scheduler_status(state: tauri::State<'_, scheduler::State>) -> scheduler::Snapshot {
    state.snapshot()
}

/// Returns the media payload an output window should render.
///
/// Called by `render.html` with its own window label.
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
    let (command_tx, command_rx) = std::sync::mpsc::channel::<scheduler::Command>();
    let scheduler_state = scheduler::State::new(command_tx);

    tauri::Builder::default()
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
            get_display_modes,
            set_display_mode,
            scheduler_set_playlist,
            scheduler_play,
            scheduler_play_item,
            media_register,
            scheduler_stop,
            scheduler_pause,
            scheduler_resume,
            scheduler_next,
            scheduler_prev,
            scheduler_set_dwell,
            scheduler_status,
            renderer_render_token,
            renderer_close,
            renderer_close_all,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
