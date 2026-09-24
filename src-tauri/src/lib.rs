//! HDMI Media Controller — backend entry point.
//!
//! The application is split into independent modules so that display
//! management, media playback, playlists, scheduling, and output rendering
//! can be developed in parallel:
//!
//! - [`display`] — enumerates and manages Windows displays.
//! - [`media`] — loads and plays local image/video files.
//! - [`playlist`] — ordered playback of media files.
//! - [`scheduler`] — time- and event-based playback scheduling.
//! - [`renderer`] — fullscreen/borderless output on the selected display.

pub mod display;
pub mod media;
pub mod playlist;
pub mod renderer;
pub mod scheduler;

use display::model::{DisplayInfo, DisplayMode, DisplayModes};

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
    tauri::Builder::default()
        .setup(|_app| {
            #[cfg(target_os = "windows")]
            begin_display_watch(_app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_displays,
            get_display_modes,
            set_display_mode
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
