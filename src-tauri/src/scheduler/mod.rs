//! Scheduled playback: drives the playlist through output windows.
//!
//! A worker thread (spawned once at startup) owns the playback state: the
//! [`Playlist`] cursor, the currently open output window, pause state, and
//! item timing. Display management stays on the calling threads — the
//! kernel reacts to [`Command`]s pushed over a channel and to a
//! `render-finished` event emitted by output windows when a video ends.
//!
//! Images are swapped after a configurable dwell period; videos run until
//! they finish. Playlist exhaustion follows [`RepeatMode`]: the kernel
//! stops when an `Off` playlist ends and wraps when `All` is set.
//!
//! Besides stepping through the playlist, the kernel can drive a *single*
//! item (from a queue click) in [`PlayMode::Once`], [`PlayMode::Loop`], or
//! [`PlayMode::Timed`] — the last showing the item for a fixed number of
//! seconds and stopping.
//!
//! The kernel never uses the Tauri main/command thread for window
//! creation (deadlock risk on Windows with `WebviewWindowBuilder`).

use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Listener};

use crate::media::{MediaItem, MediaKind};
use crate::playlist::{Playlist, RepeatMode};
use crate::renderer;

/// How a single item (clicked in the queue) is played.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayMode {
    /// Play the item to its natural end, then stop.
    Once,
    /// Play the item over and over until stopped.
    Loop,
    /// Show the item for a fixed number of seconds, then stop.
    Timed,
}

/// Signals the kernel can receive.
#[derive(Debug)]
pub enum Command {
    /// Start stepping through the playlist on the named display.
    Play { display: String },
    /// Play one specific item on the named display.
    PlayItem {
        path: String,
        display: String,
        mode: PlayMode,
        seconds: u64,
    },
    /// Stop playback and close the output window.
    Stop,
    /// Pause advancing / video playback.
    Pause,
    /// Resume advancing / video playback.
    Resume,
    /// Jump to the next entry.
    Next,
    /// Jump to the previous entry.
    Prev,
    /// Replace the playlist (files already validated by the caller).
    SetPlaylist(Vec<MediaItem>),
}

/// Live playback status, serialized to the UI on every change.
#[derive(Clone, Debug, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub playing: bool,
    pub paused: bool,
    pub index: usize,
    pub total: usize,
    pub title: Option<String>,
    /// Absolute path of the currently shown item.
    pub path: Option<String>,
    /// Milliseconds the current item has been on screen.
    pub elapsed_ms: u64,
    /// Milliseconds a still image stays up (`elapsedMs` reaches it before advance).
    pub dwell_ms: u64,
    /// Video position, when the shown item is a video.
    pub progress: Option<VideoProgress>,
    pub display: Option<String>,
    pub last_error: Option<String>,
}

/// Position within the currently playing video, in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoProgress {
    pub current: f64,
    pub duration: f64,
}

/// Video position payload sent by output windows over `media-progress`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaProgressPayload {
    current: f64,
    duration: f64,
}

/// Shared scheduler handle exposed to Tauri commands.
#[derive(Clone)]
pub struct State {
    tx: mpsc::Sender<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    dwell: Arc<Mutex<Duration>>,
    overlay: Arc<Mutex<String>>,
}

impl State {
    pub(crate) fn new(tx: mpsc::Sender<Command>) -> Self {
        Self {
            tx,
            snapshot: Arc::new(Mutex::new(Snapshot::default())),
            dwell: Arc::new(Mutex::new(Duration::from_secs(5))),
            overlay: Arc::new(Mutex::new(String::new())),
        }
    }

    pub(crate) fn send(&self, command: Command) -> Result<(), String> {
        self.tx
            .send(command)
            .map_err(|_| "Scheduler is not running.".into())
    }

    /// Milliseconds a still image stays on screen before advancing.
    pub(crate) fn set_dwell_ms(&self, millis: u64) {
        *self.dwell.lock().unwrap() = Duration::from_millis(millis.max(100));
    }

    /// Stores the caption shown on output windows; applied immediately to
    /// existing windows by the invoking command.
    pub(crate) fn set_overlay(&self, text: String) {
        *self.overlay.lock().unwrap() = text;
    }

    /// Current output caption (empty when none is configured).
    pub(crate) fn overlay(&self) -> String {
        self.overlay.lock().unwrap().clone()
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().unwrap().clone()
    }
}

/// An output window currently on screen.
struct Session {
    label: String,
    /// Win32 device name the window is placed on.
    display: String,
    /// Absolute path of the item being shown, for the snapshot.
    path: String,
    kind: MediaKind,
    since: Instant,
}

/// A single item played on its own (from a queue click), not via the playlist.
struct Single {
    /// Absolute path of the item being shown, for restarts.
    path: String,
    title: String,
    /// Queue position of the item, so the UI can highlight it.
    index: Option<usize>,
    mode: PlayMode,
    /// Seconds the item stays on screen for `PlayMode::Timed`.
    seconds: u64,
}

/// How long the kernel waits between decision ticks. Small enough for
/// responsive image dwell, large enough to keep idle CPU negligible.
const TICK: Duration = Duration::from_millis(50);

/// Starts the playback kernel as a detached worker thread.
pub fn spawn(app: AppHandle, receiver: mpsc::Receiver<Command>, state: State) {
    std::thread::spawn(move || {
        Kernel {
            app,
            receiver,
            state,
        }
        .run()
    });
}

struct Kernel {
    app: AppHandle,
    receiver: mpsc::Receiver<Command>,
    state: State,
}

impl Kernel {
    fn run(self) {
        let (finished_tx, finished_rx) = mpsc::channel::<()>();
        // Output windows emit `render-finished` when their video ends.
        let _listener = self.app.listen("render-finished", move |_| {
            let _ = finished_tx.send(());
        });

        let (progress_tx, progress_rx) = mpsc::channel::<(f64, f64)>();
        // Output windows emit `media-progress` while a video plays.
        let _progress_listener = self.app.listen("media-progress", move |event| {
            if let Ok(payload) = serde_json::from_str::<MediaProgressPayload>(event.payload()) {
                let _ = progress_tx.send((payload.current, payload.duration));
            }
        });

        let mut playlist = Playlist::new();
        playlist.set_repeat(RepeatMode::All);
        let mut session: Option<Session> = None;
        let mut single: Option<Single> = None;
        let mut playing = false;
        let mut paused = false;
        let mut seq = 0u64;
        // Last-known video position, valid only while the seq it belongs to matches.
        let mut progress: Option<VideoProgress> = None;
        let mut progress_seq = 0u64;
        let mut last_tick_publish = Instant::now();

        loop {
            while let Ok(cmd) = self.receiver.try_recv() {
                match cmd {
                    Command::Play { display } => {
                        single = None;
                        playing = self.start(&playlist, &display, &mut session, &mut seq);
                        paused = false;
                    }
                    Command::PlayItem {
                        path,
                        display,
                        mode,
                        seconds,
                    } => {
                        let item = MediaItem::from_path(path);
                        single = Some(Single {
                            path: item.path.clone(),
                            title: item.name.clone(),
                            index: playlist.position(&item.path),
                            mode,
                            seconds: seconds.max(1),
                        });
                        playing = self.start_item(&item, &display, &mut session, &mut seq);
                        paused = false;
                    }
                    Command::Stop => {
                        self.stop(&mut session, &mut playing, &mut paused, &mut single)
                    }
                    Command::Pause => {
                        paused = playing;
                        if let Some(active) = session.as_ref() {
                            self.send_control(&active.label, true);
                        }
                    }
                    Command::Resume => {
                        paused = false;
                        if let Some(active) = session.as_ref() {
                            self.send_control(&active.label, false);
                        }
                    }
                    Command::Next => {
                        if playing {
                            if single.is_some() {
                                self.stop(&mut session, &mut playing, &mut paused, &mut single);
                            } else {
                                let opened =
                                    self.step_forward(&mut playlist, &mut session, &mut seq);
                                if !opened {
                                    playing = false;
                                }
                            }
                        }
                    }
                    Command::Prev => {
                        if playing && single.is_none() && playlist.current().is_some() {
                            playlist.prev();
                            let display = display_of(&session);
                            let opened = self.start(&playlist, &display, &mut session, &mut seq);
                            playing = opened;
                        }
                    }
                    Command::SetPlaylist(items) => {
                        if items.is_empty() {
                            self.stop(&mut session, &mut playing, &mut paused, &mut single);
                        } else if playing && single.is_none() {
                            playlist.replace(items);
                            let display = display_of(&session);
                            playing = self.start(&playlist, &display, &mut session, &mut seq);
                        } else {
                            playlist.replace(items);
                        }
                    }
                }
                // Progress from the previous item would be misleading; drop it.
                if seq != progress_seq {
                    progress = None;
                    progress_seq = seq;
                }
                self.publish(&playlist, &session, playing, paused, &single, progress);
            }

            // Adopt any video position updates that arrived since the last tick.
            if seq != progress_seq {
                progress = None;
                progress_seq = seq;
            }
            while let Ok((current, duration)) = progress_rx.try_recv() {
                progress = Some(VideoProgress { current, duration });
            }

            if playing && !paused {
                if let Some(active) = session.as_ref() {
                    let video_ended =
                        active.kind == MediaKind::Video && finished_rx.try_recv().is_ok();
                    // Still images advance after the dwell period, except in
                    // Loop/Timed modes where their clock is the mode itself.
                    let dwell_due = active.kind == MediaKind::Image
                        && single
                            .as_ref()
                            .is_none_or(|single| single.mode == PlayMode::Once)
                        && active.since.elapsed() >= *self.state.dwell.lock().unwrap();
                    let timed_due = match single.as_ref() {
                        Some(single) if single.mode == PlayMode::Timed => {
                            active.since.elapsed() >= Duration::from_secs(single.seconds)
                        }
                        _ => false,
                    };

                    if timed_due {
                        self.stop(&mut session, &mut playing, &mut paused, &mut single);
                        self.publish(&playlist, &session, playing, paused, &single, progress);
                    } else if video_ended || dwell_due {
                        match single.as_ref() {
                            Some(active_single) if active_single.mode != PlayMode::Once => {
                                // Loop / Timed: restart the item to fill the slot.
                                let display = display_of(&session);
                                let item = MediaItem::from_path(active_single.path.clone());
                                let opened =
                                    self.start_item(&item, &display, &mut session, &mut seq);
                                if !opened {
                                    playing = false;
                                    self.stop(&mut session, &mut playing, &mut paused, &mut single);
                                }
                            }
                            Some(_) => {
                                self.stop(&mut session, &mut playing, &mut paused, &mut single)
                            }
                            None => {
                                let opened =
                                    self.step_forward(&mut playlist, &mut session, &mut seq);
                                if !opened {
                                    playing = false;
                                }
                            }
                        }
                        self.publish(&playlist, &session, playing, paused, &single, progress);
                    }
                }

                // Keep a live dwell/elapsed readout for still images on screen.
                if session
                    .as_ref()
                    .is_some_and(|active| active.kind == MediaKind::Image)
                    && last_tick_publish.elapsed() >= Duration::from_millis(500)
                {
                    last_tick_publish = Instant::now();
                    self.publish(&playlist, &session, playing, paused, &single, progress);
                }
            }

            // Drain stale finish signals that raced a stop/switch.
            if !playing {
                while finished_rx.try_recv().is_ok() {}
                progress = None;
            }

            std::thread::sleep(TICK);
        }
    }

    /// Opens the playlist's current entry on `display`.
    fn start(
        &self,
        playlist: &Playlist,
        display: &str,
        session: &mut Option<Session>,
        seq: &mut u64,
    ) -> bool {
        let Some(item) = playlist.current().cloned() else {
            self.set_error("Playlist is empty.".into());
            self.close_session(session);
            return false;
        };
        self.start_item(&item, display, session, seq)
    }

    /// Opens one item on `display`, replacing any active output.
    fn start_item(
        &self,
        item: &MediaItem,
        display: &str,
        session: &mut Option<Session>,
        seq: &mut u64,
    ) -> bool {
        self.close_session(session);

        let displays = crate::display::list_displays().unwrap_or_default();
        let Some(display_info) = displays.iter().find(|d| d.device_name == display).cloned() else {
            self.set_error(format!("Display {display} is no longer connected."));
            return false;
        };
        *seq += 1;
        let label = renderer::label_for(display, *seq);
        let overlay = self.state.overlay();
        if let Err(err) = renderer::open(&self.app, &display_info, item, &label, &overlay) {
            self.set_error(err.to_string());
            return false;
        }
        *session = Some(Session {
            label,
            display: display.to_string(),
            path: item.path.clone(),
            kind: item.kind,
            since: Instant::now(),
        });
        true
    }

    /// Moves to the next playlist entry. Returns `false` when the playlist
    /// is exhausted (`RepeatMode::Off`), which also closes the output.
    fn step_forward(
        &self,
        playlist: &mut Playlist,
        session: &mut Option<Session>,
        seq: &mut u64,
    ) -> bool {
        if !playlist.advance() {
            self.close_session(session);
            return false;
        }
        let display = display_of(session);
        self.start(playlist, &display, session, seq)
    }

    /// Closes the active output and returns to idle.
    fn stop(
        &self,
        session: &mut Option<Session>,
        playing: &mut bool,
        paused: &mut bool,
        single: &mut Option<Single>,
    ) {
        self.close_session(session);
        *playing = false;
        *paused = false;
        *single = None;
    }

    fn close_session(&self, session: &mut Option<Session>) {
        if let Some(active) = session.take() {
            renderer::close(&self.app, &active.label);
        }
    }

    /// Tells an output window to pause (`true`) or resume (`false`) video.
    fn send_control(&self, label: &str, paused: bool) {
        let _ = self.app.emit_to(label, "output-control", paused);
    }

    /// Stores the latest error so the UI can surface it.
    fn set_error(&self, message: String) {
        self.state.snapshot.lock().unwrap().last_error = Some(message);
    }

    /// Emits the current playback status to the control UI.
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &self,
        playlist: &Playlist,
        session: &Option<Session>,
        playing: bool,
        paused: bool,
        single: &Option<Single>,
        progress: Option<VideoProgress>,
    ) {
        let last_error = self.state.snapshot.lock().unwrap().last_error.clone();
        let snapshot = Snapshot {
            playing,
            paused,
            index: single
                .as_ref()
                .and_then(|active| active.index)
                .unwrap_or_else(|| playlist.index()),
            total: playlist.len(),
            title: single
                .as_ref()
                .map(|active| active.title.clone())
                .or_else(|| playlist.current().map(|item| item.name.clone())),
            path: session.as_ref().map(|active| active.path.clone()),
            elapsed_ms: session
                .as_ref()
                .map(|active| active.since.elapsed().as_millis() as u64)
                .unwrap_or(0),
            dwell_ms: self.state.dwell.lock().unwrap().as_millis() as u64,
            progress: if playing
                && !paused
                && session
                    .as_ref()
                    .is_some_and(|active| active.kind == MediaKind::Video)
            {
                progress
            } else {
                None
            },
            display: session.as_ref().map(|active| active.display.clone()),
            last_error,
        };
        *self.state.snapshot.lock().unwrap() = snapshot.clone();
        let _ = self.app.emit("scheduler-state", snapshot);
    }
}

/// Display device name an active session is placed on, or empty.
fn display_of(session: &Option<Session>) -> String {
    session
        .as_ref()
        .map(|active| active.display.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_mode_deserializes_from_ui_values() {
        assert_eq!(
            serde_json::from_str::<PlayMode>("\"once\"").unwrap(),
            PlayMode::Once
        );
        assert_eq!(
            serde_json::from_str::<PlayMode>("\"loop\"").unwrap(),
            PlayMode::Loop
        );
        assert_eq!(
            serde_json::from_str::<PlayMode>("\"timed\"").unwrap(),
            PlayMode::Timed
        );
    }

    #[test]
    fn play_mode_rejects_unknown_values() {
        assert!(serde_json::from_str::<PlayMode>("\"fast\"").is_err());
        assert!(serde_json::from_str::<PlayMode>("\"\"").is_err());
    }

    #[test]
    fn snapshot_serializes_progress_fields_as_camel_case() {
        let snapshot = Snapshot {
            playing: true,
            paused: false,
            index: 2,
            total: 5,
            title: Some("clip.mp4".into()),
            path: Some("C:\\x\\clip.mp4".into()),
            elapsed_ms: 1234,
            dwell_ms: 5000,
            progress: Some(VideoProgress {
                current: 1.5,
                duration: 10.0,
            }),
            display: Some(r"\\.\DISPLAY1".into()),
            last_error: None,
        };
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(value["elapsedMs"], 1234);
        assert_eq!(value["dwellMs"], 5000);
        assert_eq!(value["path"], "C:\\x\\clip.mp4");
        assert_eq!(value["progress"]["current"], 1.5);
        assert_eq!(value["progress"]["duration"], 10.0);
    }

    #[test]
    fn media_progress_payload_parses_renderer_events() {
        let parsed: MediaProgressPayload =
            serde_json::from_str(r#"{"current":2.25,"duration":30}"#).unwrap();
        assert_eq!(parsed.current, 2.25);
        assert_eq!(parsed.duration, 30.0);
    }
}
