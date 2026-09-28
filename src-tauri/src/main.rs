// Prevents an additional console window on Windows.
// DO NOT REMOVE!!
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    hdmi_media_controller_lib::run()
}
