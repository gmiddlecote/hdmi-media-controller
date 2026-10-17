//! Terminal application state and rendering.
//!
//! Holds the playlist, scheduler state, renderer state, and display list.
//! Renders a split-screen TUI: file browser on the left, playlist in the
//! middle, status and controls on the right.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::audio::AudioDevice;
use crate::display::model::DisplayInfo;
use crate::media::{MediaItem, MediaKind, ObjectFit};
use crate::renderer::{RendererState, Volume};
use crate::scheduler::{Command, PlayMode, State as SchedulerState};

/// The main TUI application.
pub struct App {
    registry: crate::media::Registry,
    renderer: RendererState,
    volume: Volume,
    scheduler: SchedulerState,
    displays: Vec<DisplayInfo>,
    audio_devices: Vec<AudioDevice>,
    selected_display: Option<String>,
    /// Current UI mode.
    mode: Mode,
    /// File browser state.
    file_browser: FileBrowser,
    /// Playlist list state.
    playlist_state: ListState,
    /// Status message shown at the bottom.
    status: String,
    status_time: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Normal browsing/playlist view.
    Normal,
    /// File browser is focused.
    Browse,
    /// Display selection is active.
    SelectDisplay,
}

struct FileBrowser {
    current_dir: PathBuf,
    entries: Vec<DirEntry>,
    state: ListState,
}

#[derive(PartialEq)]
struct DirEntry {
    name: String,
    path: PathBuf,
    kind: EntryKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Dir,
    File,
}

impl App {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = crate::media::Registry::default();
        let renderer = RendererState::default();
        let volume = Volume::default();
        let (scheduler, scheduler_rx) = SchedulerState::new();

        // Start the scheduler worker thread.
        crate::scheduler::spawn(scheduler.clone(), scheduler_rx);

        let displays = crate::display::list_displays().unwrap_or_default();
        let audio_devices = crate::audio::list_devices();

        Ok(Self {
            registry,
            renderer,
            volume,
            scheduler,
            displays,
            audio_devices,
            selected_display: None,
            mode: Mode::Normal,
            file_browser: FileBrowser::new(std::env::current_dir().unwrap_or_default()),
            playlist_state: ListState::default(),
            status: String::new(),
            status_time: None,
        })
    }

    pub fn ui(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(40),
                Constraint::Percentage(30),
            ])
            .split(f.area());

        self.render_file_browser(f, chunks[0]);
        self.render_playlist(f, chunks[1]);
        self.render_status(f, chunks[2]);
    }

    fn render_file_browser(&mut self, f: &mut Frame, area: Rect) {
        let title = match self.mode {
            Mode::Browse => " File Browser (focused) ",
            _ => " File Browser ",
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(if self.mode == Mode::Browse {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            });

        let dir = self.file_browser.current_dir.display().to_string();
        let path_para = Paragraph::new(dir)
            .block(Block::default().borders(Borders::BOTTOM))
            .style(Style::default().fg(Color::Gray));

        let path_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        f.render_widget(path_para, path_area);

        let list_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(2),
        };

        let items: Vec<ListItem> = self
            .file_browser
            .entries
            .iter()
            .map(|entry| {
                let icon = match entry.kind {
                    EntryKind::Dir => "📁 ",
                    EntryKind::File => "📄 ",
                };
                let selected = self
                    .file_browser
                    .state
                    .selected()
                    .is_some_and(|i| self.file_browser.entries.get(i) == Some(entry));
                let style = if selected {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default()
                };
                ListItem::new(format!("{}{}", icon, entry.name)).style(style)
            })
            .collect();

        let list = List::new(items).block(block);
        f.render_stateful_widget(list, list_area, &mut self.file_browser.state);

        let help_area = Rect {
            x: area.x,
            y: area.y + area.height - 1,
            width: area.width,
            height: 1,
        };
        let help = Paragraph::new("↑/↓ navigate • Enter open • ← back • a add to playlist")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        f.render_widget(help, help_area);
    }

    fn render_playlist(&mut self, f: &mut Frame, area: Rect) {
        let block = Block::default().title(" Playlist ").borders(Borders::ALL);

        let items = self.scheduler.playlist_items();
        let list_items: Vec<ListItem> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let icon = match item.kind {
                    MediaKind::Image => "🖼️ ",
                    MediaKind::Video => "🎬 ",
                    MediaKind::Audio => "🎵 ",
                };
                let fit = match item.fit {
                    ObjectFit::Cover => "cover",
                    ObjectFit::Contain => "contain",
                };
                let selected = self.playlist_state.selected() == Some(i);
                let style = if selected {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default()
                };
                ListItem::new(format!(
                    "{} {} ({}, {})",
                    icon,
                    item.name,
                    fit,
                    item.path
                        .split(std::path::MAIN_SEPARATOR)
                        .next_back()
                        .unwrap_or(&item.name)
                ))
                .style(style)
            })
            .collect();

        let list = List::new(list_items).block(block);
        f.render_stateful_widget(list, area, &mut self.playlist_state);
    }

    fn render_status(&mut self, f: &mut Frame, area: Rect) {
        let snapshot = self.scheduler.snapshot();
        let playing = if snapshot.playing { "▶" } else { "⏸" };
        let paused = if snapshot.paused { " (paused)" } else { "" };

        let display = self
            .selected_display
            .as_deref()
            .unwrap_or("(none selected)")
            .rsplit('\\')
            .next()
            .unwrap_or("none");

        let lines = vec![
            format!(
                "{} {} {}• Display: {}",
                playing,
                snapshot.title.as_deref().unwrap_or("—"),
                paused,
                display
            ),
            format!(
                "• Items: {} • Dwell: {}ms",
                snapshot.total, snapshot.dwell_ms
            ),
            String::new(),
            "Controls:".to_string(),
            "  q       Quit".to_string(),
            "  ↑/↓     Navigate playlist".to_string(),
            "  →       Enter file browser".to_string(),
            "  Enter   Play selected".to_string(),
            "  p       Play/Pause".to_string(),
            "  n       Next".to_string(),
            "  b       Prev".to_string(),
            "  s       Stop".to_string(),
            "  d       Select display".to_string(),
            "  a       Add file to playlist".to_string(),
            "  r       Toggle repeat".to_string(),
            "  +       Increase dwell".to_string(),
            "  -       Decrease dwell".to_string(),
        ];

        let status_text = if !self.status.is_empty() {
            format!("{}\n\n{}", self.status, lines.join("\n"))
        } else {
            lines.join("\n")
        };

        let para = Paragraph::new(status_text).block(
            Block::default()
                .title(" Status & Controls ")
                .borders(Borders::ALL),
        );
        f.render_widget(para, area);
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        match self.mode {
            Mode::Browse => self.handle_browse_key(key),
            Mode::SelectDisplay => self.handle_display_key(key),
            Mode::Normal => self.handle_normal_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Right => {
                self.mode = Mode::Browse;
                self.file_browser.state.select(Some(0));
            }
            KeyCode::Down => {
                self.playlist_next();
            }
            KeyCode::Up => {
                self.playlist_prev();
            }
            KeyCode::Enter => {
                self.play_selected();
            }
            KeyCode::Char('p') => {
                if self.scheduler.is_playing() {
                    let _ = self.scheduler.send(Command::Pause);
                } else {
                    let _ = self.scheduler.send(Command::Resume);
                }
            }
            KeyCode::Char('n') => {
                let _ = self.scheduler.send(Command::Next);
            }
            KeyCode::Char('b') => {
                let _ = self.scheduler.send(Command::Prev);
            }
            KeyCode::Char('s') => {
                let _ = self.scheduler.send(Command::Stop);
            }
            KeyCode::Char('d') => {
                self.mode = Mode::SelectDisplay;
                if self.selected_display.is_none() && !self.displays.is_empty() {
                    self.selected_display = Some(self.displays[0].device_name.clone());
                }
            }
            KeyCode::Char('a') => {
                self.mode = Mode::Browse;
                self.file_browser.state.select(Some(0));
            }
            KeyCode::Char('r') => {
                let _ = self.scheduler.send(Command::ToggleRepeat);
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                let _ = self.scheduler.send(Command::IncreaseDwell);
            }
            KeyCode::Char('-') => {
                let _ = self.scheduler.send(Command::DecreaseDwell);
            }
            _ => {}
        }
    }

    fn handle_browse_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Up => {
                self.file_browser.prev();
            }
            KeyCode::Down => {
                self.file_browser.next();
            }
            KeyCode::Left => {
                self.file_browser.go_up();
            }
            KeyCode::Right | KeyCode::Enter => {
                self.file_browser.enter();
            }
            KeyCode::Char('a') => {
                if let Some(entry) = self.file_browser.selected_entry() {
                    if entry.kind == EntryKind::File {
                        self.add_to_playlist(entry.path.clone());
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_display_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Up | KeyCode::Down => {
                if !self.displays.is_empty() {
                    let current = self
                        .selected_display
                        .as_ref()
                        .and_then(|d| self.displays.iter().position(|x| x.device_name == *d))
                        .unwrap_or(0);
                    let next = if key.code == KeyCode::Down {
                        (current + 1) % self.displays.len()
                    } else {
                        (current + self.displays.len() - 1) % self.displays.len()
                    };
                    self.selected_display = Some(self.displays[next].device_name.clone());
                }
            }
            KeyCode::Enter => {
                if let Some(display) = self.selected_display.take() {
                    let _ = self.scheduler.send(Command::Play { display });
                    self.mode = Mode::Normal;
                }
            }
            _ => {}
        }
    }

    fn playlist_next(&mut self) {
        let len = self.scheduler.playlist_len();
        if len == 0 {
            return;
        }
        let current = self.playlist_state.selected().unwrap_or(0);
        let next = (current + 1) % len;
        self.playlist_state.select(Some(next));
    }

    fn playlist_prev(&mut self) {
        let len = self.scheduler.playlist_len();
        if len == 0 {
            return;
        }
        let current = self.playlist_state.selected().unwrap_or(0);
        let next = if current == 0 { len - 1 } else { current - 1 };
        self.playlist_state.select(Some(next));
    }

    fn play_selected(&mut self) {
        let Some(idx) = self.playlist_state.selected() else {
            return;
        };
        let Some(item) = self.scheduler.playlist_item_at(idx) else {
            return;
        };
        let Some(display) = &self.selected_display else {
            self.status = "No display selected. Press 'd' to choose one.".into();
            self.status_time = Some(Instant::now());
            return;
        };
        let _ = self.scheduler.send(Command::PlayItem {
            path: item.path.clone(),
            display: display.clone(),
            mode: PlayMode::Once,
            seconds: 0,
            fit: item.fit,
            audio_device: String::new(),
        });
    }

    fn add_to_playlist(&mut self, path: PathBuf) {
        let item = MediaItem::from_path(path.to_string_lossy().into_owned());
        let _ = self.scheduler.send(Command::AddToPlaylist(item));
        self.status = "Added to playlist.".into();
        self.status_time = Some(Instant::now());
    }

    pub fn tick(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(time) = self.status_time {
            if time.elapsed() > Duration::from_secs(3) {
                self.status = String::new();
                self.status_time = None;
            }
        }
        Ok(())
    }
}

impl FileBrowser {
    fn new(current_dir: PathBuf) -> Self {
        let mut entries = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&current_dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let kind = if path.is_dir() {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                };
                entries.push(DirEntry { name, path, kind });
            }
            entries.sort_by(|a, b| match (a.kind, b.kind) {
                (EntryKind::Dir, _) => std::cmp::Ordering::Less,
                (_, EntryKind::Dir) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            });
        }
        let mut state = ListState::default();
        if !entries.is_empty() {
            state.select(Some(0));
        }
        Self {
            current_dir,
            entries,
            state,
        }
    }

    fn prev(&mut self) {
        let current = self.state.selected().unwrap_or(0);
        if current > 0 {
            self.state.select(Some(current - 1));
        }
    }

    fn next(&mut self) {
        let current = self.state.selected().unwrap_or(0);
        if current + 1 < self.entries.len() {
            self.state.select(Some(current + 1));
        }
    }

    fn go_up(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.refresh();
        }
    }

    fn enter(&mut self) {
        let Some(idx) = self.state.selected() else {
            return;
        };
        let Some(entry) = self.entries.get(idx) else {
            return;
        };
        if entry.kind == EntryKind::Dir {
            self.current_dir = entry.path.clone();
            self.refresh();
        }
    }

    fn refresh(&mut self) {
        let mut entries = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&self.current_dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let kind = if path.is_dir() {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                };
                entries.push(DirEntry { name, path, kind });
            }
            entries.sort_by(|a, b| match (a.kind, b.kind) {
                (EntryKind::Dir, _) => std::cmp::Ordering::Less,
                (_, EntryKind::Dir) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            });
        }
        self.entries = entries;
        self.state.select(Some(0));
    }

    fn selected_entry(&self) -> Option<&DirEntry> {
        let idx = self.state.selected()?;
        self.entries.get(idx)
    }
}
