//! Per-output frame clock — opt-in (`per_output_frame_clock` config knob).
//!
//! ## Why
//!
//! The default render path drives EVERY output from a single global
//! tick: anything that dirties the scene calls
//! [`crate::state::MargoState::request_repaint`], which sets one dirty
//! flag and pings one calloop source; that source renders all outputs
//! together, and a single `pending_vblanks` counter gates the next
//! repaint. On a mixed-refresh multi-monitor setup (say a 144 Hz panel
//! next to a 60 Hz one) this paces everything off whichever vblank the
//! loop happens to service — the fast monitor can't run ahead of the
//! slow one.
//!
//! When `per_output_frame_clock = true`, each output instead carries
//! its own [`OutputClock`]: a present `Timer` re-armed off that
//! output's last vblank at `last_present + refresh_interval`, a dirty
//! flag, and an in-flight gate. The render loop renders only the
//! output(s) whose timer fired and that are dirty. Animations are
//! sampled when an output ticks (they key off absolute time, so a
//! per-output cadence samples each running animation at the correct
//! progress — a faster output just samples more often).
//!
//! ## Safety
//!
//! This module is inert unless the flag is on: with it off,
//! `MargoState::per_output_clocks` stays empty and every call site
//! branches back to the byte-for-byte original global-tick path. The
//! pure scheduling logic here (next-tick time, is-due) is unit-tested
//! without touching DRM, since the live present path can only be
//! verified on hardware.

use std::time::{Duration, Instant};

use smithay::reexports::calloop::RegistrationToken;

/// Per-output frame-clock bookkeeping. One per active output, held in
/// `MargoState::per_output_clocks` keyed by `Output::name()`. Only
/// alive when `per_output_frame_clock` is enabled.
#[derive(Debug, Default)]
pub struct OutputClock {
    /// Scene needs a render on this output. Set by `request_repaint`
    /// (which marks every clock dirty, since a global repaint can
    /// affect any output) and cleared once the output renders.
    pub dirty: bool,
    /// A `queue_frame` for this output is awaiting its vblank. While
    /// true the present timer must not render again (the DRM
    /// compositor accepts one pending page-flip per output).
    pub pending_vblank: bool,
    /// Monotonic instant of this output's last vblank (page-flip
    /// landing). The next present timer is armed at
    /// `last_present + refresh_interval`. `None` before the first
    /// vblank — the first tick fires immediately.
    pub last_present: Option<Instant>,
    /// In-flight present `Timer` token, so we can cancel/replace it on
    /// re-arm. `None` when no timer is currently scheduled.
    pub timer_token: Option<RegistrationToken>,
}

impl OutputClock {
    /// Delay from `now` until this output's next present, given its
    /// `refresh_interval` and `last_present`. Returns `Duration::ZERO`
    /// when the output is already due (or has never presented), so the
    /// timer fires on the next loop turn.
    ///
    /// Pure: no clock reads, no side effects — `now` is passed in so
    /// the computation is unit-testable.
    pub fn next_tick_delay(&self, now: Instant, refresh_interval: Duration) -> Duration {
        match self.last_present {
            // Never presented → render as soon as possible.
            None => Duration::ZERO,
            Some(last) => {
                let target = last + refresh_interval;
                // saturating: if we're already past the target, due now.
                target.saturating_duration_since(now)
            }
        }
    }

    /// True when this output is ready to present at `now`: not waiting
    /// on a vblank, dirty, and its refresh interval has elapsed since
    /// the last present (or it has never presented).
    ///
    /// Pure — `now` is injected for testability.
    pub fn is_due(&self, now: Instant, refresh_interval: Duration) -> bool {
        if self.pending_vblank || !self.dirty {
            return false;
        }
        match self.last_present {
            None => true,
            Some(last) => now.saturating_duration_since(last) >= refresh_interval,
        }
    }
}

/// Frame interval for a refresh rate expressed in millihertz (the unit
/// smithay's `Mode::refresh` uses). Falls back to 60 Hz when the rate
/// is zero or negative (some virtual outputs report `refresh == 0`).
///
/// Pure helper shared by the scheduler and its tests so the 60 Hz
/// fallback is exercised directly.
pub fn refresh_interval_from_mhz(refresh_mhz: i32) -> Duration {
    if refresh_mhz > 0 {
        // mHz → seconds-per-frame.
        Duration::from_secs_f64(1000.0 / refresh_mhz as f64)
    } else {
        Duration::from_secs_f64(1.0 / 60.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ_60: i32 = 60_000; // 60 Hz in mHz
    const HZ_144: i32 = 144_000; // 144 Hz in mHz

    #[test]
    fn refresh_interval_60hz() {
        let d = refresh_interval_from_mhz(HZ_60);
        // 1/60 s ≈ 16.667 ms
        assert!((d.as_secs_f64() - 1.0 / 60.0).abs() < 1e-9);
    }

    #[test]
    fn refresh_interval_144hz() {
        let d = refresh_interval_from_mhz(HZ_144);
        assert!((d.as_secs_f64() - 1.0 / 144.0).abs() < 1e-9);
    }

    #[test]
    fn refresh_interval_zero_falls_back_to_60hz() {
        assert_eq!(
            refresh_interval_from_mhz(0),
            Duration::from_secs_f64(1.0 / 60.0)
        );
        assert_eq!(
            refresh_interval_from_mhz(-1),
            Duration::from_secs_f64(1.0 / 60.0)
        );
    }

    #[test]
    fn never_presented_is_due_immediately() {
        let mut c = OutputClock {
            dirty: true,
            ..Default::default()
        };
        let now = Instant::now();
        let interval = refresh_interval_from_mhz(HZ_60);
        assert_eq!(c.next_tick_delay(now, interval), Duration::ZERO);
        assert!(c.is_due(now, interval));

        // Not dirty → never due, even with no prior present.
        c.dirty = false;
        assert!(!c.is_due(now, interval));
    }

    #[test]
    fn pending_vblank_blocks_due() {
        let c = OutputClock {
            dirty: true,
            pending_vblank: true,
            ..Default::default()
        };
        let now = Instant::now();
        assert!(!c.is_due(now, refresh_interval_from_mhz(HZ_60)));
    }

    #[test]
    fn delay_counts_down_from_last_present() {
        let interval = refresh_interval_from_mhz(HZ_60);
        let last = Instant::now();
        let c = OutputClock {
            dirty: true,
            last_present: Some(last),
            ..Default::default()
        };

        // Half an interval after last present → ~half an interval to go.
        let half = interval / 2;
        let now = last + half;
        let delay = c.next_tick_delay(now, interval);
        // Allow a tiny epsilon for Duration arithmetic.
        let expected = interval - half;
        let diff = delay.abs_diff(expected);
        assert!(diff < Duration::from_micros(10), "delay drift too large");
        assert!(!c.is_due(now, interval), "not due before a full interval");

        // A full interval later → due, zero delay.
        let now = last + interval;
        assert_eq!(c.next_tick_delay(now, interval), Duration::ZERO);
        assert!(c.is_due(now, interval));
    }

    #[test]
    fn two_outputs_tick_independently() {
        // Synthetic 60 Hz + 144 Hz outputs sharing a start instant.
        // Over a fixed window, count how many present ticks each would
        // fire — the 144 Hz output must tick more often. This models
        // the scheduler: after each tick we advance last_present by one
        // interval and re-check is_due.
        fn count_ticks(refresh_mhz: i32, window: Duration) -> u32 {
            let interval = refresh_interval_from_mhz(refresh_mhz);
            let start = Instant::now();
            let mut clock = OutputClock {
                dirty: true,
                last_present: None,
                ..Default::default()
            };
            let mut ticks = 0u32;
            // March a virtual clock across the window in interval steps.
            // First tick is immediate (last_present None).
            let mut t = start;
            let end = start + window;
            // Bound the loop so a logic bug can't spin forever.
            for _ in 0..100_000 {
                if t > end {
                    break;
                }
                if clock.is_due(t, interval) {
                    ticks += 1;
                    clock.last_present = Some(t);
                    clock.dirty = true; // stays dirty (continuous animation)
                }
                t += interval;
            }
            ticks
        }

        let window = Duration::from_secs(1);
        let ticks_60 = count_ticks(HZ_60, window);
        let ticks_144 = count_ticks(HZ_144, window);

        // ~60 and ~144 in a one-second window (off-by-one from the
        // immediate first tick / window boundary is fine).
        assert!(
            (59..=61).contains(&ticks_60),
            "60 Hz ticked {ticks_60} times"
        );
        assert!(
            (143..=145).contains(&ticks_144),
            "144 Hz ticked {ticks_144} times"
        );
        assert!(
            ticks_144 > ticks_60,
            "144 Hz ({ticks_144}) must tick more than 60 Hz ({ticks_60})"
        );
    }
}
