//! Display hotplug change detection.
//!
//! A polling watcher (see `crate::begin_display_watch`) periodically
//! snapshots the set of connected displays and emits a `displays-changed`
//! event when it changes, so the UI re-enumerates automatically on
//! connect/disconnect without manual refresh.

use super::model::DisplayInfo;

/// Stable snapshot key: the sorted, deduplicated monitor ids.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn display_ids(displays: &[DisplayInfo]) -> Vec<String> {
    let mut ids: Vec<String> = displays.iter().map(|d| d.id.clone()).collect();
    ids.sort();
    ids.dedup();
    ids
}

/// True when two snapshots differ (a display was connected or removed).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn snapshots_differ(before: &[String], after: &[String]) -> bool {
    before != after
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(id: &str) -> DisplayInfo {
        DisplayInfo {
            id: id.into(),
            device_name: String::new(),
            friendly_name: String::new(),
            is_attached: true,
            is_active: true,
            is_primary: false,
            connection_kind: None,
        }
    }

    #[test]
    fn snapshot_is_sorted_and_deduplicated() {
        let ids = display_ids(&[display("b"), display("a"), display("b")]);
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn detects_changed_set() {
        let before = display_ids(&[display("a"), display("b")]);
        let after = display_ids(&[display("c")]);
        assert!(snapshots_differ(&before, &after));
    }

    #[test]
    fn detects_no_change() {
        let before = display_ids(&[display("a"), display("b")]);
        let after = display_ids(&[display("b"), display("a")]);
        assert!(!snapshots_differ(&before, &after));
    }
}
