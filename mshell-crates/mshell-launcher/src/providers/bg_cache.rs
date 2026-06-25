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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::provider::RefreshNotifier;

struct Cached<T> {
    value: T,
    captured_at: Instant,
}

pub(crate) struct BgCache<T> {
    inner: Arc<Mutex<Option<Cached<T>>>>,
    in_flight: Arc<AtomicBool>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
    ttl: Duration,
}

impl<T: Clone + Send + 'static> BgCache<T> {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            in_flight: Arc::new(AtomicBool::new(false)),
            notifier: Arc::new(Mutex::new(None)),
            ttl,
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
        let (value, fresh) = match self.inner.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(cached) => (
                    Some(cached.value.clone()),
                    cached.captured_at.elapsed() < self.ttl,
                ),
                None => (None, false),
            },
            Err(_) => (None, false),
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
            std::thread::spawn(move || {
                let value = refresh();
                if let Ok(mut guard) = inner.lock() {
                    *guard = Some(Cached {
                        value,
                        captured_at: Instant::now(),
                    });
                }
                in_flight.store(false, Ordering::Release);
                if let Ok(slot) = notifier.lock()
                    && let Some(notify) = slot.as_ref()
                {
                    notify();
                }
            });
        }

        value
    }

    /// Drop the cached snapshot so the next [`Self::get`] is treated as
    /// cold and kicks a fresh off-thread refresh. Used by providers
    /// whose inputs changed (e.g. a panel-open asked for a re-scan):
    /// the rescan still happens on a worker thread, not the caller's.
    pub(crate) fn invalidate(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = None;
        }
    }

    /// Seed the snapshot directly (no refresh). Test helper for
    /// injecting a deterministic value.
    #[cfg(test)]
    pub(crate) fn seed(&self, value: T) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = Some(Cached {
                value,
                captured_at: Instant::now(),
            });
        }
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
