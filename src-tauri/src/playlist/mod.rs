//! Playlists: ordered collections of files the scheduler steps through.
//!
//! A [`Playlist`] owns a queue of [`crate::media::MediaItem`]s plus a
//! cursor. Advancing honors the current [`RepeatMode`]: `Off` stops at the
//! end, `One` always returns the same item, and `All` wraps around.
//!
//! The playlist is deliberately kept free of scheduling concerns so it can
//! be driven by the [`crate::scheduler`] kernel or unit-tested in isolation.

use crate::media::MediaItem;

/// How a playlist behaves once it reaches its last entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RepeatMode {
    /// Stop advancing when the playlist is exhausted.
    #[default]
    Off,
    /// Keep replaying the current item forever.
    One,
    /// Wrap around and keep going.
    All,
}

/// An ordered media queue with a playback cursor.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Playlist {
    items: Vec<MediaItem>,
    index: usize,
    repeat: RepeatMode,
}

impl Playlist {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of queued items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Item the cursor currently points at, if any.
    pub fn current(&self) -> Option<&MediaItem> {
        self.items.get(self.index)
    }

    /// Borrows the queued items as a slice.
    pub fn items(&self) -> &[MediaItem] {
        &self.items
    }

    /// First queue position holding the given path, if any.
    pub fn position(&self, path: &str) -> Option<usize> {
        self.items.iter().position(|item| item.path == path)
    }

    /// Cursor position (index into the queue).
    pub fn index(&self) -> usize {
        self.index
    }

    /// Replaces the whole queue, keeping the cursor in range.
    pub fn replace(&mut self, items: Vec<MediaItem>) {
        self.items = items;
        self.index = self.index.min(self.items.len().saturating_sub(1));
    }

    /// Appends a batch of items to the queue.
    pub fn extend(&mut self, items: Vec<MediaItem>) {
        if self.items.is_empty() {
            self.items = items;
        } else {
            self.items.extend(items);
        }
    }

    /// Removes the item at `index`; returns it when present.
    pub fn remove(&mut self, index: usize) -> Option<MediaItem> {
        if index >= self.items.len() {
            return None;
        }
        let removed = self.items.remove(index);
        if self.index >= index {
            self.index = self
                .index
                .saturating_sub(1)
                .min(self.items.len().saturating_sub(1));
        }
        Some(removed)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.index = 0;
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    /// Advances to the next item. Returns `false` when the playlist is
    /// exhausted (`RepeatMode::Off`) — after that the cursor stays put.
    ///
    /// * `RepeatMode::Off`: `true` until the last item was returned.
    /// * `RepeatMode::One`: always `true`, cursor never moves.
    /// * `RepeatMode::All`: always `true`, wrapping at the end.
    pub fn advance(&mut self) -> bool {
        match self.repeat {
            RepeatMode::One => true,
            RepeatMode::All => {
                self.index = (self.index + 1) % self.items.len();
                true
            }
            RepeatMode::Off => {
                if self.index + 1 >= self.items.len() {
                    false
                } else {
                    self.index += 1;
                    true
                }
            }
        }
    }

    /// Moves the cursor one item back. `RepeatMode::All` wraps around to the
    /// end; the other modes clamp to the first item.
    pub fn prev(&mut self) {
        match self.repeat {
            RepeatMode::All if !self.items.is_empty() => {
                self.index = (self.index + self.items.len() - 1) % self.items.len();
            }
            _ => {
                self.index = self.index.max(1) - 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(names: &[&str]) -> Vec<MediaItem> {
        names
            .iter()
            .map(|name| MediaItem::from_path(format!("/x/{name}.png")))
            .collect()
    }

    #[test]
    fn cursor_walks_forward_and_stops_at_end() {
        let mut playlist = Playlist::new();
        playlist.replace(media(&["a", "b", "c"]));
        playlist.set_repeat(RepeatMode::Off);

        assert_eq!(playlist.current().unwrap().name, "a.png");
        assert!(playlist.advance());
        assert_eq!(playlist.current().unwrap().name, "b.png");
        assert!(playlist.advance());
        assert_eq!(playlist.current().unwrap().name, "c.png");
        assert!(!playlist.advance());
        assert_eq!(playlist.current().unwrap().name, "c.png");
    }

    #[test]
    fn repeat_all_wraps() {
        let mut playlist = Playlist::new();
        playlist.replace(media(&["a", "b"]));
        playlist.set_repeat(RepeatMode::All);

        assert!(playlist.advance());
        assert_eq!(playlist.current().unwrap().name, "b.png");
        assert!(playlist.advance());
        assert_eq!(playlist.current().unwrap().name, "a.png");
    }

    #[test]
    fn repeat_one_keeps_item() {
        let mut playlist = Playlist::new();
        playlist.replace(media(&["a", "b"]));
        playlist.set_repeat(RepeatMode::One);

        playlist.advance();
        assert_eq!(playlist.current().unwrap().name, "a.png");
    }

    #[test]
    fn prev_moves_backward_and_clamps() {
        let mut playlist = Playlist::new();
        playlist.replace(media(&["a", "b", "c"]));
        playlist.set_repeat(RepeatMode::Off);

        playlist.advance();
        playlist.advance();
        assert_eq!(playlist.current().unwrap().name, "c.png");
        playlist.prev();
        assert_eq!(playlist.current().unwrap().name, "b.png");
        playlist.prev();
        playlist.prev();
        assert_eq!(playlist.current().unwrap().name, "a.png");
    }

    #[test]
    fn remove_reindexes_cursor() {
        let mut playlist = Playlist::new();
        playlist.replace(media(&["a", "b", "c"]));
        playlist.advance();
        playlist.remove(0);
        assert_eq!(playlist.index(), 0);
        assert_eq!(playlist.current().unwrap().name, "b.png");
    }

    #[test]
    fn position_locates_items_and_items_borrows_queue() {
        let mut playlist = Playlist::new();
        playlist.replace(media(&["a", "b", "c"]));
        assert_eq!(playlist.position("/x/b.png"), Some(1));
        assert_eq!(playlist.position("/x/nope.png"), None);
        assert_eq!(
            playlist
                .items()
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a.png", "b.png", "c.png"]
        );
    }
}
