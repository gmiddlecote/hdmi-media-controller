//! Media playback.
//!
//! Planned responsibilities:
//! - Load local image and video files selected by the user.
//! - Report media metadata (dimensions, codec, duration).
//! - Hand decoded frames to [`crate::renderer`] for output on the selected
//!   external display.
//!
//! Not implemented yet; this module carves out the boundary so playback
//! can be developed independently of display management and scheduling.

/// Kind of media a source can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
}
