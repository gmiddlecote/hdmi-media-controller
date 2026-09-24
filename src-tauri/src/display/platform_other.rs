//! Non-Windows fallback.
//!
//! Display management is a Windows-only feature for now. On other
//! platforms a descriptive error is returned instead of fabricated data.

use super::model::{DisplayInfo, DisplayMode, DisplayModes};
use super::DisplayError;

pub fn list_displays() -> Result<Vec<DisplayInfo>, DisplayError> {
    Err(DisplayError::NotSupportedOnPlatform)
}

pub fn get_display_modes(_device_name: &str) -> Result<DisplayModes, DisplayError> {
    Err(DisplayError::NotSupportedOnPlatform)
}

pub fn set_display_mode(_device_name: &str, _mode: &DisplayMode) -> Result<(), DisplayError> {
    Err(DisplayError::NotSupportedOnPlatform)
}
