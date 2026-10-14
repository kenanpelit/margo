//! Config auto-reload (`auto_reload_config`, default on) — mango port
//! (`ea66bdc5` "feat: auto reload config" + `14168dfd` "opt: make config
//! watcher purely event-driven").
//!
//! Watches the config file plus every file it currently `source`s /
//! `include`s for changes and reloads automatically — no more manual
//! `mctl reload` / `Super+Ctrl+R` after every edit. The watch set is
//! re-synced after each reload (`ConfigWatcher::update`) since a
//! `source`/`include` line can add or drop files from the tree.
//!
//! The *parent directories* are watched, not the files themselves: an
//! editor that saves atomically (write a temp file, rename it over the
//! original — helix, `sed -i`, many vim setups) replaces the inode, and
//! inotify drops a watch on the old inode for good. Events are then
//! filtered against the tracked file set. Both the path as configured
//! and its canonical (symlink-resolved) target are tracked, so a
//! dotfiles-managed `config.conf -> ~/dotfiles/…` symlink reloads on an
//! edit of the target and on the link itself being replaced.
//!
//! Only content-changing event kinds are subscribed to. The reload reads
//! every watched file, so open/close-nowrite events would re-trigger the
//! watcher off its own reload forever.
//!
//! `notify`'s callback runs on its own background thread and
//! `MargoState` isn't `Send`, so this thread only debounces and then
//! calls `on_settled`, which the caller wires to a `calloop::channel`
//! `Sender` — the actual reload runs on the compositor thread.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, EventKindMask, RecommendedWatcher, RecursiveMode, Watcher};

/// Coalesces a burst of filesystem events (an editor's multi-write-per-
/// save, several `source`'d files changing at once) into one reload.
const DEBOUNCE_MS: u64 = 300;

type FileSet = Arc<Mutex<HashSet<PathBuf>>>;

/// Owns the live filesystem watcher. Dropping it stops watching; the
/// background thread exits once the watcher's event sender drops and
/// its channel disconnects.
pub struct ConfigWatcher {
    watcher: RecommendedWatcher,
    dirs: HashSet<PathBuf>,
    files: FileSet,
}

impl ConfigWatcher {
    /// Start watching `paths`. `on_settled` fires once per debounced
    /// batch of relevant changes, from a background thread. Returns
    /// `None` if the watcher itself couldn't be created (e.g. the inotify
    /// instance limit is exhausted) — losing auto-reload beats taking
    /// the compositor down over it.
    pub fn new(paths: &[PathBuf], on_settled: impl Fn() + Send + 'static) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
        let config = notify::Config::default().with_event_kinds(
            EventKindMask::CREATE
                | EventKindMask::REMOVE
                | EventKindMask::MODIFY_DATA
                | EventKindMask::MODIFY_NAME
                | EventKindMask::ACCESS_CLOSE,
        );
        let watcher = match RecommendedWatcher::new(tx, config) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "config watcher: failed to create, auto-reload disabled"
                );
                return None;
            }
        };

        let files: FileSet = Arc::default();
        let mut this = ConfigWatcher {
            watcher,
            dirs: HashSet::new(),
            files: files.clone(),
        };
        this.update(paths);
        std::thread::spawn(move || watch_loop(rx, files, on_settled));
        Some(this)
    }

    /// Re-sync the watch set after a reload — only touches directories
    /// that actually changed instead of recreating the watcher.
    pub fn update(&mut self, paths: &[PathBuf]) {
        let files = tracked_files(paths);
        let dirs: HashSet<PathBuf> = files
            .iter()
            .filter_map(|f| f.parent())
            .filter(|d| !d.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .collect();

        for stale in self.dirs.difference(&dirs) {
            let _ = self.watcher.unwatch(stale);
        }
        let mut watched = HashSet::new();
        for dir in dirs {
            if self.dirs.contains(&dir) {
                watched.insert(dir);
                continue;
            }
            match self.watcher.watch(&dir, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    watched.insert(dir);
                }
                Err(e) => {
                    tracing::warn!(error = %e, dir = %dir.display(), "config watcher: couldn't watch")
                }
            }
        }
        self.dirs = watched;

        match self.files.lock() {
            Ok(mut guard) => *guard = files,
            Err(poisoned) => *poisoned.into_inner() = files,
        }
    }
}

/// Every path an event may arrive under for `paths`: each path as given
/// plus its canonical target when that differs (symlinked dotfiles).
fn tracked_files(paths: &[PathBuf]) -> HashSet<PathBuf> {
    let mut files = HashSet::new();
    for p in paths {
        if let Ok(canon) = std::fs::canonicalize(p) {
            files.insert(canon);
        }
        files.insert(p.clone());
    }
    files
}

fn is_relevant(event: &Event, files: &FileSet) -> bool {
    if matches!(event.kind, EventKind::Access(_))
        && !matches!(
            event.kind,
            EventKind::Access(notify::event::AccessKind::Close(
                notify::event::AccessMode::Write
            ))
        )
    {
        return false;
    }
    let files = match files.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    event.paths.iter().any(|p| files.contains(p))
}

fn watch_loop(rx: mpsc::Receiver<notify::Result<Event>>, files: FileSet, on_settled: impl Fn()) {
    let mut pending = false;
    let mut last_event_at = Instant::now();
    loop {
        // Idle: block for the next event. A change pending: wait only up
        // to the debounce deadline — no fixed-rate wakeups.
        let recv = if pending {
            let wait = Duration::from_millis(DEBOUNCE_MS).saturating_sub(last_event_at.elapsed());
            rx.recv_timeout(wait)
        } else {
            rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };
        match recv {
            Ok(Ok(event)) => {
                if is_relevant(&event, &files) {
                    pending = true;
                    last_event_at = Instant::now();
                }
            }
            Ok(Err(e)) => tracing::warn!(error = %e, "config watcher: watch error"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if pending && last_event_at.elapsed() >= Duration::from_millis(DEBOUNCE_MS) {
            pending = false;
            on_settled();
        }
    }
}

/// Every file `config_path` currently reads, transitively through
/// `include`/`source`, plus the main path as configured (which may be a
/// symlink the parser canonicalised away). Falls back to just the main
/// file if the walk fails — a config that doesn't parse at all
/// shouldn't also lose its own watch.
pub(crate) fn watch_list(config_path: Option<&Path>) -> Vec<PathBuf> {
    let mut list = margo_config::config_source_files(config_path).unwrap_or_else(|e| {
        tracing::warn!(error = ?e, "config watcher: couldn't walk source/include tree");
        Vec::new()
    });
    if let Ok(main) = margo_config::resolve_config_path(config_path) {
        list.push(main);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn wait_for(count: &AtomicUsize, at_least: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if count.load(Ordering::SeqCst) >= at_least {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn survives_atomic_rename_saves_and_ignores_reads() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join("config.conf");
        std::fs::write(&conf, "a=1\n").unwrap();

        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let _watcher = ConfigWatcher::new(std::slice::from_ref(&conf), move || {
            c.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        for (i, body) in ["a=2\n", "a=3\n"].iter().enumerate() {
            let tmp = dir.path().join(".config.conf.tmp");
            std::fs::write(&tmp, body).unwrap();
            std::fs::rename(&tmp, &conf).unwrap();
            assert!(wait_for(&count, i + 1), "save #{} not seen", i + 1);
        }

        let before = count.load(Ordering::SeqCst);
        let _ = std::fs::read_to_string(&conf).unwrap();
        std::fs::write(dir.path().join("unrelated.json"), "{}").unwrap();
        std::thread::sleep(Duration::from_millis(DEBOUNCE_MS * 3));
        assert_eq!(count.load(Ordering::SeqCst), before);
    }
}
