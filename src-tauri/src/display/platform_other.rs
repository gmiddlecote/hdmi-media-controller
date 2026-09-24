//! Non-Windows fallback.
//!
//! Display enumeration is a Windows-only feature for now. On other
//! platforms a descriptive error is returned instead of a fabricated
//! device list.

use super::model::DisplayInfo;
use super::DisplayError;

pub fn list_displays() -> Result<Vec<DisplayInfo>, DisplayError> {
    Err(DisplayError::NotSupportedOnPlatform)
}
