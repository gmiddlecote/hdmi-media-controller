//! Cues: named, saved shows.
//!
//! A cue is a queue plus the settings it plays with, so putting on a show is
//! one click rather than rebuilding the queue and re-tuning the dwell, caption
//! and output device by hand. Cues live in their own file next to the saved
//! session so they survive restarts, and they are independent of the working
//! queue: playing a cue loads it, and editing what is on screen afterwards
//! does not touch the stored cue.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::logging;
use crate::media::PlaylistEntry;

/// One saved show.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cue {
    /// Stable identifier; empty means "not saved yet", and one is assigned on
    /// the way in.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub entries: Vec<PlaylistEntry>,
    /// How long a still image stays up while this cue plays.
    #[serde(default)]
    pub dwell_millis: u64,
    /// Caption shown on the output windows.
    #[serde(default)]
    pub overlay: String,
    /// Master display for the cue (empty follows whatever is set).
    #[serde(default)]
    pub display: String,
    /// Output device to restore as the system default (see [`crate::audio`]).
    #[serde(default)]
    pub preferred_audio_device: String,
}

impl Cue {
    /// Rejects a cue that could never be played or recognised in the list.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("A cue needs a name.".into());
        }
        if self.entries.is_empty() {
            return Err("A cue needs at least one file.".into());
        }
        Ok(())
    }
}

/// Where the saved cues are written.
pub fn path() -> PathBuf {
    logging::data_dir().join("cues.json")
}

/// Reads the saved cues. A missing or unreadable file reads as no cues, so a
/// corrupt file never stops the app from starting.
pub fn load() -> Vec<Cue> {
    let Ok(bytes) = fs::read(path()) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Writes `cues`, replacing whatever was stored.
pub fn save(cues: &[Cue]) -> Result<(), String> {
    let file = path();
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    }
    let json = serde_json::to_vec_pretty(cues).map_err(|err| err.to_string())?;
    fs::write(file, json).map_err(|err| err.to_string())
}

/// Builds an id that does not collide with the cues already stored.
fn fresh_id(cues: &[Cue], stamp: u128) -> String {
    let mut id = format!("c{stamp:x}");
    let mut suffix = 1;
    while cues.iter().any(|cue| cue.id == id) {
        id = format!("c{stamp:x}-{suffix}");
        suffix += 1;
    }
    id
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

/// Inserts a new cue, or replaces the stored one with the same id in place.
/// The id actually used is returned.
pub fn upsert(cues: &mut Vec<Cue>, mut cue: Cue) -> Result<String, String> {
    cue.name = cue.name.trim().to_string();
    cue.validate()?;
    let assigned = match cues
        .iter()
        .position(|stored| !cue.id.is_empty() && stored.id == cue.id)
    {
        Some(index) => {
            let id = cues[index].id.clone();
            cue.id = id.clone();
            cues[index] = cue;
            id
        }
        None => {
            let id = fresh_id(cues, now_nanos());
            cue.id = id.clone();
            cues.push(cue);
            id
        }
    };
    Ok(assigned)
}

/// Removes the cue with `id`, reporting whether one went.
pub fn remove(cues: &mut Vec<Cue>, id: &str) -> bool {
    let before = cues.len();
    cues.retain(|cue| cue.id != id);
    cues.len() != before
}

#[cfg(test)]
mod tests {
    use super::{fresh_id, remove, upsert, Cue};

    fn entry(path: &str) -> crate::media::PlaylistEntry {
        crate::media::PlaylistEntry {
            path: path.into(),
            ..Default::default()
        }
    }

    fn cue(name: &str) -> Cue {
        Cue {
            name: name.into(),
            entries: vec![entry("a.jpg")],
            ..Default::default()
        }
    }

    #[test]
    fn a_new_cue_is_given_an_id() {
        let mut cues = Vec::new();
        let id = upsert(&mut cues, cue("Lobby")).unwrap();
        assert!(!id.is_empty());
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].id, id);
    }

    #[test]
    fn saving_the_same_id_replaces_in_place() {
        let mut cues = Vec::new();
        let id = upsert(&mut cues, cue("Lobby")).unwrap();
        let mut edited = cue("Lobby (renamed)");
        edited.id = id.clone();
        edited.entries.push(entry("b.mp4"));
        upsert(&mut cues, edited).unwrap();

        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].id, id);
        assert_eq!(cues[0].name, "Lobby (renamed)");
        assert_eq!(cues[0].entries.len(), 2);
    }

    #[test]
    fn a_blank_id_appends_rather_than_replacing() {
        let mut cues = Vec::new();
        upsert(&mut cues, cue("Lobby")).unwrap();
        upsert(&mut cues, cue("Stage")).unwrap();
        assert_eq!(cues.len(), 2);
    }

    #[test]
    fn names_are_trimmed() {
        let mut cues = Vec::new();
        upsert(&mut cues, cue("  Lobby  ")).unwrap();
        assert_eq!(cues[0].name, "Lobby");
    }

    #[test]
    fn a_nameless_cue_is_rejected() {
        let mut cues = Vec::new();
        assert!(upsert(&mut cues, cue("   ")).is_err());
        assert!(cues.is_empty());
    }

    #[test]
    fn an_empty_cue_is_rejected() {
        let mut cues = Vec::new();
        let mut empty = cue("Lobby");
        empty.entries.clear();
        assert!(upsert(&mut cues, empty).is_err());
        assert!(cues.is_empty());
    }

    #[test]
    fn a_rejected_cue_leaves_the_stored_ones_alone() {
        let mut cues = Vec::new();
        upsert(&mut cues, cue("Lobby")).unwrap();
        let mut broken = cue("Stage");
        broken.id = cues[0].id.clone();
        broken.entries.clear();
        assert!(upsert(&mut cues, broken).is_err());
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].name, "Lobby");
    }

    #[test]
    fn ids_never_collide() {
        let cues = vec![cue("A"), cue("B")];
        let first = fresh_id(&cues, 7);
        let mut with_first = cues.clone();
        with_first[0].id = first.clone();
        let second = fresh_id(&with_first, 7);
        assert_ne!(first, second);
    }

    #[test]
    fn remove_reports_whether_a_cue_went() {
        let mut cues = Vec::new();
        let id = upsert(&mut cues, cue("Lobby")).unwrap();
        assert!(remove(&mut cues, &id));
        assert!(cues.is_empty());
        assert!(!remove(&mut cues, &id));
    }

    #[test]
    fn a_cue_round_trips_through_json() {
        let mut original = cue("Lobby");
        original.dwell_millis = 5000;
        original.overlay = "Welcome".into();
        original.display = "DISPLAY1".into();
        original.preferred_audio_device = "device-a".into();
        let json = serde_json::to_string(&original).unwrap();
        let back: Cue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn a_cue_keeps_its_ticking_so_a_partly_built_cue_survives_a_restart() {
        let mut original = cue("Lobby");
        original.entries.push(entry("b.jpg"));
        original.entries[1].selected = false;
        let json = serde_json::to_string(&original).unwrap();
        let back: Cue = serde_json::from_str(&json).unwrap();
        assert!(back.entries[0].selected);
        assert!(!back.entries[1].selected);
    }

    #[test]
    fn an_older_cue_without_the_newer_fields_still_loads() {
        let json = r#"{"name":"Lobby","entries":[{"path":"a.jpg"}]}"#;
        let cue: Cue = serde_json::from_str(json).unwrap();
        assert_eq!(cue.name, "Lobby");
        assert_eq!(cue.entries.len(), 1);
        assert!(cue.dwell_millis == 0);
        assert!(cue.overlay.is_empty());
    }
}
