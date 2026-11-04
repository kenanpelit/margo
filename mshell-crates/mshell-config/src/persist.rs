//! Debounced, off-thread profile persistence.
//!
//! `update_config` used to serialise the whole `Config` (and
//! `Config::default()`), diff them and write the profile with two fsyncs on
//! the GTK main thread — for every slider step and spin-button repeat, ~200
//! fsyncs for one 0→100 drag. Now the store is patched at once and the write
//! is queued here: one writer thread, a short trailing debounce, the last
//! value wins. The config watcher asks [`Persister::busy`] before reloading
//! (a reload mid-burst would read the older file back) and recognises the
//! file it just wrote ([`Persister::is_own_write`]) without re-parsing it.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::schema::config::Config;

/// Trailing debounce: a write happens this long after the last change.
pub(crate) const PERSIST_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Default)]
struct State {
    pending: Option<(PathBuf, Config)>,
    deadline: Option<Instant>,
    in_flight: bool,
    /// Hash of the last bytes written, per path.
    last_written: Option<(PathBuf, u64)>,
    writer_started: bool,
    writes: u64,
}

#[derive(Clone)]
pub(crate) struct Persister {
    inner: Arc<(Mutex<State>, Condvar)>,
    debounce: Duration,
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

impl Persister {
    pub(crate) fn new(debounce: Duration) -> Self {
        Self {
            inner: Arc::new((Mutex::new(State::default()), Condvar::new())),
            debounce,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue `config` for `path`, replacing any queued value. A queued value
    /// for a different file (a profile switch mid-burst) is written first.
    pub(crate) fn schedule(&self, path: PathBuf, config: Config) {
        let mut st = self.lock();
        if let Some((old_path, _)) = &st.pending
            && *old_path != path
        {
            drop(st);
            self.flush();
            st = self.lock();
        }
        st.pending = Some((path, config));
        st.deadline = Some(Instant::now() + self.debounce);
        if !st.writer_started {
            st.writer_started = true;
            let this = self.clone();
            let spawned = std::thread::Builder::new()
                .name("config-persist".into())
                .spawn(move || this.writer_loop());
            if let Err(e) = spawned {
                st.writer_started = false;
                drop(st);
                tracing::error!(error = %e, "config: no persist thread; writing inline");
                self.flush();
                return;
            }
        }
        self.inner.1.notify_all();
    }

    fn writer_loop(&self) {
        let mut st = self.lock();
        loop {
            match st.deadline {
                None => {
                    st = self.inner.1.wait(st).unwrap_or_else(|e| e.into_inner());
                }
                Some(deadline) => {
                    let now = Instant::now();
                    if now < deadline {
                        st = self
                            .inner
                            .1
                            .wait_timeout(st, deadline - now)
                            .unwrap_or_else(|e| e.into_inner())
                            .0;
                        continue;
                    }
                    st = self.write_pending(st);
                }
            }
        }
    }

    /// Take the queued value and write it (lock released during the I/O).
    fn write_pending<'a>(&'a self, mut st: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        st.deadline = None;
        let Some((path, config)) = st.pending.take() else {
            return st;
        };
        st.in_flight = true;
        drop(st);
        let written = crate::config_utils::persist_config_layer_bytes(&config, &path);
        let mut st = self.lock();
        st.in_flight = false;
        match written {
            Ok(bytes) => {
                st.last_written = Some((path, hash_bytes(&bytes)));
                st.writes += 1;
            }
            Err(e) => {
                tracing::error!(error = %e, path = %path.display(), "config: persisting the profile failed");
            }
        }
        self.inner.1.notify_all();
        st
    }

    /// Write any queued value now and wait for an in-flight write (shutdown,
    /// profile switch, tests).
    pub(crate) fn flush(&self) {
        let mut st = self.lock();
        while st.in_flight {
            st = self.inner.1.wait(st).unwrap_or_else(|e| e.into_inner());
        }
        if st.pending.is_some() {
            drop(self.write_pending(st));
        }
    }

    /// A write is queued or running: the file on disk is about to change.
    pub(crate) fn busy(&self) -> bool {
        let st = self.lock();
        st.pending.is_some() || st.in_flight
    }

    /// `path` holds exactly the bytes this persister last wrote to it.
    pub(crate) fn is_own_write(&self, path: &Path) -> bool {
        let last = self.lock().last_written.clone();
        let Some((last_path, hash)) = last else {
            return false;
        };
        last_path == path && std::fs::read(path).is_ok_and(|b| hash_bytes(&b) == hash)
    }

    #[cfg(test)]
    pub(crate) fn writes(&self) -> u64 {
        self.lock().writes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_is_one_write_of_the_last_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.yaml");
        let p = Persister::new(Duration::from_millis(100));
        let mut cfg = Config::default();
        for w in 0..50 {
            cfg.osd.width = 100 + w;
            p.schedule(path.clone(), cfg.clone());
        }
        assert!(p.busy());
        std::thread::sleep(Duration::from_millis(400));
        assert!(!p.busy());
        assert_eq!(p.writes(), 1, "coalesced into one write");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("width: 149"), "{body}");
        assert!(p.is_own_write(&path));
        std::fs::write(&path, "osd:\n  width: 1\n").unwrap();
        assert!(!p.is_own_write(&path), "a hand edit is not ours");
    }

    #[test]
    fn flush_writes_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.yaml");
        let p = Persister::new(Duration::from_secs(60));
        let mut cfg = Config::default();
        cfg.osd.width = 777;
        p.schedule(path.clone(), cfg);
        p.flush();
        assert!(!p.busy());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("width: 777")
        );
    }

    #[test]
    fn switching_files_mid_burst_writes_the_first_file() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.yaml"), dir.path().join("b.yaml"));
        let p = Persister::new(Duration::from_secs(60));
        let mut cfg = Config::default();
        cfg.osd.width = 111;
        p.schedule(a.clone(), cfg.clone());
        cfg.osd.width = 222;
        p.schedule(b.clone(), cfg);
        assert!(std::fs::read_to_string(&a).unwrap().contains("width: 111"));
        p.flush();
        assert!(std::fs::read_to_string(&b).unwrap().contains("width: 222"));
    }
}
