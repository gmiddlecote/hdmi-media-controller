//! Windows implementation of display enumeration via the Win32
//! `EnumDisplayDevices` API (user32).
//!
//! The Win32 declarations are provided with raw FFI on purpose: this keeps
//! the dependency tree small (no `windows` crate needed for one function).

use super::model::{DisplayInfo, StateFlags, EDD_GET_DEVICE_INTERFACE_NAME};
use super::DisplayError;

/// `DISPLAY_DEVICE` and `DEVMODE` device name buffer sizes (WCHAR count).
const DISPLAY_DEVICE_NAME_LEN: usize = 32;
const DISPLAY_DEVICE_STRING_LEN: usize = 128;
const DISPLAY_DEVICE_ID_LEN: usize = 128;
const DISPLAY_DEVICE_KEY_LEN: usize = 128;

/// Mirrors the Win32 `DISPLAY_DEVICE` structure.
#[repr(C)]
struct DisplayDevice {
    cb: u32,
    device_name: [u16; DISPLAY_DEVICE_NAME_LEN],
    device_string: [u16; DISPLAY_DEVICE_STRING_LEN],
    state_flags: u32,
    device_id: [u16; DISPLAY_DEVICE_ID_LEN],
    device_key: [u16; DISPLAY_DEVICE_KEY_LEN],
}

impl DisplayDevice {
    fn new() -> Self {
        Self {
            cb: std::mem::size_of::<Self>() as u32,
            device_name: [0; DISPLAY_DEVICE_NAME_LEN],
            device_string: [0; DISPLAY_DEVICE_STRING_LEN],
            state_flags: 0,
            device_id: [0; DISPLAY_DEVICE_ID_LEN],
            device_key: [0; DISPLAY_DEVICE_KEY_LEN],
        }
    }
}

/// Decodes a null-terminated UTF-16 buffer into a `String`.
fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&unit| unit == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

unsafe extern "system" {
    fn EnumDisplayDevicesW(
        lp_device: *const u16,
        i_dev_num: u32,
        lp_display_device: *mut DisplayDevice,
        dw_flags: u32,
    ) -> i32;
}

/// Enumerates the displays attached to the Windows desktop.
///
/// Two-level enumeration: display adapters (which own the `\\.\DISPLAYN`
/// device names) are queried first; every adapter is then queried for the
/// monitor it drives. Mirroring drivers and devices that are not part of
/// the desktop are filtered out.
pub fn list_displays() -> Result<Vec<DisplayInfo>, DisplayError> {
    let mut displays = Vec::new();

    let mut adapter_index = 0u32;
    loop {
        let mut adapter = DisplayDevice::new();
        let adapter_found =
            unsafe { EnumDisplayDevicesW(std::ptr::null(), adapter_index, &mut adapter, 0) };
        if adapter_found == 0 {
            break;
        }

        if StateFlags(adapter.state_flags).is_mirroring() {
            adapter_index += 1;
            continue;
        }

        let mut monitor_index = 0u32;
        loop {
            let mut monitor = DisplayDevice::new();
            let monitor_found = unsafe {
                EnumDisplayDevicesW(
                    adapter.device_name.as_ptr(),
                    monitor_index,
                    &mut monitor,
                    EDD_GET_DEVICE_INTERFACE_NAME,
                )
            };
            if monitor_found == 0 {
                break;
            }

            let flags = StateFlags(monitor.state_flags);
            if flags.attached_to_desktop() {
                displays.push(DisplayInfo::from_device(
                    wide_to_string(&monitor.device_id),
                    wide_to_string(&adapter.device_name),
                    wide_to_string(&monitor.device_string),
                    flags,
                ));
            }

            monitor_index += 1;
        }

        adapter_index += 1;
    }

    Ok(displays)
}
