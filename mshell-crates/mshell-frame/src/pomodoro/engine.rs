//! The Pomodoro cycle as a pure state machine — no clock, no GTK.
//!
//! Every operation takes `now` so the tests drive time by hand. A running
//! phase stores its *deadline* (not a per-second counter), so the shell
//! needs exactly one timer per phase and nothing ticks while the timer is
//! paused or idle. The cycle is `work → short break → work → … → long
//! break` with `cycles` work sessions per round; finishing (or skipping)
//! the long break starts the next round at session 1.

use std::time::{Duration, Instant};

/// One of the three Pomodoro phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Work,
    ShortBreak,
    LongBreak,
}

impl Phase {
    /// Human label (pill tooltip, menu hero, notification).
    pub fn label(self) -> &'static str {
        match self {
            Phase::Work => "Focus",
            Phase::ShortBreak => "Short break",
            Phase::LongBreak => "Long break",
        }
    }

    /// Lower-case token used by `mshellctl pomodoro status` and CSS.
    pub fn token(self) -> &'static str {
        match self {
            Phase::Work => "work",
            Phase::ShortBreak => "short-break",
            Phase::LongBreak => "long-break",
        }
    }

    pub fn is_break(self) -> bool {
        self != Phase::Work
    }
}

/// Durations and auto-start rules (from `config.pomodoro`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub work: Duration,
    pub short_break: Duration,
    pub long_break: Duration,
    /// Work sessions per round (the long break comes after the last one).
    pub cycles: u32,
    pub auto_start_breaks: bool,
    pub auto_start_work: bool,
}

impl Settings {
    /// Build from minute counts, clamping nonsense to a usable range (a zero
    /// duration would fire forever; zero cycles would never reach work).
    pub fn from_minutes(
        work: u32,
        short_break: u32,
        long_break: u32,
        cycles: u32,
        auto_start_breaks: bool,
        auto_start_work: bool,
    ) -> Self {
        let mins = |m: u32| Duration::from_secs(u64::from(m.clamp(1, 600)) * 60);
        Self {
            work: mins(work),
            short_break: mins(short_break),
            long_break: mins(long_break),
            cycles: cycles.clamp(1, 12),
            auto_start_breaks,
            auto_start_work,
        }
    }

    pub fn duration(&self, phase: Phase) -> Duration {
        match phase {
            Phase::Work => self.work,
            Phase::ShortBreak => self.short_break,
            Phase::LongBreak => self.long_break,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_minutes(25, 5, 15, 4, false, false)
    }
}

/// Coarse run state, for buttons and the pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// The current phase has not started (full time left).
    Ready,
    Running,
    Paused,
}

/// A phase that ran out, reported by [`Pomodoro::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseEnd {
    pub finished: Phase,
    pub next: Phase,
    /// The next phase started on its own (auto-start rules).
    pub auto_started: bool,
}

#[derive(Debug, Clone)]
pub struct Pomodoro {
    settings: Settings,
    phase: Phase,
    /// 1-based work session within the round (`1..=cycles`).
    session: u32,
    /// Deadline of the running phase; `None` while ready / paused.
    deadline: Option<Instant>,
    /// Time left while not running.
    remaining: Duration,
    /// Work phases finished since the last full reset.
    completed: u32,
}

impl Pomodoro {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            phase: Phase::Work,
            session: 1,
            deadline: None,
            remaining: settings.work,
            completed: 0,
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn session(&self) -> u32 {
        self.session
    }

    pub fn completed(&self) -> u32 {
        self.completed
    }

    pub fn is_running(&self) -> bool {
        self.deadline.is_some()
    }

    /// Deadline of the running phase (`None` while ready or paused).
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub fn run_state(&self) -> RunState {
        if self.deadline.is_some() {
            RunState::Running
        } else if self.remaining == self.settings.duration(self.phase) {
            RunState::Ready
        } else {
            RunState::Paused
        }
    }

    /// Time left in the current phase.
    pub fn remaining(&self, now: Instant) -> Duration {
        match self.deadline {
            Some(d) => d.saturating_duration_since(now),
            None => self.remaining,
        }
    }

    /// Fraction of the current phase already elapsed, `0.0..=1.0`.
    pub fn progress(&self, now: Instant) -> f64 {
        let total = self.settings.duration(self.phase).as_secs_f64();
        if total <= 0.0 {
            return 0.0;
        }
        (1.0 - self.remaining(now).as_secs_f64() / total).clamp(0.0, 1.0)
    }

    pub fn start(&mut self, now: Instant) {
        if self.deadline.is_none() {
            self.deadline = Some(now + self.remaining);
        }
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(d) = self.deadline.take() {
            self.remaining = d.saturating_duration_since(now);
        }
    }

    pub fn toggle(&mut self, now: Instant) {
        if self.is_running() {
            self.pause(now);
        } else {
            self.start(now);
        }
    }

    /// Stop and move to the next phase without counting the current one.
    pub fn skip(&mut self) {
        self.deadline = None;
        self.advance();
    }

    /// Stop and refill the current phase.
    pub fn reset(&mut self) {
        self.deadline = None;
        self.remaining = self.settings.duration(self.phase);
    }

    /// Stop and go back to the first work session of a fresh round.
    pub fn reset_all(&mut self) {
        *self = Self::new(self.settings);
    }

    /// Finish the running phase if its deadline passed. Called by the
    /// shell's one-shot timer; a timer that wakes a little early returns
    /// `None` and the caller re-arms for [`Self::remaining`].
    pub fn tick(&mut self, now: Instant) -> Option<PhaseEnd> {
        let deadline = self.deadline?;
        if now < deadline {
            return None;
        }
        let finished = self.phase;
        if finished == Phase::Work {
            self.completed += 1;
        }
        self.deadline = None;
        self.advance();
        let auto = if self.phase.is_break() {
            self.settings.auto_start_breaks
        } else {
            self.settings.auto_start_work
        };
        if auto {
            self.start(now);
        }
        Some(PhaseEnd {
            finished,
            next: self.phase,
            auto_started: auto,
        })
    }

    /// Adopt new settings. A phase that hasn't started picks up its new
    /// duration; a running or paused one keeps its time (changing the
    /// length under a running timer would jump the readout).
    pub fn set_settings(&mut self, settings: Settings) {
        let fresh = self.run_state() == RunState::Ready;
        self.settings = settings;
        self.session = self.session.min(settings.cycles);
        if self.phase == Phase::LongBreak && self.session < settings.cycles {
            // Fewer cycles can't leave us mid-round in a long break.
            self.session = settings.cycles;
        }
        if fresh {
            self.remaining = settings.duration(self.phase);
        }
    }

    fn advance(&mut self) {
        match self.phase {
            Phase::Work if self.session >= self.settings.cycles => self.phase = Phase::LongBreak,
            Phase::Work => self.phase = Phase::ShortBreak,
            Phase::ShortBreak => {
                self.session += 1;
                self.phase = Phase::Work;
            }
            Phase::LongBreak => {
                self.session = 1;
                self.phase = Phase::Work;
            }
        }
        self.remaining = self.settings.duration(self.phase);
    }
}

/// `MM:SS` below an hour, `H:MM:SS` above.
pub fn format_remaining(d: Duration) -> String {
    // Round up so the readout shows 25:00 at the start and 00:01 in the
    // last second, never 00:00 while time is still left.
    let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quick() -> Settings {
        Settings::from_minutes(25, 5, 15, 2, false, false)
    }

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn a_fresh_timer_is_ready_with_the_full_work_phase() {
        let p = Pomodoro::new(quick());
        let now = Instant::now();
        assert_eq!(p.phase(), Phase::Work);
        assert_eq!(p.session(), 1);
        assert_eq!(p.run_state(), RunState::Ready);
        assert_eq!(p.remaining(now), 25 * MIN);
        assert_eq!(p.progress(now), 0.0);
    }

    #[test]
    fn running_counts_down_from_a_deadline_and_pause_freezes_it() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(quick());
        p.start(t0);
        assert_eq!(p.run_state(), RunState::Running);
        assert_eq!(p.remaining(t0 + 10 * MIN), 15 * MIN);
        p.pause(t0 + 10 * MIN);
        assert_eq!(p.run_state(), RunState::Paused);
        // Paused time does not pass.
        assert_eq!(p.remaining(t0 + 60 * MIN), 15 * MIN);
        p.toggle(t0 + 60 * MIN);
        assert_eq!(p.remaining(t0 + 65 * MIN), 10 * MIN);
        // start is a no-op while running (doesn't push the deadline out).
        p.start(t0 + 66 * MIN);
        assert_eq!(p.remaining(t0 + 66 * MIN), 9 * MIN);
    }

    #[test]
    fn a_full_round_goes_work_short_work_long_and_wraps() {
        let mut p = Pomodoro::new(quick());
        let mut now = Instant::now();
        let mut seen = vec![(p.phase(), p.session())];
        for _ in 0..4 {
            p.start(now);
            now += p.remaining(now);
            let end = p.tick(now).expect("phase ended");
            assert!(!end.auto_started);
            assert!(!p.is_running(), "no auto-start by default");
            seen.push((p.phase(), p.session()));
        }
        assert_eq!(
            seen,
            vec![
                (Phase::Work, 1),
                (Phase::ShortBreak, 1),
                (Phase::Work, 2),
                (Phase::LongBreak, 2),
                (Phase::Work, 1),
            ]
        );
        assert_eq!(p.completed(), 2, "two work phases finished");
    }

    #[test]
    fn tick_before_the_deadline_does_nothing() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(quick());
        assert_eq!(p.tick(t0), None, "not running");
        p.start(t0);
        assert_eq!(p.tick(t0 + 24 * MIN), None);
        assert!(p.is_running());
        assert!(p.tick(t0 + 25 * MIN).is_some());
    }

    #[test]
    fn auto_start_follows_the_breaks_and_work_switches_separately() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(Settings::from_minutes(25, 5, 15, 4, true, false));
        p.start(t0);
        let end = p.tick(t0 + 25 * MIN).unwrap();
        assert_eq!(end.finished, Phase::Work);
        assert_eq!(end.next, Phase::ShortBreak);
        assert!(end.auto_started);
        assert_eq!(
            p.remaining(t0 + 26 * MIN),
            4 * MIN,
            "break runs from the end"
        );
        let end = p.tick(t0 + 30 * MIN).unwrap();
        assert_eq!(end.next, Phase::Work);
        assert!(!end.auto_started, "auto_start_work is off");
        assert_eq!(p.run_state(), RunState::Ready);
    }

    #[test]
    fn skip_moves_on_without_counting_and_reset_refills() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(quick());
        p.start(t0);
        p.skip();
        assert_eq!(p.phase(), Phase::ShortBreak);
        assert!(!p.is_running());
        assert_eq!(p.completed(), 0, "a skipped work phase is not a pomodoro");
        p.start(t0);
        p.pause(t0 + 2 * MIN);
        p.reset();
        assert_eq!(p.phase(), Phase::ShortBreak);
        assert_eq!(p.run_state(), RunState::Ready);
        assert_eq!(p.remaining(t0), 5 * MIN);
        p.skip();
        p.skip();
        assert_eq!((p.phase(), p.session()), (Phase::LongBreak, 2));
        p.reset_all();
        assert_eq!((p.phase(), p.session()), (Phase::Work, 1));
    }

    #[test]
    fn new_settings_reach_a_ready_phase_but_not_a_running_one() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(quick());
        p.set_settings(Settings::from_minutes(50, 10, 30, 2, false, false));
        assert_eq!(
            p.remaining(t0),
            50 * MIN,
            "ready phase takes the new length"
        );
        p.start(t0);
        p.set_settings(Settings::from_minutes(20, 10, 30, 2, false, false));
        assert_eq!(
            p.remaining(t0 + MIN),
            49 * MIN,
            "running phase keeps its deadline"
        );
    }

    #[test]
    fn fewer_cycles_clamp_the_session() {
        let mut p = Pomodoro::new(Settings::from_minutes(25, 5, 15, 4, false, false));
        p.skip();
        p.skip();
        p.skip();
        p.skip();
        assert_eq!(p.session(), 3);
        p.set_settings(Settings::from_minutes(25, 5, 15, 2, false, false));
        assert_eq!(p.session(), 2);
        assert_eq!(p.phase(), Phase::Work);
        p.skip();
        assert_eq!(p.phase(), Phase::LongBreak, "session 2 of 2 → long break");
    }

    #[test]
    fn settings_clamp_zero_values() {
        let s = Settings::from_minutes(0, 0, 0, 0, false, false);
        assert_eq!(s.work, MIN);
        assert_eq!(s.cycles, 1);
        // One cycle: every work phase is followed by the long break.
        let mut p = Pomodoro::new(s);
        p.skip();
        assert_eq!(p.phase(), Phase::LongBreak);
    }

    #[test]
    fn remaining_formats_round_up_to_the_second() {
        assert_eq!(format_remaining(25 * MIN), "25:00");
        assert_eq!(format_remaining(Duration::from_millis(400)), "00:01");
        assert_eq!(format_remaining(Duration::ZERO), "00:00");
        assert_eq!(
            format_remaining(Duration::from_secs(3 * 3600 + 61)),
            "3:01:01"
        );
    }
}
