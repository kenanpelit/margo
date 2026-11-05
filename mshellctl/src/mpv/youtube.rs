//! YouTube playback state shared with the launcher's `yt` provider:
//! video-id extraction, the resume/history store (`progress.json`) and the
//! resume decision.
//!
//! `progress.json` lives in `$XDG_STATE_HOME/margo/youtube/` and is
//! written only by `mshellctl` (the `play` hints and the `__track`
//! position tracker), always as a locked read-modify-write so the two
//! never clobber each other. The launcher only reads it (history +
//! "continue watching") and asks `mshellctl play forget` to drop entries.
//! The on-disk shape is the contract with
//! `mshell-launcher/src/providers/youtube/progress.rs`; both sides test
//! against `tests/fixtures/youtube-progress.json` in that crate.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Entries kept; the least recently played are dropped past this.
pub const HISTORY_CAP: usize = 500;
/// A saved position shorter than this is not worth resuming.
const MIN_RESUME_SECS: f64 = 10.0;
/// Closer than this to the end counts as watched — start over.
const END_MARGIN_SECS: f64 = 15.0;
/// Rewind a little on resume so the viewer regains context.
const RESUME_REWIND_SECS: f64 = 2.0;

/// The 11-character id of a YouTube watch / share / shorts / live /
/// embed URL (an mpv `ytdl://` prefix is accepted). `None` otherwise.
pub fn video_id(url: &str) -> Option<String> {
    let url = url.trim();
    let url = url.strip_prefix("ytdl://").unwrap_or(url);
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.strip_prefix("www.").unwrap_or(host);
    let candidate = match host {
        "youtu.be" => path.split(['?', '#', '/']).next(),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => {
            if let Some(query) = path.strip_prefix("watch?") {
                query_param(query, "v")
            } else {
                ["shorts/", "live/", "embed/", "v/"].iter().find_map(|p| {
                    path.strip_prefix(p)
                        .and_then(|r| r.split(['?', '#', '/']).next())
                })
            }
        }
        _ => None,
    }?;
    valid_video_id(candidate).then(|| candidate.to_string())
}

/// `true` for an 11-char `[A-Za-z0-9_-]` id.
pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split(['&', '#'])
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

/// `true` when the URL carries its own start time (`t=` / `start=`),
/// which wins over the saved position.
pub fn has_timestamp(url: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else {
        return false;
    };
    query
        .split(['&', '#'])
        .filter_map(|kv| kv.split_once('='))
        .any(|(k, v)| (k == "t" || k == "start") && !v.is_empty())
}

/// One video's history + resume record.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProgressEntry {
    pub title: String,
    pub channel: String,
    pub channel_id: String,
    /// Seconds; `None` until known (live streams never have one).
    pub duration: Option<f64>,
    /// Last saved playback position, seconds.
    pub position: f64,
    /// Played to the end: the next play starts over.
    pub completed: bool,
    /// Unix seconds of the last time it was loaded or saved.
    pub last_played: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProgressFile {
    pub version: u32,
    pub videos: BTreeMap<String, ProgressEntry>,
}

impl Default for ProgressFile {
    fn default() -> Self {
        Self {
            version: 1,
            videos: BTreeMap::new(),
        }
    }
}

impl ProgressFile {
    /// Drop the least recently played entries past [`HISTORY_CAP`].
    pub fn trim(&mut self) {
        if self.videos.len() <= HISTORY_CAP {
            return;
        }
        let mut by_age: Vec<(u64, String)> = self
            .videos
            .iter()
            .map(|(id, e)| (e.last_played, id.clone()))
            .collect();
        by_age.sort();
        let excess = self.videos.len() - HISTORY_CAP;
        for (_, id) in by_age.into_iter().take(excess) {
            self.videos.remove(&id);
        }
    }
}

/// Where to start `entry` (seconds), or `None` to start from the top.
pub fn resume_start(entry: &ProgressEntry) -> Option<f64> {
    if entry.completed || entry.position < MIN_RESUME_SECS {
        return None;
    }
    if let Some(d) = entry.duration
        && entry.position > d - END_MARGIN_SECS
    {
        return None;
    }
    Some((entry.position - RESUME_REWIND_SECS).max(0.0))
}

/// Metadata the launcher passes along when it starts a video, so history
/// rows have a channel + duration before mpv reports them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hints {
    pub title: Option<String>,
    pub channel: Option<String>,
    pub channel_id: Option<String>,
    pub duration: Option<f64>,
}

impl Hints {
    pub fn apply(&self, entry: &mut ProgressEntry) {
        let set = |slot: &mut String, v: &Option<String>| {
            if let Some(v) = v.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                *slot = v.to_string();
            }
        };
        set(&mut entry.title, &self.title);
        set(&mut entry.channel, &self.channel);
        set(&mut entry.channel_id, &self.channel_id);
        if let Some(d) = self.duration.filter(|d| *d > 0.0) {
            entry.duration = Some(d);
        }
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `$XDG_STATE_HOME/margo/youtube`.
pub fn state_dir() -> PathBuf {
    margo_fs::xdg::state_home().join("margo").join("youtube")
}

/// The store at `dir/progress.json`. Every mutation is a locked
/// read-modify-write (`flock` on `dir/progress.lock`).
pub struct ProgressStore {
    dir: PathBuf,
}

impl ProgressStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn default_location() -> Self {
        Self::new(state_dir())
    }

    fn file(&self) -> PathBuf {
        self.dir.join("progress.json")
    }

    /// Current contents (empty when missing; a file that doesn't parse
    /// reads as empty and is never written over — see [`Self::update`]).
    pub fn load(&self) -> ProgressFile {
        read(&self.file()).unwrap_or_default()
    }

    /// Apply `f` under the lock and write the result atomically. Refuses
    /// (Err) when the file on disk does not parse, so a hand-edited or
    /// damaged file is kept (with a `.bak` copy) instead of reset.
    pub fn update(&self, f: impl FnOnce(&mut ProgressFile)) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let _lock = FileLock::acquire(&self.dir.join("progress.lock"))?;
        let path = self.file();
        let mut data = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str::<ProgressFile>(&raw).map_err(|e| {
                let _ = margo_fs::side_file::backup_broken(&path);
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "{} does not parse ({e}); not saving over it",
                        path.display()
                    ),
                )
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProgressFile::default(),
            Err(e) => return Err(e),
        };
        f(&mut data);
        data.trim();
        let body = serde_json::to_vec_pretty(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        margo_fs::atomic_write::atomic_write(&path, &body)
    }

    /// Record that `id` started (hints + last-played stamp).
    pub fn touch(&self, id: &str, hints: &Hints) -> std::io::Result<()> {
        let now = now_secs();
        self.update(|data| {
            let entry = data.videos.entry(id.to_string()).or_default();
            hints.apply(entry);
            entry.last_played = now;
        })
    }

    /// Drop `id` (or everything with `None`): history row + resume point.
    pub fn forget(&self, id: Option<&str>) -> std::io::Result<()> {
        self.update(|data| match id {
            Some(id) => {
                data.videos.remove(id);
            }
            None => data.videos.clear(),
        })
    }
}

fn read(path: &Path) -> Option<ProgressFile> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// RAII `flock(LOCK_EX)`.
struct FileLock {
    _file: std::fs::File,
}

impl FileLock {
    fn acquire(path: &Path) -> std::io::Result<Self> {
        use std::os::fd::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        // SAFETY: flock on a file descriptor we own; released on close.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_ids_from_every_url_shape() {
        let id = Some("dQw4w9WgXcQ".to_string());
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=42",
            "ytdl://https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?t=10",
            "https://m.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/live/dQw4w9WgXcQ?si=x",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RD",
        ] {
            assert_eq!(video_id(url), id, "{url}");
        }
        for url in [
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/playlist?list=PLai5B987bZ9CoVR",
            "https://www.youtube.com/channel/UCaYhcUwRBNscFNUKTjgPFiA",
            "https://evil.test/watch?v=dQw4w9WgXcQ",
            "/home/u/clip.mkv",
        ] {
            assert_eq!(video_id(url), None, "{url}");
        }
    }

    #[test]
    fn url_timestamps_are_detected() {
        assert!(has_timestamp("https://youtu.be/dQw4w9WgXcQ?t=10"));
        assert!(has_timestamp("https://youtube.com/watch?v=x&start=3"));
        assert!(!has_timestamp("https://youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(!has_timestamp("https://youtube.com/watch?v=x&t="));
    }

    #[test]
    fn resume_rules() {
        let e = |position, duration, completed| ProgressEntry {
            position,
            duration,
            completed,
            ..Default::default()
        };
        assert_eq!(resume_start(&e(125.0, Some(600.0), false)), Some(123.0));
        assert_eq!(resume_start(&e(125.0, None, false)), Some(123.0));
        // Too early, too close to the end, or watched → from the top.
        assert_eq!(resume_start(&e(4.0, Some(600.0), false)), None);
        assert_eq!(resume_start(&e(590.0, Some(600.0), false)), None);
        assert_eq!(resume_start(&e(300.0, Some(600.0), true)), None);
    }

    fn tmp() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "mshellctl-yt-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn store_touch_update_forget_round_trip() {
        let dir = tmp();
        let store = ProgressStore::new(dir.clone());
        assert_eq!(store.load(), ProgressFile::default());
        let hints = Hints {
            title: Some("Rust in 100 Seconds".into()),
            channel: Some("Fireship".into()),
            channel_id: Some("UCsBjURrPoezykLs9EqgamOA".into()),
            duration: Some(149.0),
        };
        store.touch("5C_HPTJg5ek", &hints).unwrap();
        store
            .update(|d| d.videos.get_mut("5C_HPTJg5ek").unwrap().position = 61.0)
            .unwrap();
        let loaded = store.load();
        let e = &loaded.videos["5C_HPTJg5ek"];
        assert_eq!(e.channel, "Fireship");
        assert_eq!(e.duration, Some(149.0));
        assert_eq!(e.position, 61.0);
        assert!(e.last_played > 0);
        store.forget(Some("5C_HPTJg5ek")).unwrap();
        assert!(store.load().videos.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_file_is_never_written_over() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("progress.json"), "{ not json").unwrap();
        let store = ProgressStore::new(dir.clone());
        assert!(store.touch("5C_HPTJg5ek", &Hints::default()).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("progress.json")).unwrap(),
            "{ not json"
        );
        assert!(dir.join("progress.json.bak").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trim_drops_the_least_recently_played() {
        let mut f = ProgressFile::default();
        for i in 0..(HISTORY_CAP + 3) {
            f.videos.insert(
                format!("id{i:09}"),
                ProgressEntry {
                    last_played: i as u64,
                    ..Default::default()
                },
            );
        }
        f.trim();
        assert_eq!(f.videos.len(), HISTORY_CAP);
        assert!(!f.videos.contains_key("id000000000"));
        assert!(f.videos.contains_key(&format!("id{:09}", HISTORY_CAP + 2)));
    }

    /// The contract with the launcher: this fixture is what the `yt`
    /// provider parses (`mshell-launcher/tests/fixtures`).
    #[test]
    fn shared_fixture_parses_and_round_trips() {
        let raw = include_str!(
            "../../../mshell-crates/mshell-launcher/tests/fixtures/youtube-progress.json"
        );
        let f: ProgressFile = serde_json::from_str(raw).unwrap();
        assert_eq!(f.version, 1);
        let e = &f.videos["5C_HPTJg5ek"];
        assert_eq!(e.title, "Rust in 100 Seconds");
        assert_eq!(e.channel_id, "UCsBjURrPoezykLs9EqgamOA");
        let again: ProgressFile =
            serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(again, f);
    }
}
