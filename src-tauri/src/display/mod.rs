//! Display management.
//!
//! Responsible for detecting Windows monitors/displays (including
//! HDMI-connected ones), enumerating their supported resolutions, and
//! assigning output windows to a selected display.
//!
//! The real implementation lives in [`platform_windows`] (Win32
//! `EnumDisplayDevices`); [`platform_other`] provides a stub for non-Windows
//! platforms so the crate still compiles and runs there for development.

pub mod model;

#[cfg(target_os = "windows")]
mod platform_windows;
#[cfg(target_os = "windows")]
pub use platform_windows::list_displays;

#[cfg(not(target_os = "windows"))]
mod platform_other;
#[cfg(not(target_os = "windows"))]
pub use platform_other::list_displays;

/// Errors produced while enumerating or configuring displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayError {
    /// Display enumeration is not implemented on this platform.
    NotSupportedOnPlatform,
    /// Generic failure while enumerating displays.
    Enumeration,
}

impl std::fmt::Display for DisplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSupportedOnPlatform => {
                write!(f, "Display enumeration is only supported on Windows.")
            }
            Self::Enumeration => write!(f, "Failed to enumerate displays."),
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
    }
}
