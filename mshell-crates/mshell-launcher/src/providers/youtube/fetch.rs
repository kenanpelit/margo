//! Background yt-dlp listings for the `yt` provider.
//!
//! `search` runs on the GTK thread on every keystroke, so listings are
//! served from a query-keyed cache and fetched on worker threads: the
//! first ask for a key returns [`Fetch::Loading`] (the provider shows a
//! "Searching…" row), the worker waits [`SETTLE`] — a newer key in the
//! meantime aborts it before yt-dlp ever runs (the debounce) — then runs
//! yt-dlp with a hard [`TIMEOUT`], stores the result and fires the
//! [`RefreshNotifier`] so the rows appear.

use super::listing::{Entry, Request, parse_listing, ytdlp_args};
use crate::provider::RefreshNotifier;
use std::collections::{HashMap, VecDeque};
use std::io::Read as _;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Debounce before a fetch starts.
const SETTLE: Duration = Duration::from_millis(350);
/// Hard cap on one yt-dlp run.
const TIMEOUT: Duration = Duration::from_secs(15);
/// Listings kept for instant back-navigation.
const KEEP: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub enum Fetch {
    Loading,
    Ready(Vec<Entry>),
    Failed(String),
}

#[derive(Default)]
struct Cache {
    map: HashMap<String, Fetch>,
    order: VecDeque<String>,
    /// The key most recently asked for; a worker for any other key that
    /// is still settling gives up.
    wanted: String,
}

impl Cache {
    fn store(&mut self, key: String, value: Fetch) {
        if !self.map.contains_key(&key) {
            self.order.push_back(key.clone());
        }
        self.map.insert(key, value);
        while self.order.len() > KEEP {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if old == self.wanted {
                self.order.push_back(old); // never evict what's on screen
            } else {
                self.map.remove(&old);
            }
        }
    }
}

pub struct Fetcher {
    cache: Arc<Mutex<Cache>>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher {
    pub fn new() -> Self {
        Self {
            cache: Arc::default(),
            notifier: Arc::default(),
        }
    }

    pub fn set_notifier(&self, notifier: RefreshNotifier) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    /// Forget failed listings so reopening the launcher retries them.
    pub fn drop_failures(&self) {
        if let Ok(mut c) = self.cache.lock() {
            c.map.retain(|_, v| !matches!(v, Fetch::Failed(_)));
        }
    }

    /// The cached listing for `key`, without starting a fetch.
    pub fn peek(&self, key: &str) -> Option<Fetch> {
        let mut c = self.cache.lock().ok()?;
        let hit = c.map.get(key).cloned()?;
        c.wanted = key.to_string();
        Some(hit)
    }

    /// The listing for `key` (the concatenation of `requests`, each
    /// limited to `limit` rows), fetching it off-thread when missing.
    pub fn get(&self, key: &str, requests: Vec<Request>, limit: usize) -> Fetch {
        {
            let Ok(mut c) = self.cache.lock() else {
                return Fetch::Failed("internal lock poisoned".into());
            };
            c.wanted = key.to_string();
            if let Some(v) = c.map.get(key) {
                return v.clone();
            }
            c.store(key.to_string(), Fetch::Loading);
        }
        let cache = Arc::clone(&self.cache);
        let notifier = Arc::clone(&self.notifier);
        let key = key.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(SETTLE);
            let superseded = cache.lock().map(|c| c.wanted != key).unwrap_or(true);
            if superseded {
                // Typed past it: forget the placeholder so coming back
                // to this key fetches again.
                if let Ok(mut c) = cache.lock()
                    && c.map.get(&key) == Some(&Fetch::Loading)
                {
                    c.map.remove(&key);
                }
                return;
            }
            let result = run_all(&requests, limit);
            if let Ok(mut c) = cache.lock() {
                c.store(key, result);
            }
            if let Ok(slot) = notifier.lock()
                && let Some(n) = slot.as_ref()
            {
                n();
            }
        });
        Fetch::Loading
    }

    #[cfg(test)]
    pub fn insert(&self, key: &str, value: Fetch) {
        if let Ok(mut c) = self.cache.lock() {
            c.store(key.to_string(), value);
        }
    }
}

/// Run every request (in parallel) and concatenate, in request order.
/// One failing channel in a feed doesn't sink the others; the error is
/// reported only when nothing came back.
fn run_all(requests: &[Request], limit: usize) -> Fetch {
    let handles: Vec<_> = requests
        .iter()
        .cloned()
        .map(|req| std::thread::spawn(move || run_ytdlp(&req, limit)))
        .collect();
    let mut entries = Vec::new();
    let mut first_err = None;
    for h in handles {
        match h.join() {
            Ok(Ok(mut e)) => entries.append(&mut e),
            Ok(Err(err)) => {
                first_err.get_or_insert(err);
            }
            Err(_) => {
                first_err.get_or_insert_with(|| "yt-dlp worker panicked".to_string());
            }
        }
    }
    match first_err {
        Some(err) if entries.is_empty() => Fetch::Failed(err),
        _ => Fetch::Ready(entries),
    }
}

/// One yt-dlp listing, killed after [`TIMEOUT`]. Blocking.
fn run_ytdlp(req: &Request, limit: usize) -> Result<Vec<Entry>, String> {
    let mut child = Command::new("yt-dlp")
        .args(ytdlp_args(req, limit))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("yt-dlp: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("yt-dlp: no stdout")?;
    let stderr = child.stderr.take().ok_or("yt-dlp: no stderr")?;
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.take(64 * 1024).read_to_string(&mut s);
        s
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("yt-dlp: {e}")),
        }
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    match status {
        None => Err("YouTube did not answer in time".into()),
        Some(s) if !s.success() => Err(error_line(&stderr)),
        Some(_) => parse_listing(&stdout),
    }
}

/// The useful line of a yt-dlp failure (`ERROR: …`), shortened.
pub fn error_line(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("yt-dlp failed");
    let line = line.trim_start_matches("ERROR:").trim();
    let mut short: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        short.push('…');
    }
    short
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_line_prefers_the_error() {
        let stderr = "WARNING: x\nERROR: [youtube] abc: Sign in to confirm you're not a bot\n";
        assert_eq!(
            error_line(stderr),
            "[youtube] abc: Sign in to confirm you're not a bot"
        );
        assert_eq!(error_line(""), "yt-dlp failed");
        assert!(error_line(&"x".repeat(400)).ends_with('…'));
    }

    #[test]
    fn cache_serves_ready_results_and_evicts_old_keys() {
        let f = Fetcher::new();
        f.insert("a", Fetch::Ready(vec![]));
        assert_eq!(f.get("a", vec![], 1), Fetch::Ready(vec![]));
        for i in 0..(KEEP + 3) {
            f.insert(&format!("k{i}"), Fetch::Failed("x".into()));
        }
        let c = f.cache.lock().unwrap();
        assert!(c.map.len() <= KEEP + 1);
        assert!(!c.map.contains_key("k0"));
    }

    #[test]
    fn failures_are_retried_after_reopen() {
        let f = Fetcher::new();
        f.insert("q", Fetch::Failed("timeout".into()));
        f.drop_failures();
        assert!(!f.cache.lock().unwrap().map.contains_key("q"));
    }
}
