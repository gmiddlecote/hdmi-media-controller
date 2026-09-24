//! Scheduled playback.
//!
//! Planned responsibilities:
//! - Deterministic time-based playback (start/stop at wall-clock times).
//! - Drive playlist progression and media switching.
//! - Provide a persistent schedule persisted across restarts.
//!
//! Not implemented yet; this module carves out the boundary so scheduling
//! can be developed independently of playback and rendering.
