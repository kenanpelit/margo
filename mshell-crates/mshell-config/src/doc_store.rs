//! A process-wide JSON document shared by every per-monitor widget.
//!
//! Notes and the AI chat history used to be loaded by each monitor's menu
//! on its own and saved back whole on every edit: the monitor that wrote
//! last overwrote whatever another monitor had added. A parse error turned
//! into an empty document, which the next edit then saved over the user's
//! file. [`DocStore`] fixes both:
//!
//! * one store per file (hold it in a `static`), so every widget sees the
//!   same value and hears about every change through [`DocStore::subscribe`];
//! * one writer thread per store, which always writes the *latest* value, so
//!   saves never overlap and the last edit wins;
//! * a file that fails to parse is copied to `<file>.bak`, kept as is, and
//!   never saved over until it parses again ([`DocSnapshot::broken`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::watch;

/// What every subscriber sees: the shared value and, while the file on disk
/// does not parse, the parse error (saves are refused until it is fixed).
#[derive(Debug)]
pub struct DocSnapshot<T> {
    pub value: Arc<T>,
    pub broken: Option<String>,
}

impl<T> Clone for DocSnapshot<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            broken: self.broken.clone(),
        }
    }
}

/// Returned by [`DocStore::update`] when the change was applied in memory
/// but not saved, because the file on disk does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRefused {
    pub path: PathBuf,
    pub error: String,
}

#[derive(Default)]
struct Counters {
    /// Bumped by every persisted update.
    generation: u64,
    /// Generation of the value last written to disk.
    saved: u64,
    loaded: bool,
}

/// One JSON document on disk, shared process-wide. See the module docs.
pub struct DocStore<T> {
    label: &'static str,
    path: PathBuf,
    state: watch::Sender<DocSnapshot<T>>,
    counters: Mutex<Counters>,
    saved_cv: Condvar,
    /// Serialises disk access (the writer's save vs a reload).
    io: Mutex<()>,
    writer: Mutex<Option<mpsc::Sender<()>>>,
    /// The "not saved" toast was shown for the current broken spell.
    refusal_notified: AtomicBool,
}

impl<T> DocStore<T>
where
    T: Serialize + DeserializeOwned + Default + Clone + PartialEq + Send + Sync + 'static,
{
    /// A store for `path`. Does no I/O: the first [`ensure_loaded`]
    /// (Self::ensure_loaded) or [`reload`](Self::reload) reads the file.
    pub fn new(label: &'static str, path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            label,
            path,
            state: watch::Sender::new(DocSnapshot {
                value: Arc::new(T::default()),
                broken: None,
            }),
            counters: Mutex::new(Counters::default()),
            saved_cv: Condvar::new(),
            io: Mutex::new(()),
            writer: Mutex::new(None),
            refusal_notified: AtomicBool::new(false),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current value and health.
    pub fn snapshot(&self) -> DocSnapshot<T> {
        self.state.borrow().clone()
    }

    /// Change notifications: every update and every reload that changed the
    /// value or its health. The receiver starts with the current snapshot.
    pub fn subscribe(&self) -> watch::Receiver<DocSnapshot<T>> {
        self.state.subscribe()
    }

    /// Read the file once, the first time any widget needs it.
    pub fn ensure_loaded(&self) {
        let first = {
            let mut c = lock(&self.counters);
            !std::mem::replace(&mut c.loaded, true)
        };
        if first {
            self.reload();
        }
    }

    /// Re-read the file (manual refresh, or a poll that picks up hand
    /// edits). Skipped while an update is waiting to be written, so a reload
    /// can never replace an edit with the older file contents.
    pub fn reload(&self) {
        let _io = lock(&self.io);
        {
            let mut c = lock(&self.counters);
            c.loaded = true;
            if c.generation != c.saved {
                return;
            }
        }
        let read = std::fs::read_to_string(&self.path);
        // Publish under the counters lock, re-checking that no update slipped
        // in while the file was read (it would be replaced by older data).
        let c = lock(&self.counters);
        if c.generation != c.saved {
            return;
        }
        match read {
            Ok(raw) => match serde_json::from_str::<T>(&raw) {
                Ok(value) => self.publish(value, None),
                Err(e) => self.mark_broken(e.to_string()),
            },
            // No file yet (or it was removed): an empty document is right.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.publish(T::default(), None);
            }
            Err(e) => {
                tracing::warn!(doc = self.label, path = %self.path.display(), error = %e, "read failed");
            }
        }
        drop(c);
    }

    fn mark_broken(&self, error: String) {
        let was_broken = self.state.borrow().broken.is_some();
        if !was_broken {
            match margo_fs::side_file::backup_broken(&self.path) {
                Ok(bak) => tracing::warn!(
                    doc = self.label,
                    path = %self.path.display(),
                    backup = %bak.display(),
                    error = %error,
                    "file does not parse; kept a copy and not saving over it"
                ),
                Err(be) => tracing::warn!(
                    doc = self.label,
                    path = %self.path.display(),
                    error = %error,
                    backup_error = %be,
                    "file does not parse; not saving over it"
                ),
            }
        }
        // Keep the in-memory value: it is the last good state (or empty).
        self.state.send_if_modified(|s| {
            if s.broken.as_deref() == Some(error.as_str()) {
                return false;
            }
            s.broken = Some(error);
            true
        });
    }

    fn publish(&self, value: T, broken: Option<String>) {
        if broken.is_none() {
            self.refusal_notified.store(false, Ordering::Relaxed);
        }
        self.state.send_if_modified(|s| {
            if *s.value == value && s.broken == broken {
                return false;
            }
            s.value = Arc::new(value);
            s.broken = broken;
            true
        });
    }

    /// Apply `f` to the shared value, notify subscribers and queue a save.
    /// While the file does not parse the change stays in memory only and
    /// [`SaveRefused`] is returned (a toast is raised once per spell).
    pub fn update(self: &Arc<Self>, f: impl FnOnce(&mut T)) -> Result<(), SaveRefused> {
        let broken = self.state.borrow().broken.clone();
        if let Some(error) = broken {
            self.apply(f);
            self.notify_refused(&error);
            return Err(SaveRefused {
                path: self.path.clone(),
                error,
            });
        }
        {
            // Bump and apply under one lock so a concurrent reload either
            // publishes before this edit or skips (a save is now pending).
            let mut c = lock(&self.counters);
            c.generation += 1;
            self.apply(f);
        }
        self.kick_writer();
        Ok(())
    }

    /// Apply `f` and notify subscribers without saving (e.g. the AI chat
    /// with history persistence turned off).
    pub fn update_in_memory(&self, f: impl FnOnce(&mut T)) {
        self.apply(f);
    }

    fn apply(&self, f: impl FnOnce(&mut T)) {
        self.state.send_if_modified(|s| {
            let mut next = (*s.value).clone();
            f(&mut next);
            if next == *s.value {
                return false;
            }
            s.value = Arc::new(next);
            true
        });
    }

    fn notify_refused(&self, error: &str) {
        if self.refusal_notified.swap(true, Ordering::Relaxed) {
            return;
        }
        tracing::warn!(doc = self.label, path = %self.path.display(), error, "change not saved: file does not parse");
        let delivered = crate::toast::try_push_toast(crate::toast::ToastEvent {
            icon: "dialog-warning-symbolic".to_string(),
            title: format!("{} not saved", self.label),
            body: Some(format!(
                "{} does not parse — fix it (a copy is in .bak); changes stay in memory only",
                self.path.display()
            )),
            severity: crate::toast::ToastSeverity::Warn,
        });
        if !delivered {
            self.refusal_notified.store(false, Ordering::Relaxed);
        }
    }

    fn kick_writer(self: &Arc<Self>) {
        let mut writer = lock(&self.writer);
        if let Some(tx) = writer.as_ref()
            && tx.send(()).is_ok()
        {
            return;
        }
        let (tx, rx) = mpsc::channel::<()>();
        let _ = tx.send(());
        let store = Arc::downgrade(self);
        let spawned = std::thread::Builder::new()
            .name(format!("{}-writer", self.label.to_lowercase()))
            .spawn(move || {
                while rx.recv().is_ok() {
                    // Coalesce a burst: one write covers every queued kick.
                    while rx.try_recv().is_ok() {}
                    let Some(store) = store.upgrade() else { break };
                    store.write_latest();
                }
            });
        match spawned {
            Ok(_) => *writer = Some(tx),
            Err(e) => {
                tracing::error!(doc = self.label, error = %e, "could not start the writer; saving inline");
                drop(writer);
                self.write_latest();
            }
        }
    }

    /// Write the newest value. Always the newest: a save that started for an
    /// older generation still writes what is current now.
    fn write_latest(&self) {
        let _io = lock(&self.io);
        let generation = lock(&self.counters).generation;
        let snap = self.snapshot();
        if snap.broken.is_none() {
            let res = serde_json::to_vec_pretty(&*snap.value)
                .map_err(std::io::Error::other)
                .and_then(|json| crate::atomic_write::atomic_write(&self.path, &json));
            if let Err(e) = res {
                tracing::warn!(doc = self.label, path = %self.path.display(), error = %e, "save failed");
            }
        }
        let mut c = lock(&self.counters);
        c.saved = c.saved.max(generation);
        self.saved_cv.notify_all();
    }

    /// Block until every queued update is on disk, or `timeout` passes.
    /// Returns whether everything was written.
    pub fn flush(&self, timeout: Duration) -> bool {
        let c = lock(&self.counters);
        let (c, _) = self
            .saved_cv
            .wait_timeout_while(c, timeout, |c| c.saved < c.generation)
            .unwrap_or_else(|e| e.into_inner());
        c.saved >= c.generation
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
    struct Doc {
        #[serde(default)]
        items: Vec<String>,
    }

    const T: Duration = Duration::from_secs(5);

    fn read(path: &Path) -> Doc {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// Two widgets (two monitors) share one store: an edit made through one
    /// is seen by the other, and the other's later edit keeps it.
    #[test]
    fn edits_from_two_widgets_both_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.json");
        std::fs::write(&path, r#"{"items":["old"]}"#).unwrap();
        let store = DocStore::<Doc>::new("Notes", path.clone());
        let monitor_a = store.clone();
        let monitor_b = store.clone();
        let b_rx = monitor_b.subscribe();
        monitor_a.ensure_loaded();
        monitor_b.ensure_loaded();

        monitor_a.update(|d| d.items.push("from A".into())).unwrap();
        assert!(b_rx.has_changed().unwrap(), "B is told about A's edit");
        monitor_b.update(|d| d.items.push("from B".into())).unwrap();
        assert!(store.flush(T));
        assert_eq!(read(&path).items, ["old", "from A", "from B"]);
    }

    /// A file that does not parse is kept: no update ever writes over it,
    /// and a `.bak` copy is left next to it.
    #[test]
    fn parse_error_refuses_to_save_and_keeps_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.json");
        let broken = r#"{"items": ["a", }"#;
        std::fs::write(&path, broken).unwrap();
        let store = DocStore::<Doc>::new("Notes", path.clone());
        store.ensure_loaded();
        assert!(store.snapshot().broken.is_some());

        let res = store.update(|d| d.items.push("new".into()));
        assert!(res.is_err(), "the save is refused");
        assert!(store.flush(T));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        let bak = dir.path().join("notes.json.bak");
        assert_eq!(std::fs::read_to_string(bak).unwrap(), broken);

        // Fixed by hand: a reload lifts the guard and saving resumes.
        std::fs::write(&path, r#"{"items":["fixed"]}"#).unwrap();
        store.reload();
        assert!(store.snapshot().broken.is_none());
        store.update(|d| d.items.push("new".into())).unwrap();
        assert!(store.flush(T));
        assert_eq!(read(&path).items, ["fixed", "new"]);
    }

    /// A burst of edits ends with the final value on disk.
    #[test]
    fn rapid_updates_end_with_the_latest_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        let store = DocStore::<Doc>::new("Doc", path.clone());
        store.ensure_loaded();
        for i in 0..200 {
            store.update(|d| d.items = vec![i.to_string()]).unwrap();
        }
        assert!(store.flush(T));
        assert_eq!(read(&path).items, ["199"]);
    }

    /// A missing file is an empty document, and the first edit creates it.
    #[test]
    fn missing_file_is_empty_and_first_edit_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/doc.json");
        let store = DocStore::<Doc>::new("Doc", path.clone());
        store.ensure_loaded();
        assert_eq!(*store.snapshot().value, Doc::default());
        store.update(|d| d.items.push("x".into())).unwrap();
        assert!(store.flush(T));
        assert_eq!(read(&path).items, ["x"]);
    }

    /// `update_in_memory` notifies but never writes.
    #[test]
    fn in_memory_update_does_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        let store = DocStore::<Doc>::new("Doc", path.clone());
        store.ensure_loaded();
        let mut rx = store.subscribe();
        rx.mark_unchanged();
        store.update_in_memory(|d| d.items.push("x".into()));
        assert!(rx.has_changed().unwrap());
        assert!(store.flush(T));
        assert!(!path.exists());
    }
}
