// Prevents an additional console window on Windows in release builds.
// DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    hdmi_media_controller_lib::run()
}
