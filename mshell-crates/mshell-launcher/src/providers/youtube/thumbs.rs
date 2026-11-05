//! Thumbnail cache for `yt` rows: `$XDG_CACHE_HOME/margo/youtube-thumbs/`.
//!
//! Rows ask [`Thumbs::get`] for a video / channel image; a cached file is
//! returned at once, a missing one is queued for download and the row
//! shows its themed icon until the [`RefreshNotifier`] repaints. Up to
//! [`WORKERS`] downloads run at a time, each on a worker thread, never on
//! the GTK thread. Downloads:
//!
//! * only from YouTube's image hosts (re-checked here, not just when the
//!   listing was parsed), with redirects off so a host can't bounce us
//!   elsewhere;
//! * capped at [`MAX_BYTES`] and accepted only as JPEG / PNG (by magic
//!   bytes — never cache an HTML error page as an image);
//! * attempted once per key per session.
//!
//! [`prune`] keeps the cache near [`CACHE_BUDGET`] and drops images
//! unused for [`MAX_AGE`] (run off-thread once per launcher session).

use crate::provider::RefreshNotifier;
use std::collections::{HashSet, VecDeque};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

const WORKERS: usize = 4;
const MAX_BYTES: u64 = 512 * 1024;
pub const CACHE_BUDGET: u64 = 64 * 1024 * 1024;
pub const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);

pub fn cache_dir() -> PathBuf {
    margo_fs::xdg::cache_home()
        .join("margo")
        .join("youtube-thumbs")
}

/// File name for a cache key (a video / channel id — validated, so it
/// can't escape the directory).
fn file_name(key: &str) -> Option<String> {
    let ok = !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    ok.then(|| format!("{key}.img"))
}

/// Hosts a thumbnail may come from.
pub fn allowed_host(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    matches!(
        host,
        "i.ytimg.com" | "img.youtube.com" | "yt3.ggpht.com" | "yt3.googleusercontent.com"
    )
}

/// JPEG or PNG by magic bytes.
pub fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

#[derive(Default)]
struct Queue {
    pending: VecDeque<(String, String, PathBuf)>,
    active: usize,
}

pub struct Thumbs {
    dir: PathBuf,
    /// Download missing images (off in tests: no network).
    fetch: bool,
    attempted: Arc<Mutex<HashSet<String>>>,
    queue: Arc<Mutex<Queue>>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
}

impl Thumbs {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            fetch: true,
            attempted: Arc::default(),
            queue: Arc::default(),
            notifier: Arc::default(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Downloads enabled (not [`Self::offline`]).
    pub fn fetching(&self) -> bool {
        self.fetch
    }

    /// Serve cached files only; never download.
    pub fn offline(dir: PathBuf) -> Self {
        Self {
            fetch: false,
            ..Self::new(dir)
        }
    }

    pub fn set_notifier(&self, notifier: RefreshNotifier) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    /// The cached image for `key`, queueing a download of `url` when it
    /// is missing. Never blocks.
    pub fn get(&self, key: &str, url: Option<&str>) -> Option<PathBuf> {
        let path = self.dir.join(file_name(key)?);
        if path.is_file() {
            return Some(path);
        }
        if !self.fetch {
            return None;
        }
        let url = url.filter(|u| allowed_host(u))?;
        let first = self
            .attempted
            .lock()
            .map(|mut set| set.insert(key.to_string()))
            .unwrap_or(false);
        if first {
            self.enqueue(key.to_string(), url.to_string(), path);
        }
        None
    }

    fn enqueue(&self, key: String, url: String, path: PathBuf) {
        let spawn = match self.queue.lock() {
            Ok(mut q) => {
                q.pending.push_back((key, url, path));
                if q.active < WORKERS {
                    q.active += 1;
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        };
        if spawn {
            let queue = Arc::clone(&self.queue);
            let notifier = Arc::clone(&self.notifier);
            std::thread::spawn(move || worker(queue, notifier));
        }
    }
}

/// Drain the queue; repaint every few images and when it runs dry.
fn worker(queue: Arc<Mutex<Queue>>, notifier: Arc<Mutex<Option<RefreshNotifier>>>) {
    let notify = || {
        if let Ok(slot) = notifier.lock()
            && let Some(n) = slot.as_ref()
        {
            n();
        }
    };
    let mut fetched = 0usize;
    loop {
        let job = match queue.lock() {
            Ok(mut q) => match q.pending.pop_front() {
                Some(job) => job,
                None => {
                    q.active -= 1;
                    break;
                }
            },
            Err(_) => return,
        };
        let (key, url, path) = job;
        match download(&url, &path) {
            Ok(()) => {
                fetched += 1;
                if fetched.is_multiple_of(4) {
                    notify();
                }
            }
            Err(err) => tracing::debug!(%key, %err, "yt: thumbnail fetch failed"),
        }
    }
    if !fetched.is_multiple_of(4) {
        notify();
    }
}

/// Fetch `url` into `dest` (atomic rename). Blocking.
fn download(url: &str, dest: &Path) -> Result<(), String> {
    if !allowed_host(url) {
        return Err("host not allowed".into());
    }
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(6))
        .build();
    let resp = agent.get(url).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("image too large".into());
    }
    if !is_image(&bytes) {
        return Err("not a JPEG/PNG".into());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    margo_fs::atomic_write::atomic_write(dest, &bytes).map_err(|e| e.to_string())
}

/// Files to delete: older than `max_age`, then the least recently
/// modified until the rest fits `budget`. Pure (tests pass the listing).
pub fn prune_plan(
    mut files: Vec<(PathBuf, u64, SystemTime)>,
    now: SystemTime,
    budget: u64,
    max_age: Duration,
) -> Vec<PathBuf> {
    files.sort_by_key(|(_, _, mtime)| *mtime);
    let mut doomed = Vec::new();
    let mut kept: Vec<(PathBuf, u64)> = Vec::new();
    for (path, size, mtime) in files {
        let age = now.duration_since(mtime).unwrap_or_default();
        if age > max_age {
            doomed.push(path);
        } else {
            kept.push((path, size));
        }
    }
    let mut total: u64 = kept.iter().map(|(_, s)| s).sum();
    for (path, size) in kept {
        if total <= budget {
            break;
        }
        total -= size;
        doomed.push(path);
    }
    doomed
}

/// Apply [`prune_plan`] to `dir` (blocking — worker thread).
pub fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let files: Vec<(PathBuf, u64, SystemTime)> = rd
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| {
                (
                    e.path(),
                    meta.len(),
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                )
            })
        })
        .collect();
    for path in prune_plan(files, SystemTime::now(), CACHE_BUDGET, MAX_AGE) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_cannot_escape_the_cache_dir() {
        assert_eq!(file_name("5C_HPTJg5ek").as_deref(), Some("5C_HPTJg5ek.img"));
        assert_eq!(file_name("../etc/passwd"), None);
        assert_eq!(file_name(""), None);
    }

    #[test]
    fn hosts_and_magic() {
        assert!(allowed_host("https://i.ytimg.com/vi/x/mqdefault.jpg"));
        assert!(allowed_host("https://yt3.ggpht.com/abc=s88"));
        assert!(!allowed_host("http://i.ytimg.com/vi/x/mqdefault.jpg"));
        assert!(!allowed_host("https://i.ytimg.com.evil.test/x"));
        assert!(is_image(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(is_image(b"\x89PNG\r\n\x1a\nrest"));
        assert!(!is_image(b"<!DOCTYPE html>"));
    }

    #[test]
    fn get_serves_cached_files_and_skips_disallowed_urls() {
        let dir = std::env::temp_dir().join(format!("mshell-yt-thumbs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("5C_HPTJg5ek.img"), [0xFF, 0xD8, 0xFF]).unwrap();
        let t = Thumbs::new(dir.clone());
        assert_eq!(
            t.get("5C_HPTJg5ek", None),
            Some(dir.join("5C_HPTJg5ek.img"))
        );
        // Not cached and not an allowed host: no download is queued.
        assert_eq!(t.get("IT0187cHGfI", Some("https://evil.test/x.jpg")), None);
        assert!(t.queue.lock().unwrap().pending.is_empty());
        assert!(t.attempted.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_drops_stale_then_oldest_over_budget() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100 * 86_400);
        let day = |d: u64| now - Duration::from_secs(d * 86_400);
        let files = vec![
            (PathBuf::from("stale"), 10, day(40)),
            (PathBuf::from("old"), 60, day(5)),
            (PathBuf::from("mid"), 30, day(2)),
            (PathBuf::from("new"), 30, day(1)),
        ];
        let doomed = prune_plan(files, now, 70, MAX_AGE);
        assert_eq!(doomed, [PathBuf::from("stale"), PathBuf::from("old")]);
    }
}
