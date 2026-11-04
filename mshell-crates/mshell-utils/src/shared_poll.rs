//! One poller per data source, shared by every widget that shows it.
//!
//! Pills used to run their own `sleep → probe` loop each: one per bar, so
//! one per monitor, and still running while the pill sat collapsed in the
//! hidden drawer. A [`SharedPoll`] runs a single loop for the whole process
//! and only while at least one [`PollSubscription`] is active (typically:
//! its widget is mapped). Subscribers read the latest value from a `watch`
//! channel.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::{Notify, watch};

/// A boxed probe / loop future.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// A process-wide poll loop. Keep it in a `static` (`LazyLock`).
pub struct SharedPoll<T: 'static> {
    interval: fn() -> Duration,
    probe: fn() -> BoxFuture<T>,
    spawn: fn(BoxFuture<()>),
    tx: watch::Sender<Option<T>>,
    active: AtomicUsize,
    /// Woken (without stored permits) on the first activation and on
    /// [`SharedPoll::refresh_now`].
    wake: Notify,
    /// A refresh was asked for since the last probe started.
    refresh: AtomicBool,
    started: AtomicBool,
}

impl<T: Clone + PartialEq + Send + Sync + 'static> SharedPoll<T> {
    /// A poll of `probe` every `interval()` (re-read each cycle, so it can
    /// follow config), run on the shared services runtime.
    pub fn new(interval: fn() -> Duration, probe: fn() -> BoxFuture<T>) -> Self {
        Self::with_spawner(interval, probe, |fut| {
            mshell_services::tokio_rt().spawn(fut);
        })
    }

    /// [`Self::new`] with an explicit spawner for the loop task.
    pub fn with_spawner(
        interval: fn() -> Duration,
        probe: fn() -> BoxFuture<T>,
        spawn: fn(BoxFuture<()>),
    ) -> Self {
        Self {
            interval,
            probe,
            spawn,
            tx: watch::channel(None).0,
            active: AtomicUsize::new(0),
            wake: Notify::new(),
            refresh: AtomicBool::new(false),
            started: AtomicBool::new(false),
        }
    }

    /// A new, initially inactive subscription. Starts the loop task on the
    /// first call; the loop idles until a subscription is activated.
    pub fn subscribe(&'static self) -> PollSubscription<T> {
        if !self.started.swap(true, Ordering::SeqCst) {
            (self.spawn)(Box::pin(self.run()));
        }
        PollSubscription {
            poll: self,
            active: AtomicBool::new(false),
        }
    }

    /// Probe again now instead of at the end of the interval (after an
    /// action that changed the state). Applies once a subscriber is active.
    pub fn refresh_now(&self) {
        self.refresh.store(true, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    /// Number of active subscriptions.
    pub fn active_subscribers(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    async fn run(&'static self) {
        loop {
            // Idle until someone shows the value. The waiter is registered
            // before the check, so an activation in between isn't missed.
            loop {
                let woken = self.wake.notified();
                tokio::pin!(woken);
                woken.as_mut().enable();
                if self.active.load(Ordering::SeqCst) > 0 {
                    break;
                }
                woken.await;
            }
            // A refresh asked for while this probe runs probes again after.
            self.refresh.store(false, Ordering::SeqCst);
            let value = (self.probe)().await;
            self.tx.send_if_modified(|cur| {
                let changed = cur.as_ref() != Some(&value);
                *cur = Some(value);
                changed
            });
            let woken = self.wake.notified();
            tokio::pin!(woken);
            woken.as_mut().enable();
            if !self.refresh.load(Ordering::SeqCst) {
                let _ = tokio::time::timeout((self.interval)(), woken).await;
            }
        }
    }
}

/// One widget's interest in a [`SharedPoll`]. Counts towards keeping the
/// loop running only while active; dropping it deactivates it.
pub struct PollSubscription<T: 'static> {
    poll: &'static SharedPoll<T>,
    active: AtomicBool,
}

impl<T: 'static> PollSubscription<T> {
    /// Mark this subscriber as showing the value (e.g. its widget mapped)
    /// or not. Idempotent.
    pub fn set_active(&self, on: bool) {
        if self.active.swap(on, Ordering::SeqCst) == on {
            return;
        }
        if on {
            if self.poll.active.fetch_add(1, Ordering::SeqCst) == 0 {
                self.poll.wake.notify_waiters();
            }
        } else {
            self.poll.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// The latest value (`None` before the first probe). It is `Send`, so
    /// it can move into a widget's command future.
    pub fn receiver(&self) -> watch::Receiver<Option<T>> {
        self.poll.tx.subscribe()
    }
}

impl<T: 'static> Drop for PollSubscription<T> {
    fn drop(&mut self) {
        self.set_active(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    fn spawn_here(fut: BoxFuture<()>) {
        tokio::spawn(fut);
    }

    fn ten_secs() -> Duration {
        Duration::from_secs(10)
    }

    static A_PROBES: AtomicUsize = AtomicUsize::new(0);
    static A: LazyLock<SharedPoll<usize>> = LazyLock::new(|| {
        SharedPoll::with_spawner(
            ten_secs,
            || Box::pin(async { A_PROBES.fetch_add(1, Ordering::SeqCst) + 1 }),
            spawn_here,
        )
    });

    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn one_loop_for_all_subscribers_and_none_while_hidden() {
        let bar1 = A.subscribe();
        let bar2 = A.subscribe();
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(A_PROBES.load(Ordering::SeqCst), 0, "nobody visible");

        bar1.set_active(true);
        bar2.set_active(true);
        bar2.set_active(true); // idempotent
        settle().await;
        assert_eq!(A_PROBES.load(Ordering::SeqCst), 1, "probes on activation");
        assert_eq!(*bar1.receiver().borrow(), Some(1));

        // Two visible pills, three intervals: three probes, not six.
        tokio::time::sleep(Duration::from_secs(35)).await;
        settle().await;
        assert_eq!(A_PROBES.load(Ordering::SeqCst), 4);
        assert_eq!(A.active_subscribers(), 2);

        // Both hidden (one by dropping it): the loop goes quiet.
        bar1.set_active(false);
        drop(bar2);
        assert_eq!(A.active_subscribers(), 0);
        tokio::time::sleep(Duration::from_secs(10)).await;
        settle().await;
        let after = A_PROBES.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(120)).await;
        settle().await;
        assert_eq!(A_PROBES.load(Ordering::SeqCst), after);
    }

    static B_PROBES: AtomicUsize = AtomicUsize::new(0);
    static B: LazyLock<SharedPoll<usize>> = LazyLock::new(|| {
        SharedPoll::with_spawner(
            ten_secs,
            || Box::pin(async { B_PROBES.fetch_add(1, Ordering::SeqCst) + 1 }),
            spawn_here,
        )
    });

    #[tokio::test(start_paused = true)]
    async fn refresh_now_probes_early() {
        let pill = B.subscribe();
        pill.set_active(true);
        settle().await;
        assert_eq!(B_PROBES.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_secs(2)).await;
        B.refresh_now();
        settle().await;
        assert_eq!(B_PROBES.load(Ordering::SeqCst), 2);
    }
}
