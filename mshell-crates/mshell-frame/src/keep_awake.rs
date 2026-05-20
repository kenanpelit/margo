//! Timed keep-awake session — a thin scheduler on top of
//! [`mshell_idle::IdleInhibitor`]. Ports the noctalia `keep-awake-plus`
//! duration model: pick a duration and the idle inhibitor is held for
//! that long, then auto-released. `None` minutes = hold indefinitely
//! (until the user turns it off).
//!
//! The inhibitor's on/off bool stays the source of truth for "is
//! something keeping us awake" (so `mctl` toggles still light the
//! pill); this layer only owns the optional *deadline*. Widgets read
//! the deadline via [`KeepAwakeSession::watch`] and recompute the
//! remaining time themselves.
//!
//! mshell's inhibitor is single-mode (it blocks the compositor's whole
//! idle ladder — dim / lock / suspend), so there is no partial/full
//! scope split like the QML plugin has.

use mshell_idle::inhibitor::IdleInhibitor;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tracing::warn;

static INSTANCE: OnceLock<KeepAwakeSession> = OnceLock::new();

pub(crate) struct KeepAwakeSession {
    /// Current auto-release deadline. `None` = inactive or unlimited.
    deadline_tx: watch::Sender<Option<Instant>>,
    /// Bumped on every (re)arm so a stale expiry task knows to bail.
    epoch: AtomicU64,
}

impl KeepAwakeSession {
    pub(crate) fn global() -> &'static Self {
        INSTANCE.get_or_init(|| {
            let (deadline_tx, _rx) = watch::channel(None);
            KeepAwakeSession { deadline_tx, epoch: AtomicU64::new(0) }
        })
    }

    /// Subscribe to deadline changes (drives the countdown display).
    pub(crate) fn watch(&self) -> watch::Receiver<Option<Instant>> {
        self.deadline_tx.subscribe()
    }

    /// Current deadline snapshot.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        *self.deadline_tx.borrow()
    }

    /// Seconds left until auto-release, or `None` when unlimited /
    /// inactive.
    pub(crate) fn remaining(&self) -> Option<Duration> {
        self.deadline()
            .map(|d| d.saturating_duration_since(Instant::now()))
    }

    /// Enable the inhibitor for `minutes` (or indefinitely when
    /// `None`), replacing any running session.
    pub(crate) fn activate(&'static self, minutes: Option<u64>) {
        let deadline = minutes.map(|m| Instant::now() + Duration::from_secs(m * 60));
        self.arm(deadline, true);
    }

    /// Push the current deadline out by `minutes`. No-op unless a timed
    /// session is running.
    pub(crate) fn extend(&'static self, minutes: u64) {
        if let Some(current) = self.deadline() {
            let base = current.max(Instant::now());
            self.arm(Some(base + Duration::from_secs(minutes * 60)), true);
        }
    }

    /// Release the inhibitor and cancel any pending expiry.
    pub(crate) fn deactivate(&'static self) {
        self.arm(None, false);
    }

    /// (Re)arm the session: bump the epoch (cancelling any in-flight
    /// expiry task), then drive the inhibitor + deadline off the main
    /// loop. When `enable` and a deadline is set, spawn a one-shot
    /// expiry that releases the inhibitor unless re-armed first.
    fn arm(&'static self, deadline: Option<Instant>, enable: bool) {
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        relm4::spawn(async move {
            if enable {
                if let Err(e) = IdleInhibitor::global().enable().await {
                    warn!(error = %e, "keep_awake: enable failed");
                    return;
                }
            } else {
                IdleInhibitor::global().disable().await;
            }
            let _ = self.deadline_tx.send(deadline);

            if let Some(dl) = deadline {
                tokio::time::sleep_until(tokio::time::Instant::from_std(dl)).await;
                // Only fire if this is still the live session.
                if self.epoch.load(Ordering::SeqCst) == epoch {
                    IdleInhibitor::global().disable().await;
                    let _ = self.deadline_tx.send(None);
                }
            }
        });
    }
}

/// Pretty `1h 23m` / `45m` / `30s` from a remaining duration.
pub(crate) fn format_remaining(d: Duration) -> String {
    let total = d.as_secs();
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}
