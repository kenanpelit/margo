//! What margo does when the session locker dies while the session is
//! locked: keep the session locked and start the locker again.
//!
//! `ext-session-lock-v1` already keeps the session locked when the locking
//! client dies (the screen stays black). Before this module the way out was
//! `force_unlock`, which tore the lock down without any authentication once
//! the locker was gone: anyone at the keyboard could open the session after
//! mlock was OOM-killed or crashed. Now:
//!
//! * margo notices the dead owner (polled once per loop iteration by
//!   [`MargoState::check_lock_owner`]) and spawns [`LOCKER_COMMAND`]; the new
//!   locker takes over the still-locked session (`lock()` accepts a new lock
//!   whose previous owner is dead) and asks for the password again.
//! * Automatic respawns are bounded ([`MAX_AUTO_RESPAWNS`] within
//!   [`RESPAWN_WINDOW`]); after that the session simply stays locked and
//!   margo retries once the window has passed.
//! * `force_unlock` (bind or `mctl dispatch`) only *restarts* the locker and
//!   never unlocks.
//! * The only way to unlock without the locker is an authenticated one from
//!   outside the locked seat: `loginctl unlock-session` from a TTY or ssh
//!   ([`MargoState::logind_unlock`], wired from logind's `Unlock` signal).
//!
//! The decisions live in [`LockGuard::decide`], a pure state machine.

use std::time::{Duration, Instant};

use smithay::reexports::wayland_server::Resource;

use super::MargoState;

/// The locker margo (re)starts: margo's own lock screen.
pub const LOCKER_COMMAND: &str = "mlock";
/// Automatic restarts allowed within [`RESPAWN_WINDOW`].
pub const MAX_AUTO_RESPAWNS: u32 = 3;
/// How long a freshly spawned locker gets to take the lock before margo
/// considers starting another one (also debounces `force_unlock`).
pub const SPAWN_GRACE: Duration = Duration::from_secs(5);
/// A restart this long after the previous one starts a new count.
pub const RESPAWN_WINDOW: Duration = Duration::from_secs(60);

/// Why margo is considering a locker restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockerTrigger {
    /// The session is locked and its owner (the locker) is gone.
    OwnerGone,
    /// The user asked for it (`force_unlock` with a dead locker).
    ForceUnlock,
}

/// What to do about a dead locker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockerDecision {
    /// Start the locker now.
    Spawn,
    /// A locker was started moments ago; give it time to lock.
    Wait,
    /// The automatic restarts are used up: report it once and stay locked.
    GiveUp,
    /// Already given up; stay locked quietly until the window passes.
    Idle,
}

/// Locker-restart bookkeeping for one lock (reset by an authenticated
/// unlock).
pub struct LockGuard {
    auto_spawns: u32,
    last_spawn: Option<Instant>,
    gave_up: bool,
    retry_timer: Option<calloop::RegistrationToken>,
    /// Starts the locker. A field so tests can count spawns instead of
    /// launching a real mlock.
    pub spawner: fn() -> anyhow::Result<()>,
}

impl Default for LockGuard {
    fn default() -> Self {
        Self {
            auto_spawns: 0,
            last_spawn: None,
            gave_up: false,
            retry_timer: None,
            spawner: default_spawner,
        }
    }
}

#[cfg(not(test))]
fn default_spawner() -> anyhow::Result<()> {
    crate::utils::spawn([LOCKER_COMMAND])
}

#[cfg(test)]
thread_local! {
    static TEST_SPAWNS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Tests never launch a real locker: they count the attempts instead.
#[cfg(test)]
fn default_spawner() -> anyhow::Result<()> {
    TEST_SPAWNS.with(|n| n.set(n.get() + 1));
    Ok(())
}

/// Locker spawns the default (test) spawner made on this thread.
#[cfg(test)]
pub fn test_spawn_count() -> u32 {
    TEST_SPAWNS.with(|n| n.get())
}

impl LockGuard {
    /// Decide what to do about a dead locker at `now`, and record a spawn
    /// when the answer is [`LockerDecision::Spawn`].
    pub fn decide(&mut self, trigger: LockerTrigger, now: Instant) -> LockerDecision {
        if let Some(last) = self.last_spawn {
            // `saturating`: a clock reading before the last spawn counts
            // as "just spawned", never as a long time ago.
            let since = now.saturating_duration_since(last);
            if since < SPAWN_GRACE {
                return LockerDecision::Wait;
            }
            if since >= RESPAWN_WINDOW {
                self.auto_spawns = 0;
                self.gave_up = false;
            }
        }
        match trigger {
            LockerTrigger::ForceUnlock => {}
            LockerTrigger::OwnerGone if self.auto_spawns >= MAX_AUTO_RESPAWNS => {
                return if std::mem::replace(&mut self.gave_up, true) {
                    LockerDecision::Idle
                } else {
                    LockerDecision::GiveUp
                };
            }
            LockerTrigger::OwnerGone => self.auto_spawns += 1,
        }
        self.last_spawn = Some(now);
        LockerDecision::Spawn
    }

    /// An authenticated unlock ended the lock: the next lock starts with a
    /// fresh restart budget.
    pub fn reset(&mut self) {
        self.auto_spawns = 0;
        self.last_spawn = None;
        self.gave_up = false;
    }
}

impl MargoState {
    /// Whether the client that owns the current lock is still connected.
    pub(crate) fn lock_owner_alive(&self) -> bool {
        self.session_lock_owner
            .as_ref()
            .is_some_and(|lock| lock.is_alive())
    }

    /// Once per loop iteration: while locked with a dead locker, restart it
    /// (bounded, see [`LockGuard::decide`]). Never unlocks.
    pub fn check_lock_owner(&mut self, now: Instant) {
        if !self.session_locked || self.lock_owner_alive() {
            return;
        }
        self.restart_locker(LockerTrigger::OwnerGone, now);
    }

    /// logind asked to unlock this session (`loginctl unlock-session`, run
    /// from a TTY or ssh — authenticated outside the locked seat). This is
    /// the one unlock that doesn't come from the locker: a live locker gets
    /// `finished` (the protocol's "the compositor unlocked by another
    /// secure way") so it exits instead of lingering.
    pub fn logind_unlock(&mut self) {
        if !self.session_locked {
            tracing::info!("session lock: logind Unlock while unlocked, nothing to do");
            return;
        }
        tracing::warn!(
            locker_alive = self.lock_owner_alive(),
            "session lock: unlocked by logind (loginctl unlock-session)"
        );
        if let Some(lock) = self.session_lock_owner.as_ref().filter(|l| l.is_alive()) {
            lock.finished();
        }
        self.end_session_lock();
    }

    /// Compositor-side end of the lock: drop the lock state and surfaces,
    /// finish maps deferred while locked and show the desktop again.
    pub(crate) fn end_session_lock(&mut self) {
        self.session_locked = false;
        self.mark_state_dirty();
        self.session_lock_owner = None;
        self.lock_surfaces.clear();
        if let Some(token) = self.lock_guard.retry_timer.take() {
            self.loop_handle.remove(token);
        }
        self.lock_guard.reset();
        self.finish_lock_deferred_maps();
        self.arrange_all();
        self.refresh_keyboard_focus();
    }

    pub(crate) fn restart_locker(&mut self, trigger: LockerTrigger, now: Instant) {
        let decision = self.lock_guard.decide(trigger, now);
        match decision {
            LockerDecision::Spawn => {
                tracing::warn!(
                    ?trigger,
                    "session lock: the locker is gone, starting {LOCKER_COMMAND} (session stays locked)"
                );
                if let Err(e) = (self.lock_guard.spawner)() {
                    tracing::error!(error = %e, "session lock: could not start {LOCKER_COMMAND}");
                }
            }
            LockerDecision::GiveUp => tracing::error!(
                "session lock: {LOCKER_COMMAND} did not take the lock after \
                 {MAX_AUTO_RESPAWNS} restarts; the session stays locked (retrying in \
                 {RESPAWN_WINDOW:?}). Press the force_unlock bind to retry now, or unlock \
                 from a TTY with `loginctl unlock-session`."
            ),
            LockerDecision::Wait | LockerDecision::Idle => {}
        }
        let delay = match decision {
            LockerDecision::Spawn => Some(SPAWN_GRACE),
            LockerDecision::GiveUp => Some(RESPAWN_WINDOW),
            LockerDecision::Wait | LockerDecision::Idle => None,
        };
        if let Some(delay) = delay {
            if let Some(token) = self.lock_guard.retry_timer.take() {
                self.loop_handle.remove(token);
            }
            // The loop may be asleep with no outputs/frames/input. Retry
            // from a timer, not just the next incidental event-loop turn.
            match self.loop_handle.insert_source(
                calloop::timer::Timer::from_duration(delay),
                |_, _, state| {
                    state.lock_guard.retry_timer = None;
                    state.check_lock_owner(Instant::now());
                    calloop::timer::TimeoutAction::Drop
                },
            ) {
                Ok(token) => self.lock_guard.retry_timer = Some(token),
                Err(e) => {
                    tracing::error!(error = %e, "session lock: retry timer failed; stays locked")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn a_dead_locker_is_restarted_and_given_time_to_lock() {
        let mut g = LockGuard::default();
        let t0 = Instant::now();
        assert_eq!(
            g.decide(LockerTrigger::OwnerGone, t0),
            LockerDecision::Spawn
        );
        // Polled every loop iteration while the new locker starts up.
        for ms in [0, 10, 1000, 4999] {
            assert_eq!(
                g.decide(LockerTrigger::OwnerGone, t0 + Duration::from_millis(ms)),
                LockerDecision::Wait,
                "{ms} ms after the spawn"
            );
        }
        assert_eq!(
            g.decide(LockerTrigger::OwnerGone, t0 + SPAWN_GRACE),
            LockerDecision::Spawn,
            "still dead after the grace period: try again"
        );
    }

    #[test]
    fn automatic_restarts_are_bounded_then_resume_after_the_window() {
        let mut g = LockGuard::default();
        let t0 = Instant::now();
        let mut t = t0;
        for i in 0..MAX_AUTO_RESPAWNS {
            assert_eq!(
                g.decide(LockerTrigger::OwnerGone, t),
                LockerDecision::Spawn,
                "try {i}"
            );
            t += SPAWN_GRACE;
        }
        assert_eq!(
            g.decide(LockerTrigger::OwnerGone, t),
            LockerDecision::GiveUp
        );
        assert_eq!(
            g.decide(LockerTrigger::OwnerGone, t + S),
            LockerDecision::Idle
        );
        assert_eq!(
            g.decide(LockerTrigger::OwnerGone, t + 30 * S),
            LockerDecision::Idle
        );
        // A minute after the last restart the budget is back.
        let last = t - SPAWN_GRACE;
        assert_eq!(
            g.decide(LockerTrigger::OwnerGone, last + RESPAWN_WINDOW),
            LockerDecision::Spawn
        );
    }

    #[test]
    fn force_unlock_restarts_even_after_giving_up_but_is_debounced() {
        let mut g = LockGuard::default();
        let mut t = Instant::now();
        for _ in 0..MAX_AUTO_RESPAWNS {
            g.decide(LockerTrigger::OwnerGone, t);
            t += SPAWN_GRACE;
        }
        assert_eq!(
            g.decide(LockerTrigger::OwnerGone, t),
            LockerDecision::GiveUp
        );
        assert_eq!(
            g.decide(LockerTrigger::ForceUnlock, t),
            LockerDecision::Spawn
        );
        assert_eq!(
            g.decide(LockerTrigger::ForceUnlock, t + S),
            LockerDecision::Wait,
            "mashing the bind must not start a locker per press"
        );
        assert_eq!(
            g.decide(LockerTrigger::ForceUnlock, t + SPAWN_GRACE),
            LockerDecision::Spawn
        );
    }

    #[test]
    fn manual_restarts_do_not_use_up_the_automatic_budget() {
        let mut g = LockGuard::default();
        let mut t = Instant::now();
        for _ in 0..10 {
            assert_eq!(
                g.decide(LockerTrigger::ForceUnlock, t),
                LockerDecision::Spawn
            );
            t += SPAWN_GRACE;
        }
        assert_eq!(g.decide(LockerTrigger::OwnerGone, t), LockerDecision::Spawn);
    }

    #[test]
    fn an_authenticated_unlock_resets_the_budget() {
        let mut g = LockGuard::default();
        let mut t = Instant::now();
        for _ in 0..MAX_AUTO_RESPAWNS {
            g.decide(LockerTrigger::OwnerGone, t);
            t += SPAWN_GRACE;
        }
        assert_eq!(
            g.decide(LockerTrigger::OwnerGone, t),
            LockerDecision::GiveUp
        );
        g.reset();
        assert_eq!(g.decide(LockerTrigger::OwnerGone, t), LockerDecision::Spawn);
    }

    #[test]
    fn a_clock_reading_before_the_last_spawn_waits() {
        let mut g = LockGuard::default();
        let t0 = Instant::now() + RESPAWN_WINDOW;
        assert_eq!(
            g.decide(LockerTrigger::OwnerGone, t0),
            LockerDecision::Spawn
        );
        assert_eq!(
            g.decide(LockerTrigger::OwnerGone, t0 - S),
            LockerDecision::Wait
        );
    }
}
