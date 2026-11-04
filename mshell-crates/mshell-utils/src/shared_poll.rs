//! One poller per data source, shared by every widget that shows it.
//!
//! Pills used to run their own `sleep → probe` loop each: one per bar, so
//! one per monitor, and still running while the pill sat collapsed in the
//! hidden drawer. A [`SharedPoll`] runs a single loop for the whole process
//! and only while at least one [`PollSubscription`] is active (typically:
//! its widget is mapped). Subscribers read the latest value from a `watch`
//! channel.
//!
//! A source that can report its own changes gets an [`EventFeed`]: a
//! long-running child (`podman events`, `mvpn status --follow`) whose stdout
//! lines trigger the probe, so the timer only backs it up. The child runs
//! while a subscriber is active; when it can't start or exits, the poll
//! falls back to its plain interval and the feed retries with a backoff.

use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::sync::{Notify, watch};
use tracing::{debug, warn};

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
    /// `true` while at least one subscription is active; event feeds wait
    /// on it to start and stop their child.
    shown: watch::Sender<bool>,
    feed: Option<EventFeed>,
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
            shown: watch::channel(false).0,
            feed: None,
        }
    }

    /// Drive this poll from `feed` as well (see [`EventFeed`]).
    pub fn with_feed(mut self, feed: EventFeed) -> Self {
        self.feed = Some(feed);
        self
    }

    /// The feed's child is running and reading events, so the source's own
    /// change reports keep the value fresh.
    pub fn feed_alive(&self) -> bool {
        self.feed
            .as_ref()
            .is_some_and(|f| f.alive.load(Ordering::SeqCst))
    }

    fn current_interval(&self) -> Duration {
        match &self.feed {
            Some(feed) if feed.alive.load(Ordering::SeqCst) => (feed.interval_while_alive)(),
            _ => (self.interval)(),
        }
    }

    /// A new, initially inactive subscription. Starts the loop task on the
    /// first call; the loop idles until a subscription is activated.
    pub fn subscribe(&'static self) -> PollSubscription<T> {
        if !self.started.swap(true, Ordering::SeqCst) {
            (self.spawn)(Box::pin(self.run()));
            if self.feed.is_some() {
                (self.spawn)(Box::pin(self.run_feed()));
            }
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
                let _ = tokio::time::timeout(self.current_interval(), woken).await;
            }
        }
    }
}

impl<T: Clone + PartialEq + Send + Sync + 'static> SharedPoll<T> {
    async fn run_feed(&'static self) {
        let Some(feed) = &self.feed else {
            return;
        };
        let mut shown = self.shown.subscribe();
        let mut backoff = feed.retry;
        loop {
            if shown.wait_for(|on| *on).await.is_err() {
                return;
            }
            let mut cmd = (feed.command)();
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let mut child = match cmd.spawn() {
                Ok(child) => child,
                Err(err) => {
                    debug!(error = %err, "event feed: spawn failed; polling instead");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(FEED_MAX_RETRY);
                    continue;
                }
            };
            let Some(stdout) = child.stdout.take() else {
                continue;
            };
            feed.alive.store(true, Ordering::SeqCst);
            let started = tokio::time::Instant::now();
            let mut lines = BufReader::new(stdout);
            let hidden = async {
                let _ = shown.wait_for(|on| !*on).await;
            };
            let exited = tokio::select! {
                () = pump(self, &mut lines, feed.on_line) => true,
                () = hidden => false,
            };
            feed.alive.store(false, Ordering::SeqCst);
            // Closing stdin is the child's cue to clean up and exit (`mvpn`
            // stops its own `mullvad` child); kill it if it doesn't.
            drop(child.stdin.take());
            if tokio::time::timeout(FEED_EXIT_GRACE, child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
            if exited {
                // The fallback poll takes over at once; restart the feed
                // later. A child that ran for a while resets the backoff.
                self.refresh_now();
                if started.elapsed() > FEED_MAX_RETRY {
                    backoff = feed.retry;
                }
                warn!(
                    retry_s = backoff.as_secs(),
                    "event feed exited; polling until it restarts"
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(FEED_MAX_RETRY);
            }
        }
    }
}

/// Longest wait between feed restarts.
const FEED_MAX_RETRY: Duration = Duration::from_secs(600);
/// How long a stopped feed child gets to exit after its stdin closes.
const FEED_EXIT_GRACE: Duration = Duration::from_secs(1);
/// Events arriving in a burst (`podman run`: create, init, start, …) cost
/// one probe: it runs once the feed has been quiet this long.
const FEED_DEBOUNCE: Duration = Duration::from_millis(300);

/// Read `lines` until EOF, asking `poll` for a probe after each burst of
/// lines `on_line` accepts.
async fn pump<T: Clone + PartialEq + Send + Sync + 'static>(
    poll: &SharedPoll<T>,
    lines: &mut (impl AsyncBufRead + Unpin),
    on_line: fn(&str) -> bool,
) {
    // next_line is cancellation-safe; read_line would lose a partially
    // read event whenever the debounce timeout wins.
    let mut lines = lines.lines();
    let mut dirty = false;
    loop {
        let read = if dirty {
            match tokio::time::timeout(FEED_DEBOUNCE, lines.next_line()).await {
                Ok(read) => read,
                Err(_) => {
                    dirty = false;
                    poll.refresh_now();
                    continue;
                }
            }
        } else {
            lines.next_line().await
        };
        match read {
            Ok(None) | Err(_) => {
                if dirty {
                    poll.refresh_now();
                }
                return;
            }
            Ok(Some(line)) => dirty |= on_line(line.trim_end()),
        }
    }
}

/// A change stream for a [`SharedPoll`]: a long-running command whose
/// stdout lines report changes. Keep it running only while a subscriber is
/// active; the child gets its stdin closed when it should stop.
pub struct EventFeed {
    command: fn() -> tokio::process::Command,
    /// Called per stdout line (trailing newline trimmed); `true` asks for a
    /// probe.
    on_line: fn(&str) -> bool,
    /// The poll interval while the feed runs (a slow safety net, or the
    /// cadence of whatever else the probe reads).
    interval_while_alive: fn() -> Duration,
    /// First wait before restarting a feed that couldn't start or exited;
    /// doubles up to ten minutes.
    retry: Duration,
    alive: AtomicBool,
}

impl EventFeed {
    pub fn new(
        command: fn() -> tokio::process::Command,
        on_line: fn(&str) -> bool,
        interval_while_alive: fn() -> Duration,
        retry: Duration,
    ) -> Self {
        Self {
            command,
            on_line,
            interval_while_alive,
            retry,
            alive: AtomicBool::new(false),
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
                self.poll.shown.send_replace(true);
                self.poll.wake.notify_waiters();
            }
        } else if self.poll.active.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.poll.shown.send_replace(false);
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

    static C_PROBES: AtomicUsize = AtomicUsize::new(0);
    static C: LazyLock<SharedPoll<usize>> = LazyLock::new(|| {
        SharedPoll::with_spawner(
            ten_secs,
            || Box::pin(async { C_PROBES.fetch_add(1, Ordering::SeqCst) + 1 }),
            spawn_here,
        )
    });

    fn only_changes(line: &str) -> bool {
        line.starts_with("change")
    }

    /// A burst of events costs one probe, irrelevant lines none, and the
    /// probe runs once the burst has settled.
    #[tokio::test(start_paused = true)]
    async fn feed_lines_probe_once_per_burst() {
        use tokio::io::AsyncWriteExt as _;
        let pill = C.subscribe();
        pill.set_active(true);
        settle().await;
        assert_eq!(C_PROBES.load(Ordering::SeqCst), 1);

        let (mut tx, rx) = tokio::io::duplex(1024);
        let pumping = tokio::spawn(async move {
            let mut lines = BufReader::new(rx);
            pump(&C, &mut lines, only_changes).await;
        });
        tx.write_all(b"change 1\nnoise\nchange 2\nchange 3\n")
            .await
            .unwrap();
        settle().await;
        assert_eq!(C_PROBES.load(Ordering::SeqCst), 1, "still in the burst");
        tokio::time::sleep(FEED_DEBOUNCE * 2).await;
        settle().await;
        assert_eq!(C_PROBES.load(Ordering::SeqCst), 2, "one probe per burst");

        tx.write_all(b"noise\n").await.unwrap();
        tokio::time::sleep(FEED_DEBOUNCE * 2).await;
        settle().await;
        assert_eq!(C_PROBES.load(Ordering::SeqCst), 2, "noise asks for nothing");

        // A change right before EOF is not lost.
        tx.write_all(b"change 4\n").await.unwrap();
        drop(tx);
        pumping.await.unwrap();
        settle().await;
        assert_eq!(C_PROBES.load(Ordering::SeqCst), 3);
    }

    static D_PROBES: AtomicUsize = AtomicUsize::new(0);
    static D: LazyLock<SharedPoll<usize>> = LazyLock::new(|| {
        SharedPoll::with_spawner(
            ten_secs,
            || Box::pin(async { D_PROBES.fetch_add(1, Ordering::SeqCst) + 1 }),
            spawn_here,
        )
        .with_feed(EventFeed::new(
            || {
                // Reports one change, then idles until stdin closes.
                let mut cmd = tokio::process::Command::new("sh");
                cmd.args(["-c", "echo change; exec cat"]);
                cmd
            },
            only_changes,
            || Duration::from_secs(600),
            Duration::from_secs(1),
        ))
    });

    async fn until(what: &str, cond: impl Fn() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// The feed child runs only while someone is shown, its change line
    /// triggers a probe, and it is stopped (stdin closed) when hidden.
    #[tokio::test]
    async fn feed_child_runs_while_shown() {
        let pill = D.subscribe();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!D.feed_alive(), "nobody shown, no child");
        assert_eq!(D_PROBES.load(Ordering::SeqCst), 0);

        pill.set_active(true);
        until("feed starts", || D.feed_alive()).await;
        // Activation probe + the child's change line.
        until("change probed", || D_PROBES.load(Ordering::SeqCst) >= 2).await;
        assert_eq!(D.current_interval(), Duration::from_secs(600));

        pill.set_active(false);
        until("feed stops", || !D.feed_alive()).await;
        assert_eq!(D.current_interval(), ten_secs());
    }
}
