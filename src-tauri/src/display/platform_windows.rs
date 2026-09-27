//! Windows implementation of display management via the Win32 API.
//!
//! Uses the lightweight `windows-sys` FFI bindings (declarations only, no
//! runtime), which keeps the struct layouts (DEVMODE, DISPLAYCONFIG_*)
//! correct without hand-maintaining them. `DISPLAY_DEVICE_*` flag
//! constants are taken from our own model because the `windows-sys` copies
//! of a few of them disagree with winuser.h.
//!
//! - Display list: two-level `EnumDisplayDevices` enumeration.
//! - Connector type: `QueryDisplayConfig` (`DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY`),
//!   merged back into the display list by matching the monitor device path.
//! - Resolution list: `EnumDisplaySettingsEx`, current setting included.
//! - Resolution selection: `ChangeDisplaySettingsEx` (test, then apply).

use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::ptr;

use windows_sys::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    QDC_ALL_PATHS, QDC_ONLY_ACTIVE_PATHS,
};
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, POINTL};
use windows_sys::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsExW, CDS_TEST,
    CDS_UPDATEREGISTRY, DEVMODEW, DISPLAY_DEVICEW, DISP_CHANGE_BADMODE, DISP_CHANGE_FAILED,
    DISP_CHANGE_RESTART, DISP_CHANGE_SUCCESSFUL, DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH,
    ENUM_CURRENT_SETTINGS,
};

use super::model::{
    output_technology_label, DisplayBounds, DisplayInfo, DisplayMode, DisplayModes, StateFlags,
    DISPLAY_DEVICE_MIRRORING_DRIVER, EDD_GET_DEVICE_INTERFACE_NAME,
};
use super::DisplayError;

/// Encodes a Rust string as a null-terminated UTF-16 buffer for Win32 calls.
fn to_utf16(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Collects monitor information for a given display adapter.
fn collect_monitors_for_adapter(
    adapter: &DISPLAY_DEVICEW,
    connections: &HashMap<String, String>,
) -> Vec<DisplayInfo> {
    let mut displays = Vec::new();
    let adapter_name = wide_to_string(&adapter.DeviceName);
    let mut monitor_index = 0u32;
    loop {
        let mut monitor = zeroed_device();
        let monitor_found = unsafe {
            EnumDisplayDevicesW(
                adapter.DeviceName.as_ptr(),
                monitor_index,
                &mut monitor,
                EDD_GET_DEVICE_INTERFACE_NAME,
            )
        };
        if monitor_found == 0 {
            break;
        }

        let flags = StateFlags(monitor.StateFlags);
        if flags.attached_to_desktop() {
            let device_path = wide_to_string(&monitor.DeviceID);
            let connection_kind = connections.get(&device_path).cloned();
            let bounds = display_bounds(&adapter.DeviceName).unwrap_or_default();
            displays.push(DisplayInfo::from_device(
                device_path,
                adapter_name.clone(),
                wide_to_string(&monitor.DeviceString),
                flags,
                connection_kind,
                bounds,
            ));
        }

        monitor_index += 1;
    }
    displays
}

/// Decodes a null-terminated UTF-16 buffer into a `String`.
fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&unit| unit == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Enumerates the displays attached to the Windows desktop.
///
/// Display adapters (which own the `\\.\DISPLAYN` device names) are queried
/// first; every adapter is then queried for the monitor it drives.
/// Mirroring drivers and devices that are not part of the desktop are
/// filtered out. Connector types come from the display config.
pub fn list_displays() -> Result<Vec<DisplayInfo>, DisplayError> {
    let connections = connection_kind_by_monitor_path();

    let mut displays = Vec::new();
    let mut adapter_index = 0u32;
    loop {
        let mut adapter = zeroed_device();
        let adapter_found =
            unsafe { EnumDisplayDevicesW(ptr::null(), adapter_index, &mut adapter, 0) };
        if adapter_found == 0 {
            break;
        }

        if adapter.StateFlags & DISPLAY_DEVICE_MIRRORING_DRIVER != 0 {
            adapter_index += 1;
            continue;
        }

        displays.extend(collect_monitors_for_adapter(&adapter, &connections));

        adapter_index += 1;
    }

    Ok(displays)
}

/// Returns the modes supported by a display plus the mode in use now.
pub fn get_display_modes(device_name: &str) -> Result<DisplayModes, DisplayError> {
    let device = to_utf16(device_name);

    let current = enum_mode(&device, ENUM_CURRENT_SETTINGS)
        .ok_or_else(|| DisplayError::Settings("no current setting available".into()))?;

    let mut modes = Vec::new();
    let mut seen = HashSet::new();
    for index in 0u32.. {
        let Some(mode) = enum_mode(&device, index) else {
            break;
        };
        if seen.insert(mode) {
            modes.push(mode);
        }
    }

    if seen.insert(current) {
        modes.push(current);
    }

    // Sorted smallest → largest by pixel count, higher refresh rate first.
    modes.sort_by(|a, b| {
        (a.width * a.height, b.refresh_rate).cmp(&(b.width * b.height, a.refresh_rate))
    });

    Ok(DisplayModes { current, modes })
}

/// Applies a display mode to the named device, updating the registry.
pub fn set_display_mode(device_name: &str, mode: &DisplayMode) -> Result<(), DisplayError> {
    let supported = get_display_modes(device_name)?;
    let supported = supported.modes.iter().any(|m| {
        m.width == mode.width && m.height == mode.height && m.refresh_rate == mode.refresh_rate
    });
    if !supported {
        return Err(DisplayError::Settings(format!(
            "unsupported mode: {}",
            mode.label()
        )));
    }

    let mut devmode = zeroed_devmode();
    devmode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
    devmode.dmPelsWidth = mode.width;
    devmode.dmPelsHeight = mode.height;
    devmode.dmDisplayFrequency = mode.refresh_rate;

    let device = to_utf16(device_name);

    let tested = unsafe {
        ChangeDisplaySettingsExW(
            device.as_ptr(),
            &devmode,
            ptr::null_mut(),
            CDS_UPDATEREGISTRY | CDS_TEST,
            ptr::null(),
        )
    };
    if tested != DISP_CHANGE_SUCCESSFUL {
        return Err(display_change_error(tested, mode));
    }

    let applied = unsafe {
        ChangeDisplaySettingsExW(
            device.as_ptr(),
            &devmode,
            ptr::null_mut(),
            CDS_UPDATEREGISTRY,
            ptr::null(),
        )
    };
    match applied {
        DISP_CHANGE_SUCCESSFUL => Ok(()),
        DISP_CHANGE_RESTART => Err(DisplayError::Settings(
            "mode change requires a restart to take effect".into(),
        )),
        _ => Err(display_change_error(applied, mode)),
    }
}

fn display_change_error(code: i32, mode: &DisplayMode) -> DisplayError {
    let detail = match code {
        DISP_CHANGE_BADMODE => "the mode is not supported by the display".to_string(),
        DISP_CHANGE_FAILED => "the display driver rejected the mode change".to_string(),
        DISP_CHANGE_RESTART => "the mode change requires a restart".to_string(),
        other => format!("ChangeDisplaySettingsEx failed (code {other})"),
    };
    DisplayError::Settings(format!("could not apply {}: {detail}", mode.label()))
}

/// Queries a display mode via `EnumDisplaySettingsEx` (either the current
/// setting or an indexed enumerative mode).
fn enum_mode(device: &[u16], index: u32) -> Option<DisplayMode> {
    let mut devmode = zeroed_devmode();
    let ok = unsafe { EnumDisplaySettingsExW(device.as_ptr(), index, &mut devmode, 0) };
    if ok == 0 {
        return None;
    }
    Some(DisplayMode {
        width: devmode.dmPelsWidth,
        height: devmode.dmPelsHeight,
        refresh_rate: devmode.dmDisplayFrequency,
        bit_depth: devmode.dmBitsPerPel,
    })
}

/// Returns the on-screen rectangle of a display (virtual desktop
/// coordinates) via the position and pixel size of its current mode.
fn display_bounds(device: &[u16]) -> Option<DisplayBounds> {
    let mut devmode = zeroed_devmode();
    let ok =
        unsafe { EnumDisplaySettingsExW(device.as_ptr(), ENUM_CURRENT_SETTINGS, &mut devmode, 0) };
    if ok == 0 {
        return None;
    }
    let position: POINTL = unsafe { devmode.Anonymous1.Anonymous2.dmPosition };
    Some(DisplayBounds {
        x: position.x,
        y: position.y,
        width: devmode.dmPelsWidth,
        height: devmode.dmPelsHeight,
    })
}

/// Maps monitor device-interface paths to connector labels using
/// `QueryDisplayConfig`. Returns an empty map when the display config
/// cannot be queried (the app degrades to a connection-free display list).
fn connection_kind_by_monitor_path() -> HashMap<String, String> {
    let mut kinds = HashMap::new();

    let Some(paths) = query_active_paths() else {
        return kinds;
    };

    for path in &paths {
        if path.targetInfo.targetAvailable == 0 {
            continue;
        }

        let mut name = unsafe { std::mem::zeroed::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() };
        name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        name.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        name.header.adapterId = path.targetInfo.adapterId;
        name.header.id = path.targetInfo.id;

        let ok = unsafe {
            DisplayConfigGetDeviceInfo(&mut name as *mut _ as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER)
        };
        if ok != ERROR_SUCCESS as i32 {
            continue;
        }

        let device_path = wide_to_string(&name.monitorDevicePath);
        if device_path.is_empty() {
            continue;
        }

        if let Some(label) = output_technology_label(name.outputTechnology) {
            kinds.insert(device_path, label.to_string());
        }
    }

    kinds
}

/// Returns the active display-path array from `QueryDisplayConfig`, retrying
/// up to three times when the buffer was too small (config changed).
fn query_active_paths() -> Option<Vec<DISPLAYCONFIG_PATH_INFO>> {
    let mut path_capacity = 0u32;
    let mut mode_capacity = 0u32;
    if unsafe { GetDisplayConfigBufferSizes(QDC_ALL_PATHS, &mut path_capacity, &mut mode_capacity) }
        != ERROR_SUCCESS
    {
        return None;
    }

    for _attempt in 0..3 {
        let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = (0..path_capacity)
            .map(|_| unsafe { std::mem::zeroed() })
            .collect();
        let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = (0..mode_capacity)
            .map(|_| unsafe { std::mem::zeroed() })
            .collect();
        let mut topology = 0i32;

        let result = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_capacity,
                paths.as_mut_ptr(),
                &mut mode_capacity,
                modes.as_mut_ptr(),
                &mut topology,
            )
        };
        match result {
            ERROR_SUCCESS => return Some(paths),
            ERROR_INSUFFICIENT_BUFFER => {
                // Capacity was too small (the config changed); sizes were
                // updated by the call, so retry with the new values.
            }
            _ => return None,
        }
    }

    None
}

fn zeroed_device() -> DISPLAY_DEVICEW {
    let mut device = unsafe { std::mem::zeroed::<DISPLAY_DEVICEW>() };
    device.cb = size_of::<DISPLAY_DEVICEW>() as u32;
    device
}

fn zeroed_devmode() -> DEVMODEW {
    let mut devmode = unsafe { std::mem::zeroed::<DEVMODEW>() };
    devmode.dmSize = size_of::<DEVMODEW>() as u16;
    devmode
}
