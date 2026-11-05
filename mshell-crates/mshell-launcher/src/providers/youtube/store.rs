//! The `yt` provider's saved state, both under
//! `$XDG_STATE_HOME/margo/youtube/`:
//!
//! * `library.json` — **Watch later** and **favourite channels**; owned
//!   (read + written) by the launcher, through `margo_fs::side_file` so a
//!   file that stops parsing is backed up and never reset.
//! * `progress.json` — history + resume positions; written only by
//!   `mshellctl` (its `play` command and the `__track` tracker, under a
//!   lock), read here. Shape: `mshellctl/src/mpv/youtube.rs`, shared
//!   fixture `tests/fixtures/youtube-progress.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Watch-later entries kept.
const WATCH_LATER_CAP: usize = 500;
/// Below this a saved position is not "continue watching".
const MIN_RESUME_SECS: f64 = 10.0;
/// Closer than this to the end counts as watched.
const END_MARGIN_SECS: f64 = 15.0;

pub fn state_dir() -> PathBuf {
    margo_fs::xdg::state_home().join("margo").join("youtube")
}

/// A saved video.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub channel_id: String,
    pub duration: Option<f64>,
}

/// A favourite channel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Channel {
    pub id: String,
    pub name: String,
    /// Allow-listed avatar URL, when known.
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub version: u32,
    /// Newest first.
    pub watch_later: Vec<Video>,
    /// In the order they were added.
    pub favourites: Vec<Channel>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            version: 1,
            watch_later: Vec::new(),
            favourites: Vec::new(),
        }
    }
}

impl Library {
    pub fn is_saved(&self, id: &str) -> bool {
        self.watch_later.iter().any(|v| v.id == id)
    }

    pub fn is_favourite(&self, channel_id: &str) -> bool {
        self.favourites.iter().any(|c| c.id == channel_id)
    }

    /// Add to / remove from Watch later; `true` when it is now saved.
    pub fn toggle_watch_later(&mut self, video: Video) -> bool {
        if let Some(i) = self.watch_later.iter().position(|v| v.id == video.id) {
            self.watch_later.remove(i);
            return false;
        }
        self.watch_later.insert(0, video);
        self.watch_later.truncate(WATCH_LATER_CAP);
        true
    }

    pub fn remove_watch_later(&mut self, id: &str) -> bool {
        let before = self.watch_later.len();
        self.watch_later.retain(|v| v.id != id);
        before != self.watch_later.len()
    }

    /// Add / remove a favourite channel; `true` when it is now a favourite.
    pub fn toggle_favourite(&mut self, channel: Channel) -> bool {
        if let Some(i) = self.favourites.iter().position(|c| c.id == channel.id) {
            self.favourites.remove(i);
            return false;
        }
        self.favourites.push(channel);
        true
    }
}

/// `library.json` with the side-file rules.
pub struct LibraryFile {
    path: PathBuf,
}

/// Outcome of reading `library.json`.
pub struct Loaded {
    pub library: Library,
    /// The file exists but does not parse: shown read-only, never saved
    /// over (a `.bak` copy was made).
    pub broken: Option<String>,
}

impl LibraryFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_location() -> Self {
        Self::new(state_dir().join("library.json"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Loaded {
        match margo_fs::side_file::load(&self.path, |raw: &str| {
            serde_json::from_str::<Library>(raw)
        }) {
            margo_fs::side_file::SideLoad::Loaded(library) => Loaded {
                library,
                broken: None,
            },
            margo_fs::side_file::SideLoad::Missing => Loaded {
                library: Library::default(),
                broken: None,
            },
            margo_fs::side_file::SideLoad::Broken { error, .. } => Loaded {
                library: Library::default(),
                broken: Some(error),
            },
        }
    }

    pub fn save(&self, library: &Library) -> Result<(), String> {
        let body = serde_json::to_vec_pretty(library).map_err(|e| e.to_string())?;
        margo_fs::side_file::save(&self.path, &body, |raw| {
            serde_json::from_str::<Library>(raw).map(|_| ())
        })
        .map_err(|e| e.to_string())
    }
}

/// One history / resume record (written by `mshellctl`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ProgressEntry {
    pub title: String,
    pub channel: String,
    pub channel_id: String,
    pub duration: Option<f64>,
    pub position: f64,
    pub completed: bool,
    pub last_played: u64,
}

impl ProgressEntry {
    /// Worth a "continue watching" row: started, not near the end.
    pub fn resumable(&self) -> bool {
        if self.completed || self.position < MIN_RESUME_SECS {
            return false;
        }
        self.duration
            .is_none_or(|d| self.position <= d - END_MARGIN_SECS)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Progress {
    pub version: u32,
    pub videos: BTreeMap<String, ProgressEntry>,
}

impl Progress {
    /// Read `progress.json` (missing or unreadable → empty).
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn default_path() -> PathBuf {
        state_dir().join("progress.json")
    }

    /// Played videos, most recent first.
    pub fn history(&self) -> Vec<(&str, &ProgressEntry)> {
        let mut out: Vec<(&str, &ProgressEntry)> = self
            .videos
            .iter()
            .filter(|(_, e)| e.last_played > 0)
            .map(|(id, e)| (id.as_str(), e))
            .collect();
        out.sort_by(|a, b| b.1.last_played.cmp(&a.1.last_played).then(a.0.cmp(b.0)));
        out
    }

    /// Started but unfinished videos, most recent first.
    pub fn continue_watching(&self) -> Vec<(&str, &ProgressEntry)> {
        self.history()
            .into_iter()
            .filter(|(_, e)| e.resumable())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRESS: &str = include_str!("../../../tests/fixtures/youtube-progress.json");

    fn video(id: &str) -> Video {
        Video {
            id: id.into(),
            title: format!("t-{id}"),
            ..Video::default()
        }
    }

    #[test]
    fn watch_later_toggles_newest_first() {
        let mut lib = Library::default();
        assert!(lib.toggle_watch_later(video("aaaaaaaaaaa")));
        assert!(lib.toggle_watch_later(video("bbbbbbbbbbb")));
        assert_eq!(lib.watch_later[0].id, "bbbbbbbbbbb");
        assert!(lib.is_saved("aaaaaaaaaaa"));
        assert!(!lib.toggle_watch_later(video("aaaaaaaaaaa")));
        assert!(!lib.is_saved("aaaaaaaaaaa"));
        assert!(lib.remove_watch_later("bbbbbbbbbbb"));
        assert!(!lib.remove_watch_later("bbbbbbbbbbb"));
    }

    #[test]
    fn favourites_toggle_in_insertion_order() {
        let mut lib = Library::default();
        let ch = |id: &str| Channel {
            id: id.into(),
            name: id.into(),
            image: None,
        };
        assert!(lib.toggle_favourite(ch("UC1")));
        assert!(lib.toggle_favourite(ch("UC2")));
        assert_eq!(lib.favourites[0].id, "UC1");
        assert!(!lib.toggle_favourite(ch("UC1")));
        assert!(!lib.is_favourite("UC1") && lib.is_favourite("UC2"));
    }

    fn tmp() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "mshell-yt-store-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn library_file_round_trips_and_protects_a_broken_file() {
        let dir = tmp();
        let file = LibraryFile::new(dir.join("library.json"));
        let mut lib = file.load().library;
        lib.toggle_watch_later(video("5C_HPTJg5ek"));
        file.save(&lib).unwrap();
        assert_eq!(file.load().library, lib);

        std::fs::write(file.path(), "{ broken").unwrap();
        let loaded = file.load();
        assert!(loaded.broken.is_some());
        assert!(file.save(&Library::default()).is_err());
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), "{ broken");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_fixture_history_and_continue_watching() {
        let dir = tmp();
        let path = dir.join("progress.json");
        std::fs::write(&path, PROGRESS).unwrap();
        let p = Progress::load(&path);
        let history: Vec<&str> = p.history().iter().map(|(id, _)| *id).collect();
        assert_eq!(history, ["Quf05tmWjaQ", "5C_HPTJg5ek", "IT0187cHGfI"]);
        // Completed and barely-started videos are not "continue".
        let cont: Vec<&str> = p.continue_watching().iter().map(|(id, _)| *id).collect();
        assert_eq!(cont, ["5C_HPTJg5ek"]);
        assert_eq!(
            Progress::load(&dir.join("missing.json")),
            Progress::default()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resumable_rules() {
        let e = |position, duration, completed| ProgressEntry {
            position,
            duration,
            completed,
            ..ProgressEntry::default()
        };
        assert!(e(60.0, Some(600.0), false).resumable());
        assert!(e(60.0, None, false).resumable());
        assert!(!e(5.0, Some(600.0), false).resumable());
        assert!(!e(590.0, Some(600.0), false).resumable());
        assert!(!e(60.0, Some(600.0), true).resumable());
    }
}
