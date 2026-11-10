//! One index per search combination, built and loaded off the GTK
//! thread.
//!
//! The store holds at most one index in memory — the combination the
//! launcher asks for — and keeps every built index on disk
//! ([`super::index::save`]), so switching scope and back loads the old
//! index instead of walking again. Per combination ([`Refresh`]):
//!
//! * `Auto` (no USB disks): built on first use, rebuilt in the background
//!   once older than its TTL; the old index is served meanwhile.
//! * `Manual` (USB disks on): never walked on its own — not on launcher
//!   open, not on a mode switch, not on plug-in. Only
//!   [`IndexStore::rebuild`] walks it; until then the view says it is
//!   missing or out of date.
//!
//! One build runs at a time. An explicit rebuild of another combination
//! cancels the running build; an automatic one waits for it. The held
//! index is dropped after an idle period, like the original cache.

use super::index::{Cancel, Index, load, prune_cache_dir, save};
use super::scope::{FsSettings, Refresh};
use crate::provider::RefreshNotifier;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One combination to show.
#[derive(Debug, Clone)]
pub struct Job {
    pub key: String,
    pub settings: FsSettings,
    pub home: String,
    pub refresh: Refresh,
    /// "Home + folders + USB disks" — for the hint while it builds.
    pub label: String,
}

/// Walks a job's roots (blocking; called on a worker thread).
pub type BuildFn = Arc<dyn Fn(&Job, &Cancel) -> Option<Index> + Send + Sync>;

/// What the launcher can show for a job right now.
#[derive(Debug, Clone, Default)]
pub struct View {
    pub index: Option<Arc<Index>>,
    /// Reading this job's index from disk.
    pub loading: bool,
    /// A build is running: (label, started, it is this job's).
    pub building: Option<(String, Instant, bool)>,
    /// No index exists for this job and none is built automatically.
    pub never_built: bool,
}

struct Building {
    key: String,
    label: String,
    started: Instant,
    cancel: Arc<Cancel>,
}

struct State {
    current: Option<(String, Arc<Index>)>,
    /// Key of the latest view — a finished load / build for any other key
    /// is not kept in memory.
    wanted: Option<String>,
    loading: Option<String>,
    /// Keys known to have no index on disk.
    absent: HashSet<String>,
    building: Option<Building>,
    last_used: Instant,
    reaper: bool,
}

pub struct IndexStore {
    state: Arc<Mutex<State>>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
    /// Where indexes are kept; `None` = memory only.
    dir: Option<PathBuf>,
    build: BuildFn,
    idle: Duration,
}

impl IndexStore {
    pub fn new(dir: Option<PathBuf>, build: BuildFn, idle: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                current: None,
                wanted: None,
                loading: None,
                absent: HashSet::new(),
                building: None,
                last_used: Instant::now(),
                reaper: false,
            })),
            notifier: Arc::new(Mutex::new(None)),
            dir,
            build,
            idle,
        }
    }

    pub fn set_notifier(&self, notifier: RefreshNotifier) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    /// The index for `job` if held, kicking a disk load or an automatic
    /// build as its policy says. Never blocks on either.
    pub fn view(&self, job: &Job) -> View {
        let Ok(mut st) = self.state.lock() else {
            return View::default();
        };
        st.last_used = Instant::now();
        st.wanted = Some(job.key.clone());
        let index = st
            .current
            .as_ref()
            .filter(|(k, _)| *k == job.key)
            .map(|(_, i)| Arc::clone(i));
        if index.is_none() && !st.absent.contains(&job.key) && st.loading.is_none() {
            match &self.dir {
                Some(dir) => self.start_load(&mut st, dir.clone(), job.key.clone()),
                None => {
                    st.absent.insert(job.key.clone());
                }
            }
        }
        let absent = st.absent.contains(&job.key);
        if let Refresh::Auto(ttl) = job.refresh
            && st.building.is_none()
        {
            let stale = index.as_ref().is_some_and(|i| i.age() > ttl);
            if stale || (index.is_none() && absent) {
                self.start_build(&mut st, job);
            }
        }
        View {
            loading: st.loading.as_deref() == Some(job.key.as_str()),
            building: st
                .building
                .as_ref()
                .map(|b| (b.label.clone(), b.started, b.key == job.key)),
            never_built: index.is_none() && absent && job.refresh == Refresh::Manual,
            index,
        }
    }

    /// Walk `job` now (an explicit "Rebuild search index"). A build of
    /// another combination is cancelled; one already running for this
    /// combination is left alone.
    pub fn rebuild(&self, job: &Job) {
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        st.wanted = Some(job.key.clone());
        if let Some(b) = &st.building {
            if b.key == job.key {
                return;
            }
            b.cancel.cancel();
        }
        self.start_build(&mut st, job);
    }

    /// Stop the running build, if any.
    pub fn cancel(&self) {
        if let Ok(st) = self.state.lock()
            && let Some(b) = &st.building
        {
            b.cancel.cancel();
        }
    }

    fn notify(notifier: &Mutex<Option<RefreshNotifier>>) {
        if let Ok(slot) = notifier.lock()
            && let Some(n) = slot.as_ref()
        {
            n();
        }
    }

    fn start_load(&self, st: &mut State, dir: PathBuf, key: String) {
        st.loading = Some(key.clone());
        let state = Arc::clone(&self.state);
        let notifier = Arc::clone(&self.notifier);
        let idle = self.idle;
        std::thread::spawn(move || {
            let loaded = load(&dir, &key);
            if let Ok(mut st) = state.lock() {
                st.loading = None;
                match loaded {
                    Some(index) => {
                        if st.wanted.as_deref() == Some(key.as_str()) {
                            st.current = Some((key, Arc::new(index)));
                            ensure_reaper(&state, &mut st, idle);
                        }
                    }
                    None => {
                        st.absent.insert(key);
                    }
                }
            }
            Self::notify(&notifier);
        });
    }

    fn start_build(&self, st: &mut State, job: &Job) {
        let cancel = Arc::new(Cancel::default());
        st.building = Some(Building {
            key: job.key.clone(),
            label: job.label.clone(),
            started: Instant::now(),
            cancel: Arc::clone(&cancel),
        });
        let state = Arc::clone(&self.state);
        let notifier = Arc::clone(&self.notifier);
        let build = Arc::clone(&self.build);
        let dir = self.dir.clone();
        let idle = self.idle;
        let job = job.clone();
        std::thread::spawn(move || {
            let built = build(&job, &cancel).map(Arc::new);
            if let Ok(mut st) = state.lock() {
                // Only clear the slot if it is still ours (an explicit
                // rebuild may have replaced a cancelled build).
                if st
                    .building
                    .as_ref()
                    .is_some_and(|b| Arc::ptr_eq(&b.cancel, &cancel))
                {
                    st.building = None;
                }
                if let Some(index) = &built {
                    st.absent.remove(&job.key);
                    if st.wanted.as_deref() == Some(job.key.as_str()) {
                        st.current = Some((job.key.clone(), Arc::clone(index)));
                        ensure_reaper(&state, &mut st, idle);
                    }
                }
            }
            Self::notify(&notifier);
            if let (Some(index), Some(dir)) = (built, dir) {
                if let Err(err) = save(&dir, &job.key, &index) {
                    tracing::warn!(?err, "file search: could not save the index");
                }
                prune_cache_dir(&dir);
            }
        });
    }

    #[cfg(test)]
    pub fn wait_idle(&self) {
        for _ in 0..2000 {
            if self
                .state
                .lock()
                .map(|s| s.building.is_none() && s.loading.is_none())
                .unwrap_or(true)
            {
                // Let a finishing thread's save land too.
                std::thread::sleep(Duration::from_millis(20));
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(test)]
    pub fn seed(&self, key: &str, index: Index) {
        if let Ok(mut st) = self.state.lock() {
            st.current = Some((key.to_string(), Arc::new(index)));
        }
    }
}

fn ensure_reaper(state: &Arc<Mutex<State>>, st: &mut State, idle: Duration) {
    if st.reaper {
        return;
    }
    st.reaper = true;
    let state = Arc::clone(state);
    std::thread::spawn(move || reap_when_idle(&state, idle));
}

/// Drop the held index once nothing viewed it for `idle` (it stays on
/// disk); a view meanwhile pushes the deadline out.
fn reap_when_idle(state: &Mutex<State>, idle: Duration) {
    loop {
        let wait = match state.lock() {
            Ok(mut st) => {
                let unused = st.last_used.elapsed();
                if st.current.is_none() || unused >= idle {
                    let dropped = st.current.take();
                    st.reaper = false;
                    drop(st);
                    drop(dropped);
                    return;
                }
                idle - unused
            }
            Err(_) => return,
        };
        std::thread::sleep(wait);
    }
}

#[cfg(test)]
mod tests {
    use super::super::index::Entry;
    use super::super::scope::SearchScope;
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn job(key: &str, refresh: Refresh) -> Job {
        Job {
            key: key.into(),
            settings: FsSettings {
                scope: SearchScope::Home,
                ..FsSettings::default()
            },
            home: "/home/u".into(),
            refresh,
            label: key.into(),
        }
    }

    /// A build that records how often it ran and lists one path named
    /// after the job.
    fn counting(n: Arc<AtomicUsize>) -> BuildFn {
        Arc::new(move |job: &Job, _c: &Cancel| {
            n.fetch_add(1, Ordering::SeqCst);
            let mut i = Index::from_entries(vec![Entry {
                path: format!("/home/u/{}", job.key),
                is_dir: Some(false),
            }]);
            i.walks = vec![super::super::index::Walked {
                root: "/home/u".into(),
                backend: super::super::index::Backend::Walk,
            }];
            Some(i)
        })
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fs-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn settle(store: &IndexStore, job: &Job) -> View {
        for _ in 0..50 {
            let v = store.view(job);
            store.wait_idle();
            if v.index.is_some() && v.building.is_none() {
                return v;
            }
        }
        store.view(job)
    }

    const AUTO: Refresh = Refresh::Auto(Duration::from_secs(300));

    #[test]
    fn switching_back_reuses_the_saved_index() {
        let dir = temp("switch");
        let n = Arc::new(AtomicUsize::new(0));
        let store = IndexStore::new(
            Some(dir.clone()),
            counting(Arc::clone(&n)),
            Duration::from_secs(60),
        );
        let a = job("a", AUTO);
        let b = job("b", AUTO);
        assert_eq!(
            settle(&store, &a).index.unwrap().entries[0].path,
            "/home/u/a"
        );
        assert_eq!(
            settle(&store, &b).index.unwrap().entries[0].path,
            "/home/u/b"
        );
        assert_eq!(n.load(Ordering::SeqCst), 2);
        // Back to a: loaded from disk, not walked again.
        assert_eq!(
            settle(&store, &a).index.unwrap().entries[0].path,
            "/home/u/a"
        );
        assert_eq!(n.load(Ordering::SeqCst), 2);
        // A fresh store (shell restart) also starts from disk.
        let store2 = IndexStore::new(
            Some(dir.clone()),
            counting(Arc::clone(&n)),
            Duration::from_secs(60),
        );
        assert!(settle(&store2, &b).index.is_some());
        assert_eq!(n.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manual_jobs_never_walk_on_their_own() {
        let dir = temp("manual");
        let n = Arc::new(AtomicUsize::new(0));
        let store = IndexStore::new(
            Some(dir.clone()),
            counting(Arc::clone(&n)),
            Duration::from_secs(60),
        );
        let usb = job("usb", Refresh::Manual);
        store.view(&usb);
        store.wait_idle();
        let v = store.view(&usb);
        assert!(v.never_built);
        assert!(v.index.is_none());
        assert_eq!(n.load(Ordering::SeqCst), 0);
        store.rebuild(&usb);
        store.wait_idle();
        let v = store.view(&usb);
        assert!(v.index.is_some());
        assert!(!v.never_built);
        assert_eq!(n.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_auto_index_is_served_and_rebuilt() {
        let n = Arc::new(AtomicUsize::new(0));
        let store = IndexStore::new(None, counting(Arc::clone(&n)), Duration::from_secs(60));
        let mut old = Index::from_entries(vec![]);
        old.built_at = Some(std::time::SystemTime::now() - Duration::from_secs(301));
        store.seed("a", old);
        let v = store.view(&job("a", AUTO));
        assert!(v.index.is_some(), "stale index still served");
        store.wait_idle();
        assert_eq!(n.load(Ordering::SeqCst), 1);
        // The same stale index under Manual is left alone.
        let n2 = Arc::new(AtomicUsize::new(0));
        let store = IndexStore::new(None, counting(Arc::clone(&n2)), Duration::from_secs(60));
        let mut old = Index::from_entries(vec![]);
        old.built_at = Some(std::time::SystemTime::now() - Duration::from_secs(99_999));
        store.seed("u", old);
        assert!(store.view(&job("u", Refresh::Manual)).index.is_some());
        store.wait_idle();
        assert_eq!(n2.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn rebuild_cancels_another_combinations_walk() {
        let started = Arc::new(AtomicUsize::new(0));
        let s2 = Arc::clone(&started);
        let build: BuildFn = Arc::new(move |job: &Job, c: &Cancel| {
            s2.fetch_add(1, Ordering::SeqCst);
            if job.key == "slow" {
                for _ in 0..2000 {
                    if c.is_cancelled() {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            Some(Index::from_entries(vec![]))
        });
        let store = IndexStore::new(None, build, Duration::from_secs(60));
        let slow = job("slow", Refresh::Manual);
        let fast = job("fast", Refresh::Manual);
        store.rebuild(&slow);
        std::thread::sleep(Duration::from_millis(30));
        let t = Instant::now();
        store.rebuild(&fast);
        store.wait_idle();
        assert!(t.elapsed() < Duration::from_secs(5));
        assert!(store.view(&fast).index.is_some());
        assert!(store.view(&slow).index.is_none());
        // Cancel stops a running build without storing anything.
        store.rebuild(&slow);
        std::thread::sleep(Duration::from_millis(30));
        store.cancel();
        store.wait_idle();
        assert!(store.view(&slow).index.is_none());
    }

    #[test]
    fn idle_index_is_dropped_from_memory() {
        let n = Arc::new(AtomicUsize::new(0));
        let store = IndexStore::new(None, counting(n), Duration::from_millis(100));
        let a = job("a", AUTO);
        let v = settle(&store, &a);
        let weak = Arc::downgrade(&v.index.unwrap());
        for _ in 0..200 {
            if weak.upgrade().is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("index still held after the idle period");
    }
}
