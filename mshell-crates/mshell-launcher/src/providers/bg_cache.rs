//! Background-refreshing snapshot cache.
//!
//! Providers that surface live external state read through a slow blocking
//! call — a `bluetoothctl` / `wpctl` / `playerctl` subprocess — can't do
//! that work inside [`Provider::search`](crate::provider::Provider::search)
//! / `browse`: those run on the GTK main thread and fire on every tab
//! switch and keystroke, so a blocking subprocess there freezes the UI
//! (~1s on a Bluetooth tab with a flaky adapter).
//!
//! [`BgCache`] serves the last snapshot instantly and never blocks: when
//! the value is cold or older than the TTL it kicks the refresh off onto a
//! worker thread and returns whatever it currently has (possibly `None` on
//! the very first access). When the refresh lands it stores the value and
//! fires the [`RefreshNotifier`] so the launcher re-runs the current query
//! and the new rows appear.
//!
//! A cache built with [`BgCache::with_idle_evict`] also drops its value
//! once nothing has read it for that long, so a big snapshot (the file
//! index) doesn't stay resident for the rest of the session after one use.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::provider::RefreshNotifier;

struct Cached<T> {
    value: T,
    captured_at: Instant,
}

/// The value plus what the idle eviction needs, under one lock.
struct Slot<T> {
    cached: Option<Cached<T>>,
    /// Last [`BgCache::get`].
    last_used: Instant,
    /// Bumped when the idle eviction drops the value, so a refresh that
    /// started before it doesn't bring the dropped value back.
    generation: u64,
    /// An eviction thread is waiting on this cache.
    reaper: bool,
}

pub(crate) struct BgCache<T> {
    inner: Arc<Mutex<Slot<T>>>,
    in_flight: Arc<AtomicBool>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
    ttl: Duration,
    /// Drop the value after this long without a `get`.
    idle_evict: Option<Duration>,
}

impl<T: Clone + Send + 'static> BgCache<T> {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Slot {
                cached: None,
                last_used: Instant::now(),
                generation: 0,
                reaper: false,
            })),
            in_flight: Arc::new(AtomicBool::new(false)),
            notifier: Arc::new(Mutex::new(None)),
            ttl,
            idle_evict: None,
        }
    }

    /// Like [`Self::new`], and the value is dropped (freed) after `idle`
    /// without a [`Self::get`]. The waiting thread only exists while a
    /// value is held.
    pub(crate) fn with_idle_evict(ttl: Duration, idle: Duration) -> Self {
        Self {
            idle_evict: Some(idle),
            ..Self::new(ttl)
        }
    }

    /// Install the callback fired (from the worker thread) after a refresh
    /// stores a new value.
    pub(crate) fn set_notifier(&self, notifier: RefreshNotifier) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    /// The current snapshot if any (fresh *or* stale), kicking an
    /// off-thread refresh when the value is cold or past its TTL. Never
    /// blocks on `refresh`. At most one refresh runs at a time; when it
    /// completes it stores the result and fires the notifier.
    pub(crate) fn get<F>(&self, refresh: F) -> Option<T>
    where
        F: FnOnce() -> T + Send + 'static,
    {
        let (value, fresh, generation) = match self.inner.lock() {
            Ok(mut slot) => {
                slot.last_used = Instant::now();
                let generation = slot.generation;
                match slot.cached.as_ref() {
                    Some(cached) => (
                        Some(cached.value.clone()),
                        cached.captured_at.elapsed() < self.ttl,
                        generation,
                    ),
                    None => (None, false, generation),
                }
            }
            Err(_) => (None, false, 0),
        };

        if !fresh
            && self
                .in_flight
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            let inner = Arc::clone(&self.inner);
            let in_flight = Arc::clone(&self.in_flight);
            let notifier = Arc::clone(&self.notifier);
            let idle_evict = self.idle_evict;
            std::thread::spawn(move || {
                let value = refresh();
                let mut stored = false;
                if let Ok(mut slot) = inner.lock()
                    && slot.generation == generation
                {
                    slot.cached = Some(Cached {
                        value,
                        captured_at: Instant::now(),
                    });
                    stored = true;
                    if let Some(idle) = idle_evict
                        && !slot.reaper
                    {
                        slot.reaper = true;
                        let inner = Arc::clone(&inner);
                        std::thread::spawn(move || reap_when_idle(&inner, idle));
                    }
                }
                in_flight.store(false, Ordering::Release);
                if stored
                    && let Ok(slot) = notifier.lock()
                    && let Some(notify) = slot.as_ref()
                {
                    notify();
                }
            });
        }

        value
    }

    /// Whether a value is held (fresh or stale).
    #[cfg(test)]
    pub(crate) fn is_cached(&self) -> bool {
        self.inner.lock().is_ok_and(|s| s.cached.is_some())
    }

    /// Drop the cached snapshot so the next [`Self::get`] is treated as
    /// cold and kicks a fresh off-thread refresh. Used by providers
    /// whose inputs changed (e.g. a panel-open asked for a re-scan):
    /// the rescan still happens on a worker thread, not the caller's.
    pub(crate) fn invalidate(&self) {
        if let Ok(mut slot) = self.inner.lock() {
            slot.cached = None;
        }
    }

    /// Seed the snapshot directly (no refresh). Test helper for
    /// injecting a deterministic value.
    #[cfg(test)]
    pub(crate) fn seed(&self, value: T) {
        if let Ok(mut slot) = self.inner.lock() {
            slot.cached = Some(Cached {
                value,
                captured_at: Instant::now(),
            });
        }
    }
}

/// Sleep until the slot has gone `idle` without a read, then drop its
/// value and exit. Runs once per stored value; a read meanwhile pushes the
/// deadline out.
fn reap_when_idle<T>(inner: &Mutex<Slot<T>>, idle: Duration) {
    loop {
        let wait = match inner.lock() {
            Ok(mut slot) => {
                let unused = slot.last_used.elapsed();
                if slot.cached.is_none() || unused >= idle {
                    // Freed after the lock is released.
                    let dropped = slot.cached.take();
                    slot.generation += 1;
                    slot.reaper = false;
                    drop(slot);
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
    use super::*;

    /// A cold `get` must return immediately (the slow refresh runs on a
    /// worker thread, never on the caller's thread) and the value must show
    /// up on a later `get` once the refresh lands.
    #[test]
    fn get_never_blocks_on_refresh_and_fills_afterwards() {
        let cache: BgCache<u32> = BgCache::new(Duration::from_secs(60));

        // The refresh blocks on this channel until the test releases it, so
        // if `get` were synchronous it would hang here.
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let ran = Arc::new(AtomicBool::new(false));
        let ran_in_refresh = Arc::clone(&ran);

        let first = cache.get(move || {
            let _ = release_rx.recv();
            ran_in_refresh.store(true, Ordering::SeqCst);
            99u32
        });
        assert_eq!(first, None, "cold get yields no value yet");
        assert!(
            !ran.load(Ordering::SeqCst),
            "refresh ran on the caller's thread — get() blocked"
        );

        // Release the worker and wait (bounded) for the value to land. A
        // still-in-flight refresh keeps `get` from kicking a second one, so
        // the `|| 0` fallback never overwrites the real value.
        release_tx.send(()).unwrap();
        let mut got = None;
        for _ in 0..400 {
            if let Some(value) = cache.get(|| 0u32) {
                got = Some(value);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(got, Some(99), "value appears once the refresh completes");
    }

    /// An idle-evicting cache frees its value once nobody read it for the
    /// idle period, and a read keeps it alive.
    #[test]
    fn idle_value_is_dropped_and_freed() {
        let idle = Duration::from_millis(150);
        let cache: BgCache<Arc<Vec<u8>>> = BgCache::with_idle_evict(Duration::from_secs(60), idle);
        let value = Arc::new(vec![0u8; 1024]);
        let weak = Arc::downgrade(&value);
        let _ = cache.get(move || value);
        for _ in 0..400 {
            if cache.is_cached() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(cache.is_cached(), "the refresh stored the value");
        // Reads inside the idle period keep it.
        for _ in 0..4 {
            std::thread::sleep(idle / 3);
            assert!(cache.get(|| Arc::new(Vec::new())).is_some());
        }
        assert!(weak.upgrade().is_some(), "still held while in use");
        std::thread::sleep(idle * 3);
        assert!(!cache.is_cached(), "evicted after the idle period");
        assert!(weak.upgrade().is_none(), "and the value is freed");
    }

    /// The notifier fires after a background refresh stores a value.
    #[test]
    fn notifier_fires_after_refresh() {
        let cache: BgCache<u32> = BgCache::new(Duration::from_secs(60));
        let hits = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let hits_cb = Arc::clone(&hits);
        cache.set_notifier(Arc::new(move || {
            hits_cb.fetch_add(1, Ordering::SeqCst);
        }));

        let _ = cache.get(|| 7u32);
        for _ in 0..400 {
            if hits.load(Ordering::SeqCst) >= 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            hits.load(Ordering::SeqCst) >= 1,
            "notifier should fire once"
        );
    }
}
