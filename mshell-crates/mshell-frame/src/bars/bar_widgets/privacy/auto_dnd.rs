//! Auto Do-Not-Disturb while the screen is being shared.
//!
//! Port of the noctalia `sharednd` plugin, natively on top of the privacy
//! monitor's PipeWire screencast detection (`screen_watch`). When the first
//! screencast starts, DND goes on; when the last one has been gone for
//! `notifications.auto_dnd_release_ms` (a stop/start pair while switching
//! the shared window collapses into nothing), the previous state comes back.
//!
//! Ownership: DND is only turned off again if *we* turned it on. If it was
//! already on when sharing started, it stays on afterwards. If the user
//! turns our DND off during the share, it becomes theirs: turning it on
//! again by hand keeps it on after the share. Turning the feature off
//! mid-share releases what we own.
//!
//! A shell restart mid-share needs no saved state: the DND flag lives in
//! mshell's own notification service (not persisted), so it comes back off
//! and the fresh machine takes ownership again when it sees the share.
//! The driver receives every DND write as a queued value
//! (`NotificationService::dnd_changes`), not just a wake-up, so a user's
//! off→on faster than two samples is still seen as theirs.
//!
//! The decision lives in the pure [`AutoDnd`] state machine (unit-tested);
//! [`ensure_started`] is the thin GTK-main-loop driver around it.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::future::{self, Either};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, NotificationsStoreFields};
use mshell_services::notification_service;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib;
use tokio::sync::Notify;

use super::monitor::screen_watch;

/// Woken by the screencast watch when its app list changes.
pub(super) static WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);

/// Re-read config this often while nothing happens.
const IDLE_TICK: Duration = Duration::from_secs(10);

static STARTED: AtomicBool = AtomicBool::new(false);

/// What the driver should do after one [`AutoDnd::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Step {
    /// Set DND to this value now.
    pub(crate) set_dnd: Option<bool>,
    /// Call `step` again at this instant (a pending release).
    pub(crate) recheck_at: Option<Instant>,
}

/// Screencast → DND ownership state machine.
#[derive(Debug, Default)]
pub(crate) struct AutoDnd {
    /// A share session is in progress (sharing seen, release not yet done).
    session: bool,
    /// We turned DND on for this session and must turn it off at the end.
    owned: bool,
    /// Our "DND on" has been seen since we set it, so a later "off" is the
    /// user's doing (not a read that raced our own write).
    seen_on: bool,
    /// Sharing stopped at this instant; release once the delay has passed.
    stopped_since: Option<Instant>,
}

impl AutoDnd {
    /// Feed the current inputs; returns the action to take.
    pub(crate) fn step(
        &mut self,
        enabled: bool,
        sharing: bool,
        dnd_on: bool,
        release_after: Duration,
        now: Instant,
    ) -> Step {
        let idle = Step {
            set_dnd: None,
            recheck_at: None,
        };
        if !enabled {
            return self.release(dnd_on);
        }
        self.observe_dnd(dnd_on);
        if sharing {
            self.stopped_since = None;
            if !self.session {
                self.session = true;
                if !dnd_on {
                    self.owned = true;
                    self.seen_on = false;
                    return Step {
                        set_dnd: Some(true),
                        recheck_at: None,
                    };
                }
            }
            return idle;
        }
        if !self.session {
            return idle;
        }
        let since = *self.stopped_since.get_or_insert(now);
        let deadline = since + release_after;
        if now >= deadline {
            return self.release(dnd_on);
        }
        Step {
            set_dnd: None,
            recheck_at: Some(deadline),
        }
    }

    /// Feed one DND value: a queued setter value (every write, in order) or
    /// the current value sampled by [`Self::step`].
    ///
    /// Ownership follows what the user does mid-session: once our DND was
    /// on, seeing it off means the user took over. Whatever they do with it
    /// afterwards (on again by hand) is theirs.
    pub(crate) fn observe_dnd(&mut self, dnd_on: bool) {
        if !self.owned {
            return;
        }
        if dnd_on {
            self.seen_on = true;
        } else if self.seen_on {
            self.owned = false;
        }
    }

    /// End the session: turn DND back off if we own it.
    fn release(&mut self, dnd_on: bool) -> Step {
        let owned = self.owned;
        *self = Self::default();
        Step {
            set_dnd: (owned && dnd_on).then_some(false),
            recheck_at: None,
        }
    }
}

/// Feed every queued DND value to the machine, oldest first.
pub(crate) fn drain_dnd_changes(
    machine: &mut AutoDnd,
    changes: &mut tokio::sync::mpsc::UnboundedReceiver<bool>,
) {
    while let Ok(value) = changes.try_recv() {
        machine.observe_dnd(value);
    }
}

fn enabled() -> bool {
    config_manager()
        .config()
        .notifications()
        .auto_dnd_screen_share()
        .get_untracked()
}

/// Start the driver once (idempotent). Cheap while the feature is off: it
/// keeps the screencast watch unwanted and re-reads the toggle on a slow tick.
pub(crate) fn ensure_started() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    // Every DND write (ours or the user's), queued in order. A property
    // watch or a wake-up would coalesce a quick off→on by the user into
    // one "on" sample and lose their ownership; the queue cannot.
    let mut changes = notification_service().map(|s| s.dnd_changes());
    glib::spawn_future_local(async move {
        let mut machine = AutoDnd::default();
        loop {
            if let Some(changes) = changes.as_mut() {
                drain_dnd_changes(&mut machine, changes);
            }
            let on = enabled();
            screen_watch::set_wanted_by_auto_dnd(on);
            let release_after = Duration::from_millis(u64::from(
                config_manager()
                    .config()
                    .notifications()
                    .auto_dnd_release_ms()
                    .get_untracked(),
            ));
            let service = notification_service();
            let dnd_on = service.as_ref().map(|s| s.dnd.get()).unwrap_or(false);
            let sharing = on && !screen_watch::apps().is_empty();
            let step = machine.step(on, sharing, dnd_on, release_after, Instant::now());
            if let (Some(v), Some(s)) = (step.set_dnd, service.as_ref()) {
                tracing::info!(dnd = v, "auto-dnd: screen sharing changed");
                s.set_dnd(v);
            }
            let tick = step
                .recheck_at
                .map(|t| t.saturating_duration_since(Instant::now()))
                .unwrap_or(IDLE_TICK)
                .min(IDLE_TICK);
            let woken = std::pin::pin!(glib::future_with_timeout(tick, WAKE.notified()));
            let changed = std::pin::pin!(async {
                match changes.as_mut() {
                    Some(changes) => changes.recv().await,
                    None => std::future::pending().await,
                }
            });
            if let Either::Right((Some(value), _)) = future::select(woken, changed).await {
                machine.observe_dnd(value);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELAY: Duration = Duration::from_millis(1000);

    fn ms(base: Instant, n: u64) -> Instant {
        base + Duration::from_millis(n)
    }

    #[test]
    fn share_start_turns_dnd_on_and_release_turns_it_off_after_delay() {
        let t = Instant::now();
        let mut m = AutoDnd::default();
        assert_eq!(m.step(true, true, false, DELAY, t).set_dnd, Some(true));
        // Still sharing, DND now on: nothing to do.
        assert_eq!(m.step(true, true, true, DELAY, ms(t, 100)).set_dnd, None);
        // Stopped: not yet — wait for the delay.
        let s = m.step(true, false, true, DELAY, ms(t, 200));
        assert_eq!(s.set_dnd, None);
        assert_eq!(s.recheck_at, Some(ms(t, 1200)));
        assert_eq!(m.step(true, false, true, DELAY, ms(t, 700)).set_dnd, None);
        // Delay passed: restore.
        assert_eq!(
            m.step(true, false, true, DELAY, ms(t, 1200)).set_dnd,
            Some(false)
        );
        // Idle afterwards.
        assert_eq!(
            m.step(true, false, false, DELAY, ms(t, 5000)),
            Step {
                set_dnd: None,
                recheck_at: None
            }
        );
    }

    #[test]
    fn stop_start_pair_inside_the_delay_does_not_flap() {
        let t = Instant::now();
        let mut m = AutoDnd::default();
        assert_eq!(m.step(true, true, false, DELAY, t).set_dnd, Some(true));
        assert_eq!(m.step(true, false, true, DELAY, ms(t, 100)).set_dnd, None);
        // Restarted before the deadline (switching the shared window).
        assert_eq!(m.step(true, true, true, DELAY, ms(t, 400)).set_dnd, None);
        // A later stop starts a fresh delay window.
        let s = m.step(true, false, true, DELAY, ms(t, 2000));
        assert_eq!(s.recheck_at, Some(ms(t, 3000)));
        assert_eq!(
            m.step(true, false, true, DELAY, ms(t, 3000)).set_dnd,
            Some(false)
        );
    }

    #[test]
    fn dnd_already_on_is_left_on_after_sharing() {
        let t = Instant::now();
        let mut m = AutoDnd::default();
        assert_eq!(m.step(true, true, true, DELAY, t).set_dnd, None);
        assert_eq!(m.step(true, false, true, DELAY, ms(t, 10)).set_dnd, None);
        assert_eq!(m.step(true, false, true, DELAY, ms(t, 2000)).set_dnd, None);
    }

    #[test]
    fn user_turning_dnd_off_mid_share_is_not_fought() {
        let t = Instant::now();
        let mut m = AutoDnd::default();
        assert_eq!(m.step(true, true, false, DELAY, t).set_dnd, Some(true));
        // User switched DND off during the share: don't turn it back on.
        assert_eq!(m.step(true, true, false, DELAY, ms(t, 100)).set_dnd, None);
        // And at the end there's nothing to restore.
        m.step(true, false, false, DELAY, ms(t, 200));
        assert_eq!(m.step(true, false, false, DELAY, ms(t, 1300)).set_dnd, None);
    }

    /// The user turned our DND off and back on during the share: it is
    /// theirs now, so the end of the share leaves it on.
    #[test]
    fn user_taking_over_dnd_mid_share_keeps_it_on_afterwards() {
        let t = Instant::now();
        let mut m = AutoDnd::default();
        assert_eq!(m.step(true, true, false, DELAY, t).set_dnd, Some(true));
        assert_eq!(m.step(true, true, true, DELAY, ms(t, 100)).set_dnd, None);
        // User: DND off …
        assert_eq!(m.step(true, true, false, DELAY, ms(t, 200)).set_dnd, None);
        // … and on again by hand.
        assert_eq!(m.step(true, true, true, DELAY, ms(t, 300)).set_dnd, None);
        assert_eq!(m.step(true, false, true, DELAY, ms(t, 400)).set_dnd, None);
        assert_eq!(m.step(true, false, true, DELAY, ms(t, 1400)).set_dnd, None);
    }

    /// A step that runs before our own "DND on" is visible must not read as
    /// the user turning it off.
    #[test]
    fn ownership_survives_a_read_before_our_dnd_shows_up() {
        let t = Instant::now();
        let mut m = AutoDnd::default();
        assert_eq!(m.step(true, true, false, DELAY, t).set_dnd, Some(true));
        assert_eq!(m.step(true, true, false, DELAY, ms(t, 1)).set_dnd, None);
        assert_eq!(m.step(true, true, true, DELAY, ms(t, 50)).set_dnd, None);
        m.step(true, false, true, DELAY, ms(t, 100));
        assert_eq!(
            m.step(true, false, true, DELAY, ms(t, 1100)).set_dnd,
            Some(false)
        );
    }

    /// Review item 84: the user turns our DND off and on again faster than
    /// the driver samples. Both samples read "on", so only the queued
    /// setter values show that the user took DND over.
    #[test]
    fn fast_user_off_on_between_two_samples_still_hands_dnd_to_the_user() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let t = Instant::now();
        let mut m = AutoDnd::default();
        assert_eq!(m.step(true, true, false, DELAY, t).set_dnd, Some(true));
        // Scripted sequence, all before the next sample: our own "on",
        // then the user's "off" and "on".
        for value in [true, false, true] {
            tx.send(value).expect("receiver alive");
        }
        drain_dnd_changes(&mut m, &mut rx);
        assert!(rx.try_recv().is_err(), "queue fully drained");
        assert_eq!(m.step(true, true, true, DELAY, ms(t, 100)).set_dnd, None);
        m.step(true, false, true, DELAY, ms(t, 200));
        assert_eq!(m.step(true, false, true, DELAY, ms(t, 1200)).set_dnd, None);
    }

    /// Our own "on" arriving through the queue is not a user action, and
    /// values queued before the share started don't touch ownership.
    #[test]
    fn queued_own_writes_and_pre_share_values_keep_ownership() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let t = Instant::now();
        let mut m = AutoDnd::default();
        for value in [true, false] {
            tx.send(value).expect("receiver alive");
        }
        drain_dnd_changes(&mut m, &mut rx);
        assert_eq!(m.step(true, true, false, DELAY, t).set_dnd, Some(true));
        tx.send(true).expect("receiver alive");
        drain_dnd_changes(&mut m, &mut rx);
        m.step(true, false, true, DELAY, ms(t, 100));
        assert_eq!(
            m.step(true, false, true, DELAY, ms(t, 1100)).set_dnd,
            Some(false)
        );
    }

    #[test]
    fn disabling_mid_share_releases_owned_dnd_immediately() {
        let t = Instant::now();
        let mut m = AutoDnd::default();
        assert_eq!(m.step(true, true, false, DELAY, t).set_dnd, Some(true));
        assert_eq!(
            m.step(false, true, true, DELAY, ms(t, 50)).set_dnd,
            Some(false)
        );
        // Disabled: sharing is ignored.
        assert_eq!(m.step(false, true, false, DELAY, ms(t, 60)).set_dnd, None);
    }

    #[test]
    fn disabled_never_touches_dnd() {
        let t = Instant::now();
        let mut m = AutoDnd::default();
        assert_eq!(m.step(false, true, false, DELAY, t).set_dnd, None);
        assert_eq!(m.step(false, false, true, DELAY, ms(t, 5000)).set_dnd, None);
    }
}
