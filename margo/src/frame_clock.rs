//! Per-output frame clock — on by default (`per_output_frame_clock` knob;
//! set it to `false` to fall back to the global tick described below).
//!
//! ## Why
//!
//! The global-tick render path drives EVERY output from a single
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
//! output's last vblank, a dirty flag, and an in-flight gate. A dirty
//! output renders as soon as its previous flip has landed (the flip
//! then lands on the next vblank); only after an empty render, which
//! has no flip to pace it, does the timer wait `refresh_interval`. The render loop renders only the
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
    /// Monotonic instant of this output's last present: a real vblank
    /// (page-flip landing) or an empty render standing in for one. `None`
    /// before the first present — the first tick fires immediately.
    pub last_present: Option<Instant>,
    /// Whether `last_present` was a real page-flip. After a real vblank
    /// the DRM gate (`pending_vblank`) already paces us to one flip per
    /// vblank, so a dirty output renders at once and its flip lands on
    /// the very next vblank. After an empty render nothing paces us, so
    /// the next tick waits a full `refresh_interval` instead of spinning.
    pub last_present_was_vblank: bool,
    /// In-flight present `Timer` token, so we can cancel/replace it on
    /// re-arm. `None` when no timer is currently scheduled.
    pub timer_token: Option<RegistrationToken>,
    /// Monotonic ownership id for `timer_token`. Connector names and even the
    /// same `Output` object can be reused across soft disable/enable; a stale
    /// one-shot callback may mutate the clock only when this id still matches.
    pub timer_id: u64,
}

impl OutputClock {
    /// A real page-flip landed at `at`: drop the in-flight gate. A dirty
    /// output is due immediately afterwards (see `last_present_was_vblank`).
    pub fn note_vblank(&mut self, at: Instant) {
        self.last_present = Some(at);
        self.last_present_was_vblank = true;
        self.pending_vblank = false;
    }

    /// An empty render (no damage, no page-flip) stood in for a present
    /// at `at`. The next tick is held back by one refresh interval.
    pub fn note_estimated_present(&mut self, at: Instant) {
        self.last_present = Some(at);
        self.last_present_was_vblank = false;
        self.pending_vblank = false;
    }

    /// Earliest instant the next present may start, or `None` when it
    /// may start right away.
    fn earliest_next_present(&self, refresh_interval: Duration) -> Option<Instant> {
        match self.last_present {
            Some(last) if !self.last_present_was_vblank => Some(last + refresh_interval),
            _ => None,
        }
    }

    /// Delay from `now` until this output's next present. Zero after a
    /// real vblank (or before the first present); after an empty render,
    /// the time left until `last_present + refresh_interval`. A zero delay
    /// makes the timer fire on the next loop turn.
    ///
    /// Pure: no clock reads, no side effects — `now` is passed in so
    /// the computation is unit-testable.
    pub fn next_tick_delay(&self, now: Instant, refresh_interval: Duration) -> Duration {
        match self.earliest_next_present(refresh_interval) {
            None => Duration::ZERO,
            // saturating: if we're already past the target, due now.
            Some(target) => target.saturating_duration_since(now),
        }
    }

    /// True when this output is ready to present at `now`: not waiting
    /// on a vblank, dirty, and — only after an empty render — a refresh
    /// interval has elapsed since that pseudo-present.
    ///
    /// Pure — `now` is injected for testability.
    pub fn is_due(&self, now: Instant, refresh_interval: Duration) -> bool {
        if self.pending_vblank || !self.dirty {
            return false;
        }
        self.earliest_next_present(refresh_interval)
            .is_none_or(|target| now >= target)
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

/// Map a `CLOCK_MONOTONIC` timestamp (a DRM page-flip event's time) onto
/// `Instant`, given a simultaneous reading of both clocks. `Instant` is
/// `CLOCK_MONOTONIC` on Linux but has no public constructor from a raw
/// timestamp, so step back from `now` by the event's age. A timestamp
/// from the future (clock skew) maps to `now`.
pub fn instant_at_monotonic(event: Duration, now_monotonic: Duration, now: Instant) -> Instant {
    now.checked_sub(now_monotonic.saturating_sub(event))
        .unwrap_or(now)
}

/// Presents older than this don't predict anything: the output has been
/// idle (static screen, DPMS off, disabled) long enough that its last
/// present says nothing about when the next frame reaches the screen.
pub const PRESENT_PREDICTION_MAX_AGE: Duration = Duration::from_secs(1);

/// How far ahead of `now` the next frame rendered now will most likely be
/// presented, given each output's last present and refresh interval.
///
/// A frame rendered now lands on the first vblank after `now`; vblanks
/// keep their phase, so that is `last + k * interval` for the smallest
/// `k >= 1` past `now`. The earliest such time over every output with a
/// recent present wins (it is the first frame that can show the sample).
/// No recent present → `Duration::ZERO` (sample at `now`, as before).
/// The lead is in `(0, interval]` for the chosen output.
///
/// Pure: clocks are passed in so it is unit-testable.
pub fn predicted_present_lead(
    now: Instant,
    presents: impl IntoIterator<Item = (Instant, Duration)>,
) -> Duration {
    presents
        .into_iter()
        .filter_map(|(last, interval)| {
            let age = now.checked_duration_since(last).unwrap_or(Duration::ZERO);
            if age > PRESENT_PREDICTION_MAX_AGE {
                return None;
            }
            let interval_ns = interval.as_nanos().max(1);
            let age_ns = age.as_nanos();
            let frames = age_ns / interval_ns + 1;
            let lead_ns = frames * interval_ns - age_ns;
            Some(Duration::from_nanos(
                u64::try_from(lead_ns).unwrap_or(u64::MAX),
            ))
        })
        .min()
        .unwrap_or(Duration::ZERO)
}

/// Next `wp_presentation_feedback` sequence number for an output. Uses the
/// kernel's vblank counter from the page-flip event when it moves forward,
/// so clients can count missed vblanks; otherwise (no event metadata, or the
/// 32-bit kernel counter wrapped) just steps by one so the published value
/// stays monotonic as the protocol requires.
pub fn next_vblank_seq(prev: u64, kernel: Option<u32>) -> u64 {
    match kernel.map(u64::from) {
        Some(seq) if seq > prev => seq,
        _ => prev.wrapping_add(1),
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

    /// Drive one output through `vblanks` real 60 Hz vblanks with the
    /// scene dirty the whole time, the way the live scheduler does: the
    /// vblank handler runs `handler_latency` after the flip, the present
    /// timer is armed for `next_tick_delay`, a due output submits a frame
    /// that takes `render_time` to reach the kernel, and the flip lands on
    /// the first vblank after that. Returns how many vblanks carried a
    /// new frame.
    fn presents_under_continuous_damage(
        vblanks: u32,
        handler_latency: Duration,
        render_time: Duration,
    ) -> u32 {
        let interval = refresh_interval_from_mhz(HZ_60);
        let start = Instant::now();
        let vblank_at = |k: u32| start + interval * k;
        let mut clock = OutputClock {
            dirty: true,
            ..Default::default()
        };
        // First frame: never presented → due right away, lands on vblank 1.
        assert!(clock.is_due(start, interval));
        clock.dirty = false;
        clock.pending_vblank = true;
        let mut landing = 1;
        let mut presents = 0;
        for k in 1..=vblanks {
            if k != landing {
                continue;
            }
            presents += 1;
            clock.note_vblank(vblank_at(k));
            // Animation still running: the frame callback / tick re-dirties.
            clock.dirty = true;
            let handled = vblank_at(k) + handler_latency;
            let fire = handled + clock.next_tick_delay(handled, interval);
            assert!(clock.is_due(fire, interval), "timer fired but not due");
            clock.dirty = false;
            clock.pending_vblank = true;
            // Flip lands on the first vblank strictly after the submit.
            let submitted = fire + render_time;
            let mut next = k + 1;
            while vblank_at(next) <= submitted {
                next += 1;
            }
            landing = next;
        }
        presents
    }

    #[test]
    fn continuous_damage_presents_on_every_vblank() {
        // Regression: the timer used to wait a full interval after the
        // vblank before rendering, so the frame always missed the next
        // vblank and a 60 Hz panel animated at 30 Hz.
        let presents = presents_under_continuous_damage(
            120,
            Duration::from_micros(200),
            Duration::from_millis(3),
        );
        assert_eq!(presents, 120, "one present per vblank");
    }

    #[test]
    fn real_vblank_makes_a_dirty_output_due_immediately() {
        let interval = refresh_interval_from_mhz(HZ_60);
        let t = Instant::now();
        let mut c = OutputClock {
            pending_vblank: true,
            ..Default::default()
        };
        c.note_vblank(t);
        c.dirty = true;
        assert!(!c.pending_vblank);
        assert_eq!(c.next_tick_delay(t, interval), Duration::ZERO);
        assert!(c.is_due(t, interval));
    }

    #[test]
    fn empty_render_keeps_the_interval_gate() {
        // No page-flip happened, so nothing paces us: rendering again at
        // once would spin. The next tick waits one refresh interval.
        let interval = refresh_interval_from_mhz(HZ_60);
        let t = Instant::now();
        let mut c = OutputClock {
            pending_vblank: true,
            ..Default::default()
        };
        c.note_estimated_present(t);
        c.dirty = true;
        assert!(!c.pending_vblank);
        assert_eq!(c.next_tick_delay(t, interval), interval);
        assert!(!c.is_due(t + interval / 2, interval));
        assert!(c.is_due(t + interval, interval));

        // A real vblank after the empty render lifts the gate again.
        c.note_vblank(t + interval / 2);
        assert!(c.is_due(t + interval / 2, interval));
    }

    #[test]
    fn page_flip_time_maps_onto_instant() {
        let now = Instant::now();
        let mono_now = Duration::from_secs(1000);
        let flip = mono_now - Duration::from_millis(3);
        assert_eq!(
            instant_at_monotonic(flip, mono_now, now),
            now - Duration::from_millis(3)
        );
        // A future timestamp never maps past `now`.
        assert_eq!(
            instant_at_monotonic(mono_now + Duration::from_millis(1), mono_now, now),
            now
        );
    }

    #[test]
    fn vblank_seq_follows_the_kernel_counter_and_stays_monotonic() {
        assert_eq!(next_vblank_seq(0, Some(5000)), 5000);
        assert_eq!(next_vblank_seq(5000, Some(5002)), 5002);
        // No metadata: step by one.
        assert_eq!(next_vblank_seq(5002, None), 5003);
        // Counter went backwards (wrap / CRTC reset): never publish a
        // smaller value.
        assert_eq!(next_vblank_seq(5003, Some(7)), 5004);
        assert_eq!(next_vblank_seq(5004, Some(5004)), 5005);
    }

    #[test]
    fn predicted_present_is_the_next_vblank_after_now() {
        let interval = Duration::from_millis(16);
        let last = Instant::now();
        // Right at the vblank: the next frame is one interval away.
        assert_eq!(predicted_present_lead(last, [(last, interval)]), interval);
        // 3 ms after it: 13 ms left.
        assert_eq!(
            predicted_present_lead(last + Duration::from_millis(3), [(last, interval)]),
            Duration::from_millis(13)
        );
        // Two frames skipped (static screen): still on the vblank phase.
        assert_eq!(
            predicted_present_lead(last + Duration::from_millis(37), [(last, interval)]),
            Duration::from_millis(11)
        );
    }

    #[test]
    fn predicted_present_takes_the_soonest_output() {
        let now = Instant::now();
        let slow = (now - Duration::from_millis(2), Duration::from_millis(16));
        let fast = (now - Duration::from_millis(1), Duration::from_millis(7));
        assert_eq!(
            predicted_present_lead(now, [slow, fast]),
            Duration::from_millis(6)
        );
    }

    #[test]
    fn stale_or_missing_presents_predict_now() {
        let now = Instant::now() + Duration::from_secs(5);
        assert_eq!(predicted_present_lead(now, []), Duration::ZERO);
        let stale = (now - Duration::from_secs(2), Duration::from_millis(16));
        assert_eq!(predicted_present_lead(now, [stale]), Duration::ZERO);
        // A present stamped after `now` (DRM time vs. our reading) counts
        // as "just presented", and a zero interval can't divide by zero.
        let future = (now + Duration::from_millis(1), Duration::from_millis(16));
        assert_eq!(
            predicted_present_lead(now, [future]),
            Duration::from_millis(16)
        );
        assert_eq!(
            predicted_present_lead(now, [(now, Duration::ZERO)]),
            Duration::from_nanos(1)
        );
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
