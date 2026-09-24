//! Data model for detected displays and their capabilities.

use serde::Serialize;

/// Windows-only flags; used by the Windows engine and its unit tests.
/// `allow(dead_code)` keeps the non-Windows build warning-free.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod win32_flags {
    pub(crate) const DISPLAY_DEVICE_ATTACHED_TO_DESKTOP: u32 = 0x0000_0001;
    pub(crate) const DISPLAY_DEVICE_PRIMARY_DEVICE: u32 = 0x0000_0004;
    pub(crate) const DISPLAY_DEVICE_MIRRORING_DRIVER: u32 = 0x0000_0008;
    pub(crate) const DISPLAY_DEVICE_ACTIVE: u32 = 0x0000_4000;
    pub(crate) const EDD_GET_DEVICE_INTERFACE_NAME: u32 = 0x0000_0001;
}
pub(crate) use win32_flags::*;

/// Display connector types as reported by the Windows display config
/// (`DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY`, see wingdi.h).
///
/// Returns a human-readable label or `None` for values we do not recognize.
pub fn output_technology_label(value: i32) -> Option<&'static str> {
    let label = match value {
        -1 => "Other",
        0 => "VGA",
        1 => "S-Video",
        2 => "Composite",
        3 => "Component",
        4 => "DVI",
        5 => "HDMI",
        6 => "LVDS",
        8 => "D (Japanese)",
        9 => "SDI",
        10 => "DisplayPort",
        11 => "Embedded DisplayPort",
        12 => "UDI",
        13 => "Embedded UDI",
        14 => "SDTV Dongle",
        15 => "Miracast",
        16 => "Indirect Wired",
        17 => "Indirect Virtual",
        i32::MIN => "Internal",
        _ => return None,
    };
    Some(label)
}

/// A single display (monitor) detected on the system.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    /// Stable OS identifier for the monitor (device interface path).
    pub id: String,
    /// Win32 device name, e.g. `\\.\DISPLAY1`.
    pub device_name: String,
    /// Human-readable monitor name, e.g. "LG TV", "Generic PnP Monitor".
    pub friendly_name: String,
    /// True when the display is part of the active desktop.
    pub is_attached: bool,
    /// True when the display is currently outputting.
    pub is_active: bool,
    /// True when this is the primary display.
    pub is_primary: bool,
    /// Physical connector type when known (e.g. "HDMI", "DisplayPort",
    /// "Internal"); `None` when the display config could not be queried.
    pub connection_kind: Option<String>,
}

impl DisplayInfo {
    /// Builds a [`DisplayInfo`] from a Win32 `DISPLAY_DEVICE` description.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn from_device(
        id: String,
        device_name: String,
        friendly_name: String,
        flags: StateFlags,
        connection_kind: Option<String>,
    ) -> Self {
        Self {
            id,
            device_name,
            friendly_name,
            is_attached: flags.attached_to_desktop(),
            is_active: flags.is_active(),
            is_primary: flags.is_primary(),
            connection_kind,
        }
    }
}

/// Parsed `DISPLAY_DEVICE.StateFlags` bit field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub struct StateFlags(pub u32);

impl StateFlags {
    pub(crate) fn attached_to_desktop(self) -> bool {
        self.0 & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0
    }

    pub(crate) fn is_active(self) -> bool {
        self.0 & DISPLAY_DEVICE_ACTIVE != 0
    }

    pub(crate) fn is_primary(self) -> bool {
        self.0 & DISPLAY_DEVICE_PRIMARY_DEVICE != 0
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn is_mirroring(self) -> bool {
        self.0 & DISPLAY_DEVICE_MIRRORING_DRIVER != 0
    }
}

/// A supported output resolution (and the current one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayMode {
    /// Horizontal resolution in pixels.
    pub width: u32,
    /// Vertical resolution in pixels.
    pub height: u32,
    /// Refresh rate in Hz.
    pub refresh_rate: u32,
    /// Color depth in bits per pixel.
    pub bit_depth: u32,
}

impl DisplayMode {
    /// A compact label for the UI, e.g. `1920x1080 @ 60 Hz`.
    pub fn label(&self) -> String {
        format!("{}x{} @ {} Hz", self.width, self.height, self.refresh_rate)
    }
}

/// The modes supported by a display, plus the mode in effect right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayModes {
    pub current: DisplayMode,
    pub modes: Vec<DisplayMode>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_attached_to_desktop_flag() {
        let flags = StateFlags(DISPLAY_DEVICE_ATTACHED_TO_DESKTOP);
        assert!(flags.attached_to_desktop());
        assert!(!flags.is_active());
        assert!(!flags.is_primary());
        assert!(!flags.is_mirroring());
    }

    #[test]
    fn parses_active_and_primary_flags() {
        let flags = StateFlags(DISPLAY_DEVICE_ACTIVE | DISPLAY_DEVICE_PRIMARY_DEVICE);
        assert!(flags.is_active());
        assert!(flags.is_primary());
        assert!(!flags.attached_to_desktop());
    }

    #[test]
    fn detects_mirroring_driver() {
        assert!(StateFlags(DISPLAY_DEVICE_MIRRORING_DRIVER).is_mirroring());
    }

    #[test]
    fn maps_connector_types() {
        assert_eq!(output_technology_label(5), Some("HDMI"));
        assert_eq!(output_technology_label(10), Some("DisplayPort"));
        assert_eq!(output_technology_label(4), Some("DVI"));
        assert_eq!(output_technology_label(0), Some("VGA"));
        assert_eq!(output_technology_label(-1), Some("Other"));
        assert_eq!(output_technology_label(i32::MIN), Some("Internal"));
        assert_eq!(output_technology_label(99), None);
    }

    #[test]
    fn mode_label_and_ordering() {
        let mode = DisplayMode {
            width: 1920,
            height: 1080,
            refresh_rate: 60,
            bit_depth: 32,
        };
        assert_eq!(mode.label(), "1920x1080 @ 60 Hz");
    }

    #[test]
    fn display_info_serializes_to_camel_case() {
        let info = DisplayInfo {
            id: "MONITOR\\DEL4096\\{abc}\\0001".into(),
            device_name: r"\\.\DISPLAY1".into(),
            friendly_name: "LG TV".into(),
            is_attached: true,
            is_active: true,
            is_primary: false,
            connection_kind: Some("HDMI".into()),
        };

        let value = serde_json::to_value(&info).expect("serializes");

        assert_eq!(
            value,
            json!({
                "id": "MONITOR\\DEL4096\\{abc}\\0001",
                "deviceName": r"\\.\DISPLAY1",
                "friendlyName": "LG TV",
                "isAttached": true,
                "isActive": true,
                "isPrimary": false,
                "connectionKind": "HDMI",
            })
        );
    }

    #[test]
    fn display_modes_serialize_to_camel_case() {
        let modes = DisplayModes {
            current: DisplayMode {
                width: 1920,
                height: 1080,
                refresh_rate: 60,
                bit_depth: 32,
            },
            modes: vec![DisplayMode {
                width: 1280,
                height: 720,
                refresh_rate: 60,
                bit_depth: 32,
            }],
        };

        let value = serde_json::to_value(&modes).expect("serializes");

        assert_eq!(value["current"]["refreshRate"], 60);
        assert_eq!(value["modes"][0]["width"], 1280);
    }
}
