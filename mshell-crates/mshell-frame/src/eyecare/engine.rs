//! The 20-20-20 eye-care cycle as a pure state machine — no clock, no GTK.
//!
//! Every 20 minutes of *active* screen time, look at something 20 feet
//! away for 20 seconds. Active time only accumulates while the user is at
//! the keyboard: the shell reports "away" after `away` of no input (the
//! `ext-idle-notify` timeout, which honours idle inhibitors — a playing
//! video keeps counting as eyes-on-screen). Being away at least that long
//! is a natural break, so it resets the active count. Like the Pomodoro
//! engine, a counting phase stores its *start*, so the shell arms one
//! timer for the next deadline and nothing ticks in between.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Active screen time between breaks.
    pub active: Duration,
    /// Length of a break.
    pub break_len: Duration,
    /// No input for this long = the user is away (a natural break).
    pub away: Duration,
}

impl Settings {
    /// From config units, clamped to a usable range. `away` is never
    /// shorter than a break: being away counts as having taken one.
    pub fn new(active_minutes: u32, break_seconds: u32, away_seconds: u32) -> Self {
        let break_len = Duration::from_secs(u64::from(break_seconds.clamp(5, 600)));
        let away = Duration::from_secs(u64::from(away_seconds.clamp(30, 3600))).max(break_len);
        Self {
            active: Duration::from_secs(u64::from(active_minutes.clamp(1, 240)) * 60),
            break_len,
            away,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::new(20, 20, 120)
    }
}

/// What the pill shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Counting active time; `left` until the next break.
    Active { left: Duration },
    /// The user is away; the count is held (and reset).
    Away,
    /// A break is running; `left` until it ends.
    Break { left: Duration },
}

/// A transition the shell announces (notification + sound).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    BreakStarted,
    BreakFinished,
}

#[derive(Debug, Clone)]
pub struct EyeCare {
    settings: Settings,
    /// Active time counted before `counting_since`.
    banked: Duration,
    /// Start of the current counting stretch (`None` while away / on break).
    counting_since: Option<Instant>,
    /// End of the running break.
    break_until: Option<Instant>,
    away: bool,
}

impl EyeCare {
    pub fn new(settings: Settings, now: Instant) -> Self {
        Self {
            settings,
            banked: Duration::ZERO,
            counting_since: Some(now),
            break_until: None,
            away: false,
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    pub fn is_away(&self) -> bool {
        self.away
    }

    /// Active time counted towards the next break.
    pub fn active_elapsed(&self, now: Instant) -> Duration {
        self.banked
            + self
                .counting_since
                .map(|s| now.saturating_duration_since(s))
                .unwrap_or_default()
    }

    pub fn mode(&self, now: Instant) -> Mode {
        if let Some(until) = self.break_until {
            Mode::Break {
                left: until.saturating_duration_since(now),
            }
        } else if self.away {
            Mode::Away
        } else {
            Mode::Active {
                left: self
                    .settings
                    .active
                    .saturating_sub(self.active_elapsed(now)),
            }
        }
    }

    /// When [`Self::tick`] next has something to do (`None` while away).
    pub fn next_deadline(&self) -> Option<Instant> {
        if let Some(until) = self.break_until {
            return Some(until);
        }
        let since = self.counting_since?;
        Some(since + self.settings.active.saturating_sub(self.banked))
    }

    /// The user went away (`true`) or came back (`false`).
    pub fn set_away(&mut self, away: bool, now: Instant) {
        if away == self.away {
            return;
        }
        self.away = away;
        if self.break_until.is_some() {
            return; // a break runs its course either way
        }
        if away {
            // No input for `settings.away` ≥ a break: that was a rest.
            self.counting_since = None;
            self.banked = Duration::ZERO;
        } else {
            self.counting_since = Some(now);
        }
    }

    /// Advance to `now`, returning a transition if one is due.
    pub fn tick(&mut self, now: Instant) -> Option<Event> {
        if let Some(until) = self.break_until {
            if now < until {
                return None;
            }
            self.restart_count(now);
            return Some(Event::BreakFinished);
        }
        if self.counting_since.is_some() && self.active_elapsed(now) >= self.settings.active {
            self.begin_break(now);
            return Some(Event::BreakStarted);
        }
        None
    }

    /// Take a break now (the pill's click, `mshellctl eyecare break`).
    /// Returns `false` if one is already running.
    pub fn start_break(&mut self, now: Instant) -> bool {
        if self.break_until.is_some() {
            return false;
        }
        self.begin_break(now);
        true
    }

    /// End a running break early without the "break over" announcement.
    /// Returns `false` if none was running.
    pub fn skip_break(&mut self, now: Instant) -> bool {
        if self.break_until.is_none() {
            return false;
        }
        self.restart_count(now);
        true
    }

    /// Cancel any break and start counting from zero.
    pub fn reset(&mut self, now: Instant) {
        self.restart_count(now);
    }

    /// Adopt new settings, keeping the time already counted.
    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
    }

    fn begin_break(&mut self, now: Instant) {
        self.counting_since = None;
        self.banked = Duration::ZERO;
        self.break_until = Some(now + self.settings.break_len);
    }

    fn restart_count(&mut self, now: Instant) {
        self.break_until = None;
        self.banked = Duration::ZERO;
        self.counting_since = (!self.away).then_some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);
    const SEC: Duration = Duration::from_secs(1);

    fn twenty() -> Settings {
        Settings::new(20, 20, 120)
    }

    #[test]
    fn twenty_active_minutes_start_a_twenty_second_break() {
        let t0 = Instant::now();
        let mut e = EyeCare::new(twenty(), t0);
        assert_eq!(e.mode(t0), Mode::Active { left: 20 * MIN });
        assert_eq!(e.next_deadline(), Some(t0 + 20 * MIN));
        assert_eq!(e.tick(t0 + 19 * MIN), None);
        assert_eq!(e.tick(t0 + 20 * MIN), Some(Event::BreakStarted));
        assert_eq!(
            e.mode(t0 + 20 * MIN + 5 * SEC),
            Mode::Break { left: 15 * SEC }
        );
        assert_eq!(e.next_deadline(), Some(t0 + 20 * MIN + 20 * SEC));
        assert_eq!(e.tick(t0 + 20 * MIN + 20 * SEC), Some(Event::BreakFinished));
        // The next stretch counts from the end of the break.
        assert_eq!(
            e.mode(t0 + 21 * MIN),
            Mode::Active {
                left: 19 * MIN + 20 * SEC
            }
        );
    }

    #[test]
    fn being_away_is_a_natural_break_and_nothing_counts_meanwhile() {
        let t0 = Instant::now();
        let mut e = EyeCare::new(twenty(), t0);
        e.set_away(true, t0 + 15 * MIN);
        assert_eq!(e.mode(t0 + 40 * MIN), Mode::Away);
        assert_eq!(e.next_deadline(), None, "nothing to wake for while away");
        assert_eq!(e.tick(t0 + 40 * MIN), None);
        e.set_away(false, t0 + 40 * MIN);
        assert_eq!(
            e.mode(t0 + 40 * MIN),
            Mode::Active { left: 20 * MIN },
            "the away stretch reset the count"
        );
        assert_eq!(e.next_deadline(), Some(t0 + 60 * MIN));
    }

    #[test]
    fn a_break_runs_its_course_even_if_the_user_leaves() {
        let t0 = Instant::now();
        let mut e = EyeCare::new(twenty(), t0);
        assert!(e.start_break(t0));
        assert!(!e.start_break(t0), "already on a break");
        e.set_away(true, t0 + 5 * SEC);
        assert_eq!(e.tick(t0 + 20 * SEC), Some(Event::BreakFinished));
        assert_eq!(e.mode(t0 + 30 * SEC), Mode::Away, "still away: count held");
        e.set_away(false, t0 + MIN);
        assert_eq!(e.mode(t0 + MIN), Mode::Active { left: 20 * MIN });
    }

    #[test]
    fn skip_and_reset_restart_the_count() {
        let t0 = Instant::now();
        let mut e = EyeCare::new(twenty(), t0);
        assert!(!e.skip_break(t0), "no break to skip");
        e.start_break(t0 + MIN);
        assert!(e.skip_break(t0 + MIN + 3 * SEC));
        assert_eq!(
            e.tick(t0 + MIN + 30 * SEC),
            None,
            "no BreakFinished after a skip"
        );
        assert_eq!(
            e.mode(t0 + 2 * MIN),
            Mode::Active {
                left: 20 * MIN - 57 * SEC
            }
        );
        e.reset(t0 + 10 * MIN);
        assert_eq!(e.mode(t0 + 10 * MIN), Mode::Active { left: 20 * MIN });
    }

    #[test]
    fn repeated_presence_reports_are_ignored() {
        let t0 = Instant::now();
        let mut e = EyeCare::new(twenty(), t0);
        e.set_away(false, t0 + 5 * MIN); // already present: must not restart
        assert_eq!(e.mode(t0 + 5 * MIN), Mode::Active { left: 15 * MIN });
    }

    #[test]
    fn new_settings_keep_the_counted_time() {
        let t0 = Instant::now();
        let mut e = EyeCare::new(twenty(), t0);
        e.set_settings(Settings::new(30, 20, 120));
        assert_eq!(e.mode(t0 + 10 * MIN), Mode::Active { left: 20 * MIN });
        e.set_settings(Settings::new(5, 20, 120));
        assert_eq!(
            e.tick(t0 + 10 * MIN),
            Some(Event::BreakStarted),
            "already over"
        );
    }

    #[test]
    fn settings_clamp_and_away_is_never_shorter_than_a_break() {
        let s = Settings::new(0, 300, 30);
        assert_eq!(s.active, MIN);
        assert_eq!(s.break_len, 300 * SEC);
        assert_eq!(s.away, 300 * SEC);
        assert_eq!(Settings::default().away, 120 * SEC);
    }
}
