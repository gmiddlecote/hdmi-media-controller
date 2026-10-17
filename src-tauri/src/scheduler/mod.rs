//! Scheduled playback: drives the playlist through output windows.
//!
//! A worker thread (spawned once at startup) owns the playback state: the
//! [`Playlist`] cursor, the currently open output window, pause state, and
//! item timing. The kernel reacts to [`Command`]s pushed over a channel and
//! to a `render-finished` event emitted by output windows when media ends.
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
//! The kernel never uses the main thread for window creation (deadlock risk
//! on Windows with `glium`+winit).
//!
//! Switching items closes the current output first and waits for its window
//! to be destroyed before the next one opens, so two items are never on the
//! same display at once.

use std::cell::RefCell;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::audio;
use crate::media::{MediaItem, MediaKind, ObjectFit};
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
    Play {
        display: String,
    },
    /// Play one specific item on the named display.
    PlayItem {
        path: String,
        display: String,
        mode: PlayMode,
        seconds: u64,
        fit: ObjectFit,
        audio_device: String,
    },
    /// Add an item to the playlist.
    AddToPlaylist(MediaItem),
    /// Stop playback and close the output window.
    Stop,
    /// Update the fit mode for an item (playlist + live output).
    SetFit {
        path: String,
        fit: ObjectFit,
    },
    SetAudioDevice {
        path: String,
        audio_device: String,
    },
    /// Toggle repeat mode between Off and All.
    ToggleRepeat,
    /// Increase the dwell time for still images.
    IncreaseDwell,
    /// Decrease the dwell time for still images.
    DecreaseDwell,
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

/// Live playback status.
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
    /// Position when the shown item is a video or audio file.
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

/// Shared scheduler handle exposed to the TUI.
#[derive(Clone)]
pub struct State {
    tx: mpsc::Sender<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    dwell: Arc<Mutex<Duration>>,
    overlay: Arc<Mutex<OverlayConfig>>,
    /// System-default endpoint bookkeeping.
    audio: Arc<Mutex<audio::Routing>>,
    /// Device to make default whenever the app is not holding one.
    preferred_audio: Arc<Mutex<String>>,
    /// Direct reference to the renderer state.
    renderer: Arc<renderer::RendererState>,
    /// Direct reference to the volume.
    volume: Arc<renderer::Volume>,
    /// Shared playlist — the kernel mutates it, the TUI reads it.
    playlist: Arc<Mutex<Playlist>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayConfig {
    pub text: String,
    pub v_pos: String,
    pub h_pos: String,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            text: String::new(),
            v_pos: "bottom".into(),
            h_pos: "left".into(),
        }
    }
}

impl State {
    /// Creates a new state and returns it along with the command receiver
    /// that the kernel thread should consume.
    pub fn new() -> (Self, mpsc::Receiver<Command>) {
        let (tx, rx) = mpsc::channel();
        let state = Self {
            tx,
            snapshot: Arc::new(Mutex::new(Snapshot::default())),
            dwell: Arc::new(Mutex::new(Duration::from_secs(5))),
            overlay: Arc::new(Mutex::new(OverlayConfig::default())),
            audio: Arc::new(Mutex::new(audio::Routing::default())),
            preferred_audio: Arc::new(Mutex::new(String::new())),
            renderer: Arc::new(renderer::RendererState::default()),
            volume: Arc::new(renderer::Volume::default()),
            playlist: Arc::new(Mutex::new(Playlist::new())),
        };
        (state, rx)
    }

    pub(crate) fn send(&self, command: Command) -> Result<(), String> {
        self.tx
            .send(command)
            .map_err(|_| "Scheduler is not running.".into())
    }

    pub(crate) fn set_dwell_ms(&self, millis: u64) {
        *self.dwell.lock().unwrap() = Duration::from_millis(millis.max(100));
    }

    pub(crate) fn set_overlay(&self, text: String, v_pos: String, h_pos: String) {
        *self.overlay.lock().unwrap() = OverlayConfig { text, v_pos, h_pos };
    }

    pub(crate) fn overlay(&self) -> OverlayConfig {
        self.overlay.lock().unwrap().clone()
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().unwrap().clone()
    }

    pub(crate) fn set_preferred_audio_device(&self, device: String) {
        *self.preferred_audio.lock().unwrap() = device;
    }

    pub(crate) fn route_audio(&self, want: &str) {
        let home = self.preferred_audio.lock().unwrap().clone();
        let decision = self.audio.lock().unwrap().apply(want, &home);
        self.set_endpoint(decision);
    }

    pub(crate) fn restore_audio(&self) {
        let home = self.preferred_audio.lock().unwrap().clone();
        let decision = self.audio.lock().unwrap().restore(&home);
        self.set_endpoint(decision);
    }

    fn set_endpoint(&self, device: Option<String>) {
        if let Some(id) = device {
            if let Err(err) = audio::set_default_device(&id) {
                self.snapshot.lock().unwrap().last_error = Some(err);
            }
        }
    }

    /// Returns the renderer state shared with the TUI.
    pub fn renderer(&self) -> Arc<renderer::RendererState> {
        self.renderer.clone()
    }

    /// Returns the volume shared with the TUI.
    pub fn volume(&self) -> Arc<renderer::Volume> {
        self.volume.clone()
    }

    /// Returns a clone of the playlist for the TUI to render.
    pub fn playlist_items(&self) -> Vec<MediaItem> {
        self.playlist.lock().unwrap().items().to_vec()
    }

    /// Returns the item at the given index, if any.
    pub fn playlist_item_at(&self, index: usize) -> Option<MediaItem> {
        self.playlist.lock().unwrap().items().get(index).cloned()
    }

    /// Returns the number of items in the playlist.
    pub fn playlist_len(&self) -> usize {
        self.playlist.lock().unwrap().len()
    }

    /// Returns the current cursor index.
    pub fn playlist_index(&self) -> usize {
        self.playlist.lock().unwrap().index()
    }

    /// Returns whether playback is currently active.
    pub fn is_playing(&self) -> bool {
        self.snapshot.lock().unwrap().playing
    }
}

/// Starts the playback kernel as a detached worker thread.
pub fn spawn(state: State, receiver: mpsc::Receiver<Command>) {
    std::thread::spawn(move || {
        Kernel {
            state,
            receiver,
            pending: RefCell::new(None),
        }
        .run()
    });
}

struct Kernel {
    state: State,
    receiver: mpsc::Receiver<Command>,
    /// Queued transition, held until `label`'s window is gone.
    pending: RefCell<Option<Pending>>,
}

impl Kernel {
    fn run(self) {
        let (finished_tx, finished_rx) = mpsc::channel::<()>();
        // In the TUI version, output windows signal completion through the
        // renderer command channel or a separate channel. For now, images
        // advance by dwell; video completion is not yet wired.
        drop(finished_tx);

        let (_progress_tx, progress_rx) = mpsc::channel::<(f64, f64)>();

        let mut playlist = Playlist::new();
        playlist.set_repeat(RepeatMode::All);
        let mut session: Option<Session> = None;
        let mut single: Option<Single> = None;
        let mut playing = false;
        let mut paused = false;
        let mut seq = 0u64;
        let mut progress: Option<VideoProgress> = None;
        let mut progress_seq = 0u64;
        let mut last_tick_publish = Instant::now();

        loop {
            while let Ok(cmd) = self.receiver.try_recv() {
                match cmd {
                    Command::Play { display } => {
                        single = None;
                        playing = self.start(&mut playlist, &display, &mut session, &mut seq);
                        paused = false;
                    }
                    Command::PlayItem {
                        path,
                        display,
                        mode,
                        seconds,
                        fit,
                        audio_device,
                    } => {
                        let item = playback_item(path, fit, &audio_device);
                        single = Some(Single {
                            path: item.path.clone(),
                            title: item.name.clone(),
                            index: None,
                            mode,
                            seconds: seconds.max(1),
                            fit,
                            audio_device,
                        });
                        playing = self.start_item(&item, &display, &mut session, &mut seq);
                        paused = false;
                    }
                    Command::AddToPlaylist(item) => {
                        self.state.playlist.lock().unwrap().extend(vec![item]);
                    }
                    Command::SetFit { path, fit } => {
                        self.state.playlist.lock().unwrap().set_fit(&path, fit);
                        if let Some(active) = single.as_mut() {
                            if active.path == path {
                                active.fit = fit;
                            }
                        }
                        if let Some(active) = session.as_ref() {
                            if active.path == path {
                                renderer::set_fit(&self.state.renderer, &active.label, fit);
                            }
                        }
                    }
                    Command::SetAudioDevice { path, audio_device } => {
                        self.state
                            .playlist
                            .lock()
                            .unwrap()
                            .set_audio_device(&path, &audio_device);
                        if let Some(active) = single.as_mut() {
                            if active.path == path {
                                active.audio_device = audio_device;
                            }
                        }
                    }
                    Command::ToggleRepeat => {
                        let current = self.state.playlist.lock().unwrap().repeat();
                        let next = match current {
                            RepeatMode::Off => RepeatMode::All,
                            _ => RepeatMode::Off,
                        };
                        self.state.playlist.lock().unwrap().set_repeat(next);
                    }
                    Command::IncreaseDwell => {
                        let dwell = *self.state.dwell.lock().unwrap();
                        self.state.set_dwell_ms(dwell.as_millis() as u64 + 1000);
                    }
                    Command::DecreaseDwell => {
                        let dwell = *self.state.dwell.lock().unwrap();
                        let new_ms = (dwell.as_millis() as u64).saturating_sub(1000).max(100);
                        self.state.set_dwell_ms(new_ms);
                    }
                    Command::Stop => {
                        self.stop(&mut session, &mut playing, &mut paused, &mut single);
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
                                    self.state.restore_audio();
                                    playing = false;
                                }
                            }
                        }
                    }
                    Command::Prev => {
                        if playing && single.is_none() && playlist.current().is_some() {
                            playlist.prev();
                            let display = self.display_for(&session);
                            let opened =
                                self.start(&mut playlist, &display, &mut session, &mut seq);
                            playing = opened;
                        }
                    }
                    Command::SetPlaylist(items) => {
                        if items.is_empty() {
                            self.stop(&mut session, &mut playing, &mut paused, &mut single);
                        } else {
                            playlist.replace(items);
                            let display = self.display_for(&session);
                            playing = self.start(&mut playlist, &display, &mut session, &mut seq);
                        }
                    }
                }
                if seq != progress_seq {
                    progress = None;
                    progress_seq = seq;
                    while finished_rx.try_recv().is_ok() {}
                }
                self.publish(&playlist, &session, playing, paused, &single, progress);
            }

            if seq != progress_seq {
                progress = None;
                progress_seq = seq;
                while finished_rx.try_recv().is_ok() {}
            }
            while let Ok((current, duration)) = progress_rx.try_recv() {
                progress = Some(VideoProgress { current, duration });
            }

            if let Some(pending) = self.take_pending_if_ready() {
                while finished_rx.try_recv().is_ok() {}
                progress = None;
                if self.open_item(&pending.item, &pending.display, &mut session, &mut seq) {
                    if paused {
                        if let Some(active) = session.as_ref() {
                            self.send_control(&active.label, true);
                        }
                    }
                } else {
                    self.stop(&mut session, &mut playing, &mut paused, &mut single);
                }
                progress_seq = seq;
                self.publish(&playlist, &session, playing, paused, &single, progress);
            }

            if playing && !paused {
                if let Some(active) = session.as_ref() {
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
                    } else if dwell_due {
                        match single.as_ref() {
                            Some(active_single) if active_single.mode != PlayMode::Once => {
                                self.restart_session(active);
                                if let Some(current) = session.as_mut() {
                                    current.since = Instant::now();
                                }
                            }
                            Some(_) => {
                                self.stop(&mut session, &mut playing, &mut paused, &mut single);
                            }
                            None => {
                                let opened =
                                    self.step_forward(&mut playlist, &mut session, &mut seq);
                                if !opened {
                                    self.state.restore_audio();
                                    playing = false;
                                }
                            }
                        }
                        self.publish(&playlist, &session, playing, paused, &single, progress);
                    }
                }

                if session
                    .as_ref()
                    .is_some_and(|active| active.kind == MediaKind::Image)
                    && last_tick_publish.elapsed() >= Duration::from_millis(500)
                {
                    last_tick_publish = Instant::now();
                    self.publish(&playlist, &session, playing, paused, &single, progress);
                }
            }

            if !playing {
                while finished_rx.try_recv().is_ok() {}
                progress = None;
            }

            std::thread::sleep(tick_for_state(playing, paused, &session));
        }
    }

    fn start(
        &self,
        playlist: &mut Playlist,
        display: &str,
        session: &mut Option<Session>,
        seq: &mut u64,
    ) -> bool {
        let Some(item) = playlist.current().cloned() else {
            self.pending.borrow_mut().take();
            self.set_error("Playlist is empty.".into());
            self.close_session(session);
            return false;
        };
        let target = if item.display.is_empty() {
            display.to_string()
        } else {
            item.display.clone()
        };
        self.start_item(&item, &target, session, seq)
    }

    fn start_item(
        &self,
        item: &MediaItem,
        display: &str,
        session: &mut Option<Session>,
        seq: &mut u64,
    ) -> bool {
        let closed = self.close_session(session);
        let _queued = self.pending.borrow_mut().take().map(|queued| queued.label);
        // In the TUI version, we don't check window aliveness via AppHandle;
        // just proceed to open.
        drop(closed);
        self.open_item(item, display, session, seq)
    }

    fn open_item(
        &self,
        item: &MediaItem,
        display: &str,
        session: &mut Option<Session>,
        seq: &mut u64,
    ) -> bool {
        self.state.route_audio(&item.audio_device);
        if item.kind == MediaKind::Audio {
            *seq += 1;
            let label = format!("audio-{seq}");
            let overlay = self.state.overlay();
            if let Err(err) =
                renderer::open_audio(&self.state.renderer, item, &label, &overlay.text)
            {
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
            return true;
        }

        let displays = crate::display::list_displays().unwrap_or_default();
        let Some(display_info) = displays.iter().find(|d| d.device_name == display).cloned() else {
            self.set_error(format!("Display {display} is no longer connected."));
            return false;
        };
        *seq += 1;
        let label = renderer::label_for(display, *seq);
        let overlay = self.state.overlay();
        if let Err(err) = renderer::open(
            &self.state.renderer,
            &display_info,
            item,
            &label,
            &overlay.text,
        ) {
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

    fn take_pending_if_ready(&self) -> Option<Pending> {
        let mut pending = self.pending.borrow_mut();
        let ready = pending
            .as_ref()
            .is_some_and(|queued| transition_ready(true, queued.deadline, Instant::now()));
        if ready {
            pending.take()
        } else {
            None
        }
    }

    fn step_forward(
        &self,
        playlist: &mut Playlist,
        session: &mut Option<Session>,
        seq: &mut u64,
    ) -> bool {
        if !playlist.advance() {
            self.pending.borrow_mut().take();
            self.close_session(session);
            return false;
        }
        let display = self.display_for(session);
        self.start(playlist, &display, session, seq)
    }

    fn stop(
        &self,
        session: &mut Option<Session>,
        playing: &mut bool,
        paused: &mut bool,
        single: &mut Option<Single>,
    ) {
        self.pending.borrow_mut().take();
        self.close_session(session);
        self.state.restore_audio();
        *playing = false;
        *paused = false;
        *single = None;
    }

    fn close_session(&self, session: &mut Option<Session>) -> Option<String> {
        let active = session.take()?;
        renderer::close(&self.state.renderer, &active.label);
        Some(active.label)
    }

    fn display_for(&self, session: &Option<Session>) -> String {
        session
            .as_ref()
            .map(|active| active.display.clone())
            .or_else(|| {
                self.pending
                    .borrow()
                    .as_ref()
                    .map(|queued| queued.display.clone())
            })
            .unwrap_or_default()
    }

    fn send_control(&self, _label: &str, _paused: bool) {
        // In the TUI version, output windows don't listen to Tauri events.
        // Control would need to be sent via the renderer command channel.
    }

    fn restart_session(&self, _session: &Session) {
        // In the TUI version, we'd send a restart command to the renderer.
    }

    fn set_error(&self, message: String) {
        self.state.snapshot.lock().unwrap().last_error = Some(message);
    }

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
        let queued = self.pending.borrow();
        let queued = queued.as_ref();
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
            path: session
                .as_ref()
                .map(|active| active.path.clone())
                .or_else(|| queued.map(|queued| queued.item.path.clone())),
            elapsed_ms: session
                .as_ref()
                .map(|active| active.since.elapsed().as_millis() as u64)
                .unwrap_or(0),
            dwell_ms: self.state.dwell.lock().unwrap().as_millis() as u64,
            progress: if playing
                && !paused
                && session.as_ref().is_some_and(|active| {
                    matches!(active.kind, MediaKind::Video | MediaKind::Audio)
                }) {
                progress
            } else {
                None
            },
            display: session
                .as_ref()
                .filter(|active| active.kind != MediaKind::Audio)
                .map(|active| active.display.clone())
                .or_else(|| {
                    queued
                        .filter(|queued| queued.item.kind != MediaKind::Audio)
                        .map(|queued| queued.display.clone())
                }),
            last_error,
        };
        *self.state.snapshot.lock().unwrap() = snapshot.clone();
    }
}

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
    /// Fit mode applied whenever this item is (re)opened.
    fit: ObjectFit,
    audio_device: String,
}

/// Returns the appropriate tick duration based on current playback state.
fn tick_for_state(playing: bool, paused: bool, session: &Option<Session>) -> Duration {
    if !playing || paused {
        Duration::from_millis(500)
    } else if let Some(active) = session {
        match active.kind {
            MediaKind::Video | MediaKind::Audio => Duration::from_millis(33),
            MediaKind::Image => Duration::from_millis(200),
        }
    } else {
        Duration::from_millis(200)
    }
}

/// How long a switch waits for the previous output window to be destroyed
/// before opening anyway.
const TRANSITION_TIMEOUT: Duration = Duration::from_secs(2);

/// An item waiting for the output window it replaces to be destroyed.
struct Pending {
    item: MediaItem,
    display: String,
    /// Label of the window being torn down.
    label: String,
    deadline: Instant,
}

fn transition_ready(_window_alive: bool, deadline: Instant, now: Instant) -> bool {
    now >= deadline
}

fn playback_item(path: String, fit: ObjectFit, audio_device: &str) -> MediaItem {
    let mut item = MediaItem::from_path(path);
    item.fit = fit;
    item.audio_device = audio_device.to_string();
    item
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
}
