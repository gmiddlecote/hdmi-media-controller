//! Monitor identification from EDID and PnP hardware ids.
//!
//! `EnumDisplayDevicesW` only reports the monitor's consumer name, which is
//! "Generic PnP Monitor" for laptop panels and for any monitor whose EDID
//! carries no name. Windows caches the monitor's raw EDID blob in the device
//! enum key, so that is used instead: it holds the real model string
//! ("B160UAN08.1", "HP E233") and, failing that, the manufacturer and product
//! code that the PnP hardware id is built from ("AUO9BB0" -> "AUO 9BB0").

/// Registry value holding the raw EDID blob under a device's `Device Parameters`.
pub const EDID_VALUE: &str = "EDID";

/// Tag byte that introduces a monitor-name descriptor.
const NAME_DESCRIPTOR_TAG: u8 = 0xFC;

/// Byte where the base block's four 18-byte descriptors start.
const FIRST_DESCRIPTOR_OFFSET: usize = 54;

/// Length of the EDID base block; descriptors cannot start past its end.
const BASE_BLOCK_LEN: usize = 128;

/// Shortest run of text accepted as a monitor name.
const MIN_NAME_LEN: usize = 4;

/// Longest name an EDID text descriptor can hold.
const MAX_NAME_LEN: usize = 13;

fn is_printable(byte: u8) -> bool {
    (0x20..0x7F).contains(&byte)
}

fn has_standard_header(edid: &[u8]) -> bool {
    edid.len() >= BASE_BLOCK_LEN && edid[..8] == [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]
}

fn name_after_tag(edid: &[u8], tag: usize) -> Option<String> {
    for skip in 1..=2 {
        let start = tag + skip;
        if start >= edid.len() {
            break;
        }
        let mut text = String::new();
        for &byte in &edid[start..] {
            if !is_printable(byte) {
                break;
            }
            text.push(byte as char);
            if text.len() >= MAX_NAME_LEN {
                break;
            }
        }
        let trimmed = text.trim();
        if trimmed.len() >= MIN_NAME_LEN {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Reads the monitor model name from a raw EDID blob.
///
/// The scan is deliberately tolerant: spec-conformant EDID puts the text
/// directly after the `0xFC` tag, but several laptop panels shift the whole
/// descriptor and leave a padding byte, which would hide the name from a
/// fixed-offset parser.
pub fn monitor_name(edid: &[u8]) -> Option<String> {
    if !has_standard_header(edid) {
        return None;
    }
    for index in FIRST_DESCRIPTOR_OFFSET..BASE_BLOCK_LEN {
        if edid[index] == NAME_DESCRIPTOR_TAG {
            if let Some(name) = name_after_tag(edid, index) {
                return Some(name);
            }
        }
    }
    None
}

/// Splits a monitor device path into its segments.
///
/// `EnumDisplayDevicesW` returns the device interface form
/// `\\?\DISPLAY#AUO9BB0#5&...#{e6f07b5f-...}`, where the enum key segments are
/// separated by `#`; other Win32 calls use plain backslashes, so both count.
fn device_path_segments(device_path: &str) -> impl Iterator<Item = &str> {
    let trimmed = device_path.strip_prefix(r"\\?\").unwrap_or(device_path);
    trimmed.split(['\\', '#']).filter(|part| !part.is_empty())
}

/// Registry key a monitor's EDID is stored under:
/// `\\?\DISPLAY#AUO9BB0#5&...#{...}` -> `DISPLAY\AUO9BB0\5&...`.
pub fn enum_key_path(device_path: &str) -> Option<String> {
    let mut segments = device_path_segments(device_path);
    let kind = segments.next()?;
    let hardware = segments.next()?;
    let instance = segments.next()?;
    // A trailing interface GUID means there is no enum instance to read.
    if instance.starts_with('{') {
        return None;
    }
    Some(format!("{kind}\\{hardware}\\{instance}"))
}

/// PnP hardware id of a monitor, e.g. `AUO9BB0` for `\\?\DISPLAY#AUO9BB0#...`.
pub fn hardware_id(device_path: &str) -> Option<&str> {
    let mut segments = device_path_segments(device_path);
    segments.next()?;
    let hardware = segments.next()?;
    (!hardware.is_empty()).then_some(hardware)
}

/// Renders a PnP hardware id as a readable manufacturer + product label,
/// e.g. `AUO9BB0` -> `AUO 9BB0`.
pub fn hardware_label(hardware_id: &str) -> Option<String> {
    if hardware_id.len() < MIN_NAME_LEN + 3 {
        return None;
    }
    let (manufacturer, product) = hardware_id.split_at(3);
    if !manufacturer.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    if !product.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("{manufacturer} {product}"))
}

/// True when a Win32 device string says nothing about the monitor.
pub fn is_generic_label(label: &str) -> bool {
    let normalized = label.trim().to_ascii_lowercase();
    normalized.is_empty()
        || normalized == "generic pnp monitor"
        || normalized == "generic monitor"
        || normalized == "generic non-monitor"
        || normalized == "microsoft basic display adapter"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a 128-byte EDID with `0xFC` at `tag` and `text` following it.
    fn edid_with_name_at(tag: usize, padding: usize, text: &str) -> Vec<u8> {
        let mut edid = vec![0u8; BASE_BLOCK_LEN];
        edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        edid[tag] = NAME_DESCRIPTOR_TAG;
        let start = tag + 1 + padding;
        for (offset, byte) in text.bytes().enumerate() {
            edid[start + offset] = byte;
        }
        edid[start + text.len()] = b'\n';
        edid
    }

    #[test]
    fn reads_name_from_conformant_edid() {
        let edid = edid_with_name_at(54 + 5, 0, "B160UAN08.1");
        assert_eq!(monitor_name(&edid).as_deref(), Some("B160UAN08.1"));
    }

    #[test]
    fn reads_name_from_shifted_edid() {
        // Laptop panels in the wild put the tag at descriptor offset +3 and
        // leave a padding byte before the text.
        let edid = edid_with_name_at(111, 1, "B160UAN08.1");
        assert_eq!(monitor_name(&edid).as_deref(), Some("B160UAN08.1"));
    }

    #[test]
    fn trims_padding_spaces() {
        let edid = edid_with_name_at(54 + 5, 0, "HP E233     ");
        assert_eq!(monitor_name(&edid).as_deref(), Some("HP E233"));
    }

    #[test]
    fn ignores_edid_without_a_name_descriptor() {
        let mut edid = vec![0u8; BASE_BLOCK_LEN];
        edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        edid[54] = 0xFD;
        assert_eq!(monitor_name(&edid), None);
    }

    #[test]
    fn rejects_data_without_the_standard_header() {
        assert_eq!(monitor_name(&[0x42; BASE_BLOCK_LEN]), None);
    }

    #[test]
    fn rejects_short_and_garbage_names() {
        let edid = edid_with_name_at(54 + 5, 0, "ab");
        assert_eq!(monitor_name(&edid), None);
    }

    #[test]
    fn derives_enum_key_path() {
        assert_eq!(
            enum_key_path(r"\\?\DISPLAY\AUO9BB0\5&e37b56d&0&UID4355").as_deref(),
            Some(r"DISPLAY\AUO9BB0\5&e37b56d&0&UID4355")
        );
    }

    #[test]
    fn derives_enum_key_path_from_the_device_interface_form() {
        // This is the shape EnumDisplayDevicesW actually hands back.
        assert_eq!(
            enum_key_path(
                r"\\?\DISPLAY#AUO9BB0#5&e37b56d&0&UID4355#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
            )
            .as_deref(),
            Some(r"DISPLAY\AUO9BB0\5&e37b56d&0&UID4355")
        );
    }

    #[test]
    fn derives_hardware_id() {
        assert_eq!(
            hardware_id(r"\\?\DISPLAY\AUO9BB0\5&e37b56d&0&UID4355"),
            Some("AUO9BB0")
        );
        assert_eq!(
            hardware_id(r"\\?\DISPLAY#HPN3460#5&e37b56d&0&UID4354#{e6f07b5f-ee97-4a90-b0}"),
            Some("HPN3460")
        );
    }

    #[test]
    fn formats_hardware_labels() {
        assert_eq!(hardware_label("AUO9BB0").as_deref(), Some("AUO 9BB0"));
        assert_eq!(hardware_label("SNY07F2").as_deref(), Some("SNY 07F2"));
    }

    #[test]
    fn rejects_hardware_ids_that_are_not_codes() {
        assert_eq!(hardware_label("Display1"), None);
        assert_eq!(hardware_label("AU9"), None);
        assert_eq!(hardware_label(""), None);
    }

    #[test]
    fn flags_uninformative_device_strings() {
        assert!(is_generic_label("Generic PnP Monitor"));
        assert!(is_generic_label("  generic monitor "));
        assert!(is_generic_label(""));
        assert!(!is_generic_label("LG TV"));
    }
}
