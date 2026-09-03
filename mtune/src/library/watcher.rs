// SPDX-License-Identifier: GPL-3.0-or-later
//! Debounced inotify watch over the library roots. RAII: dropping the
//! `LibraryWatcher` stops watching and ends the debounce thread.

use crate::library::LibraryEvent;
use crate::library::config::LibrarySection;
use notify::event::ModifyKind;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(500);

pub struct LibraryWatcher {
    _inner: notify::RecommendedWatcher,
    _pump: std::thread::JoinHandle<()>,
}

impl std::fmt::Debug for LibraryWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryWatcher").finish_non_exhaustive()
    }
}

impl LibraryWatcher {
    /// Start watching `lib`'s resolved roots. `sink` receives coalesced
    /// `LibraryEvent`s for playable files only.
    pub fn start(
        lib: LibrarySection,
        sink: async_channel::Sender<LibraryEvent>,
    ) -> anyhow::Result<Self> {
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<Event>>();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = raw_tx.send(res);
        })?;
        let mode = if lib.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        for root in lib.resolved_roots() {
            watcher.watch(&root, mode)?;
        }

        let lib2 = lib.clone();
        let pump = std::thread::Builder::new()
            .name("mtune-watch".into())
            .spawn(move || debounce_loop(raw_rx, lib2, sink))?;

        Ok(Self {
            _inner: watcher,
            _pump: pump,
        })
    }
}

fn debounce_loop(
    raw_rx: mpsc::Receiver<notify::Result<Event>>,
    lib: LibrarySection,
    sink: async_channel::Sender<LibraryEvent>,
) {
    // path -> does it exist now (after coalescing this window's events)
    let mut pending: HashMap<PathBuf, bool> = HashMap::new();

    loop {
        // Block for the first event of a burst.
        let Ok(first) = raw_rx.recv() else {
            return; // watcher dropped
        };
        absorb(first, &lib, &mut pending);

        // Drain whatever else lands within the debounce window.
        let deadline = Instant::now() + DEBOUNCE;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match raw_rx.recv_timeout(deadline - now) {
                Ok(ev) => absorb(ev, &lib, &mut pending),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }

        for (path, exists) in pending.drain() {
            let ev = if exists {
                LibraryEvent::Added(path)
            } else {
                LibraryEvent::Removed(path)
            };
            if sink.send_blocking(ev).is_err() {
                return; // consumer gone
            }
        }
    }
}

fn absorb(ev: notify::Result<Event>, lib: &LibrarySection, pending: &mut HashMap<PathBuf, bool>) {
    let Ok(ev) = ev else { return };
    let interesting = matches!(
        ev.kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    );
    if !interesting {
        return;
    }
    for path in ev.paths {
        if lib.is_playable(&path) {
            pending.insert(path.clone(), path.exists());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn recv_timeout(
        rx: &async_channel::Receiver<LibraryEvent>,
        d: Duration,
    ) -> Option<LibraryEvent> {
        let deadline = Instant::now() + d;
        loop {
            if let Ok(ev) = rx.try_recv() {
                return Some(ev);
            }
            if Instant::now() > deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn detects_added_and_removed_playable_file() {
        let dir = tempfile::tempdir().unwrap();
        let lib = LibrarySection {
            roots: vec![dir.path().to_path_buf()],
            ..Default::default()
        };
        let (tx, rx) = async_channel::unbounded();
        let _w = LibraryWatcher::start(lib, tx).unwrap();

        let song = dir.path().join("new.mp3");
        fs::write(&song, b"x").unwrap();
        let added = recv_timeout(&rx, Duration::from_secs(4));
        assert!(
            matches!(added, Some(LibraryEvent::Added(ref p)) if *p == song),
            "expected Added({}), got {added:?}",
            song.display()
        );

        fs::remove_file(&song).unwrap();
        let removed = recv_timeout(&rx, Duration::from_secs(4));
        assert!(
            matches!(removed, Some(LibraryEvent::Removed(ref p)) if *p == song),
            "expected Removed({}), got {removed:?}",
            song.display()
        );
    }

    #[test]
    fn ignores_non_playable_files() {
        let dir = tempfile::tempdir().unwrap();
        let lib = LibrarySection {
            roots: vec![dir.path().to_path_buf()],
            ..Default::default()
        };
        let (tx, rx) = async_channel::unbounded();
        let _w = LibraryWatcher::start(lib, tx).unwrap();
        fs::write(dir.path().join("cover.jpg"), b"x").unwrap();
        assert!(recv_timeout(&rx, Duration::from_secs(2)).is_none());
    }
}
