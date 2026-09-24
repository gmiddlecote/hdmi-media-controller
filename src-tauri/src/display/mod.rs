//! Display management.
//!
//! Responsible for detecting Windows monitors/displays (including
//! HDMI-connected ones), identifying their connector type, enumerating
//! their supported resolutions, applying a selected mode, and notifying
//! the UI when the display set changes (hotplug).
//!
//! The real implementation lives in [`platform_windows`] (Win32 GDI +
//! display config); [`platform_other`] provides stubs for non-Windows
//! platforms so the crate still compiles and runs there for development.

pub mod hotplug;
pub mod model;

#[cfg(target_os = "windows")]
mod platform_windows;
#[cfg(target_os = "windows")]
pub use platform_windows::{get_display_modes, list_displays, set_display_mode};

#[cfg(not(target_os = "windows"))]
mod platform_other;
#[cfg(not(target_os = "windows"))]
pub use platform_other::{get_display_modes, list_displays, set_display_mode};

/// Errors produced while enumerating or configuring displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayError {
    /// Display management is not implemented on this platform.
    NotSupportedOnPlatform,
    /// Generic failure while enumerating displays.
    Enumeration,
    /// A display-settings operation failed (mode query or apply).
    Settings(String),
}

impl std::fmt::Display for DisplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSupportedOnPlatform => {
                write!(f, "Display management is only supported on Windows.")
            }
            Self::Enumeration => write!(f, "Failed to enumerate displays."),
            Self::Settings(detail) => write!(f, "Display settings error: {detail}"),
        }
    }
}

impl std::error::Error for DisplayError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_renders_a_description() {
        assert!(!DisplayError::NotSupportedOnPlatform.to_string().is_empty());
        assert!(!DisplayError::Enumeration.to_string().is_empty());
        assert!(!DisplayError::Settings("boom".into()).to_string().is_empty());
    }
}
