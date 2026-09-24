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

use display::model::DisplayInfo;

/// Returns the displays detected on the current system.
///
/// This first implementation surfaces the native Windows display
/// enumeration as a Tauri command so the control UI can render the list.
#[tauri::command]
fn list_displays() -> Result<Vec<DisplayInfo>, String> {
    display::list_displays().map_err(|err| err.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list_displays])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
