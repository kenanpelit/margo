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
//! `notify`'s own watcher callback runs on its *own* background thread,
//! not the compositor's — `MargoState` isn't `Send`, so the callback
//! can't call `reload_config()` directly. It debounces on that thread
//! (an editor's save is several writes, a `source`'d directory's
//! sibling-file churn is several events) and, once settled, calls back
//! into `on_settled` — which the caller wires to a `calloop::channel`
//! `Sender` so the *actual* reload happens on the compositor's own
//! thread, in its own event-loop dispatch turn. Same debounce shape as
//! `mshell-config`'s own file watcher (`config_manager.rs`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

/// Coalesces a burst of filesystem events (an editor's multi-write-per-
/// save, several `source`'d files changing in one `mctl reload`-free
/// edit) into a single reload instead of firing once per event.
const DEBOUNCE_MS: u64 = 300;

/// Owns the live filesystem watcher. Dropping it stops watching and
/// joins nothing — the background thread exits on its own once the
/// `notify` sender side (held by the watcher) drops and its channel
/// disconnects.
pub struct ConfigWatcher {
    watcher: RecommendedWatcher,
    watched: HashSet<PathBuf>,
}

impl ConfigWatcher {
    /// Start watching `paths`. `on_settled` fires once per debounced
    /// batch of changes, from a background thread — never the
    /// compositor's own. Returns `None` if the watcher itself couldn't
    /// be created (e.g. the host's inotify instance limit is already
    /// exhausted) — losing auto-reload is far better than taking the
    /// compositor down over it.
    pub fn new(paths: &[PathBuf], on_settled: impl Fn() + Send + 'static) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
        let mut watcher = match RecommendedWatcher::new(tx, notify::Config::default()) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "config watcher: failed to create, auto-reload disabled"
                );
                return None;
            }
        };

        let watched = watch_all(&mut watcher, paths);
        std::thread::spawn(move || watch_loop(rx, on_settled));
        Some(ConfigWatcher { watcher, watched })
    }

    /// Re-sync the watch set after a reload — only touches paths that
    /// actually changed (unwatch removed, watch newly added) rather
    /// than tearing down and recreating the watcher.
    pub fn update(&mut self, paths: &[PathBuf]) {
        let fresh: HashSet<PathBuf> = paths.iter().cloned().collect();
        for stale in self.watched.difference(&fresh) {
            let _ = self.watcher.unwatch(stale);
        }
        for new in fresh.difference(&self.watched) {
            let _ = self.watcher.watch(new, RecursiveMode::NonRecursive);
        }
        self.watched = fresh;
    }
}

fn watch_all(watcher: &mut RecommendedWatcher, paths: &[PathBuf]) -> HashSet<PathBuf> {
    let mut watched = HashSet::new();
    for p in paths {
        match watcher.watch(p, RecursiveMode::NonRecursive) {
            Ok(()) => {
                watched.insert(p.clone());
            }
            Err(e) => {
                tracing::warn!(error = %e, path = %p.display(), "config watcher: couldn't watch")
            }
        }
    }
    watched
}

fn watch_loop(rx: mpsc::Receiver<notify::Result<Event>>, on_settled: impl Fn()) {
    let mut pending = false;
    let mut last_event_at = Instant::now();
    loop {
        // Idle: block indefinitely for the next event. A change pending:
        // poll only up to the debounce deadline, same shape as
        // mshell-config's watcher — not a permanent fixed-rate wakeup
        // for a file that changes a few times a day.
        let recv = if pending {
            let wait = Duration::from_millis(DEBOUNCE_MS).saturating_sub(last_event_at.elapsed());
            rx.recv_timeout(wait)
        } else {
            rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };
        match recv {
            Ok(Ok(_event)) => {
                pending = true;
                last_event_at = Instant::now();
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
/// `include`/`source` — margo_config's own parse-tracked file list (see
/// `margo_config::config_source_files`), falling back to just the main
/// file if that walk fails for some reason (a config that doesn't
/// parse at all shouldn't also lose its *own* watch).
pub(crate) fn watch_list(config_path: Option<&Path>) -> Vec<PathBuf> {
    margo_config::config_source_files(config_path).unwrap_or_else(|e| {
        tracing::warn!(error = ?e, "config watcher: couldn't walk source/include tree");
        config_path
            .map(|p| vec![p.to_path_buf()])
            .unwrap_or_default()
    })
}
