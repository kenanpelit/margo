//! In-memory stopwatch — transient (never persisted), shared
//! process-wide. Backs the Stopwatch tab of the Alarm Clock menu and
//! the bar pill's live "running" readout.
//!
//! A single instance behind a `Mutex`. All access is from the GTK
//! main thread today, but the `Mutex` keeps it sound if a worker ever
//! peeks. The accumulator model mirrors a physical stopwatch:
//! `accumulated` holds the time from completed run segments, and while
//! Running the live time is `accumulated + started_at.elapsed()`.

use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::watch;

static INSTANCE: Mutex<Stopwatch> = Mutex::new(Stopwatch::new());

/// Bumped whenever the stopwatch's state changes (and, via
/// [`notify_changed`], when the alarm starts or stops ringing), so the bar
/// pill can sleep until something happens instead of polling.
static CHANGES: LazyLock<watch::Sender<u64>> = LazyLock::new(|| watch::channel(0).0);

/// Subscribe to stopwatch / alarm activity changes.
pub(crate) fn changes() -> watch::Receiver<u64> {
    CHANGES.subscribe()
}

/// Signal a change to every [`changes`] subscriber. Callable from any thread
/// (the alarm thread reports through this).
pub(crate) fn notify_changed() {
    CHANGES.send_modify(|generation| *generation = generation.wrapping_add(1));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopwatchState {
    Stopped,
    Running,
    Paused,
}

struct Stopwatch {
    state: StopwatchState,
    /// Time from previous run segments; frozen while paused / stopped.
    accumulated: Duration,
    /// Start of the current run segment — only set while Running.
    started_at: Option<Instant>,
}

impl Stopwatch {
    const fn new() -> Self {
        Self {
            state: StopwatchState::Stopped,
            accumulated: Duration::ZERO,
            started_at: None,
        }
    }
}

fn lock() -> std::sync::MutexGuard<'static, Stopwatch> {
    INSTANCE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Current run state.
pub(crate) fn state() -> StopwatchState {
    lock().state
}

/// Live elapsed time (advances in real time while Running).
pub(crate) fn elapsed() -> Duration {
    let sw = lock();
    match sw.state {
        StopwatchState::Running => {
            sw.accumulated + sw.started_at.map(|s| s.elapsed()).unwrap_or_default()
        }
        _ => sw.accumulated,
    }
}

/// Start a stopped stopwatch or resume a paused one. No-op if running.
pub(crate) fn start() {
    let mut sw = lock();
    if sw.state != StopwatchState::Running {
        sw.started_at = Some(Instant::now());
        sw.state = StopwatchState::Running;
        drop(sw);
        notify_changed();
    }
}

/// Pause a running stopwatch, folding the live segment into the
/// accumulator. No-op unless running.
pub(crate) fn pause() {
    let mut sw = lock();
    if sw.state == StopwatchState::Running {
        if let Some(started) = sw.started_at.take() {
            sw.accumulated += started.elapsed();
        }
        sw.state = StopwatchState::Paused;
        drop(sw);
        notify_changed();
    }
}

/// Reset to zero and stop.
pub(crate) fn reset() {
    let mut sw = lock();
    sw.state = StopwatchState::Stopped;
    sw.accumulated = Duration::ZERO;
    sw.started_at = None;
    drop(sw);
    notify_changed();
}

/// Format an elapsed duration as `MM:SS.cc` (centiseconds), promoting
/// to `H:MM:SS.cc` past an hour — compact enough for the bar pill yet
/// precise enough for the menu's hero readout.
pub(crate) fn format_elapsed(d: Duration) -> String {
    let total_cs = d.as_millis() / 10; // centiseconds
    let cs = total_cs % 100;
    let total_secs = total_cs / 100;
    let secs = total_secs % 60;
    let total_mins = total_secs / 60;
    let mins = total_mins % 60;
    let hours = total_mins / 60;
    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}.{cs:02}")
    } else {
        format!("{mins:02}:{secs:02}.{cs:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_changes_reach_subscribers() {
        let mut rx = changes();
        rx.mark_unchanged();
        start();
        assert!(rx.has_changed().unwrap_or(false), "start notifies");
        rx.mark_unchanged();
        start(); // already running: no change
        assert!(!rx.has_changed().unwrap_or(true), "no-op start is silent");
        pause();
        assert!(rx.has_changed().unwrap_or(false), "pause notifies");
        rx.mark_unchanged();
        reset();
        assert!(rx.has_changed().unwrap_or(false), "reset notifies");
    }
}
