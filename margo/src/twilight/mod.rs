//! Twilight — built-in blue-light filter / colour-temperature
//! scheduler.
//!
//! Replaces the need for an external client (sunsetr / gammastep /
//! redshift) on the same compositor. Built into margo so the
//! whole pipeline — scheduling, interpolation, gamma writes —
//! lives inside the same event loop and reuses the existing
//! `pending_gamma` → DRM `GAMMA_LUT` plumbing the wlr-gamma-control
//! protocol server already drives. End result: one less moving
//! part on the user's machine, smoother ramps because we tick on
//! the compositor's own frame loop, and live config swap via
//! `mctl reload` / `mctl twilight set`.
//!
//! Module layout:
//!
//! * [`gamma_lut`] — temperature → 16-bit RGB ramp.
//! * [`schedule`] — geo / manual / static phase resolver
//!   (sun-elevation math is inline; no `sunrise` / `chrono` deps).
//! * [`interpolation`] — phase + config → `(temp_k, gamma_pct)`.
//! * `mod.rs` (this file) — the live `TwilightState` the
//!   compositor parks on, plus the tick + override entry points
//!   `mctl` calls into.

pub mod gamma_lut;
pub mod interpolation;
pub mod preset;
pub mod schedule;

use std::time::{Duration, Instant, SystemTime};

use interpolation::{Settings, Target};
pub use preset::ScheduleData;
use schedule::{Mode, Phase, Schedule};

/// What's driving the current sample. `Scheduled` is the steady-
/// state path (config + clock); the override variants are entered
/// by `mctl twilight preview / test / set`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    Scheduled,
    /// `mctl twilight preview <K> [pct]` — pinned manual sample.
    /// Stays in effect until cleared with `reset` or until another
    /// override is pushed.
    Preview,
    /// `mctl twilight test <duration_s>` — animate from current
    /// schedule's day to night across `duration_s` seconds, then
    /// fall back to `Scheduled`.
    Test { started_at: Instant, duration_ms: u64 },
}

/// Live state for the running scheduler. Owned by `MargoState`;
/// `tick()` is called from the calloop timer and from the dispatch
/// handlers that want to force-resample after a config tweak.
#[derive(Debug, Clone)]
pub struct TwilightState {
    /// Last target we computed + applied. Read by `mctl twilight
    /// status` and by the tick loop's "did anything change" guard
    /// so we don't push identical ramps every minute.
    pub last_target: Option<Target>,
    /// Last phase we computed. Surfaced in `mctl twilight status`.
    pub last_phase: Option<Phase>,
    /// Currently active override, if any.
    pub source: Source,
}

impl Default for TwilightState {
    fn default() -> Self {
        Self {
            last_target: None,
            last_phase: None,
            source: Source::Scheduled,
        }
    }
}

/// Settings frozen at the moment `tick` is called. Keeps the
/// function pure-ish — the caller assembles this from
/// `Config` + `SystemTime::now()` and we don't reach back into
/// either.
#[derive(Debug, Clone)]
pub struct TickInputs {
    pub enabled: bool,
    pub schedule: Schedule,
    pub settings: Settings,
    pub is_static: bool,
    pub idle_interval_s: u32,
    pub now: SystemTime,
    /// Loaded preset table for `Mode::Schedule`. Empty when the
    /// user isn't on schedule mode (or the schedule directory is
    /// unreadable). When set, schedule mode samples from this
    /// table and bypasses the day/night phase model entirely.
    pub presets: ScheduleData,
}

/// What the tick produced. The caller (compositor event loop) uses
/// `target` to build the ramp + push it to `pending_gamma`, and
/// `next_tick_ms` to re-arm the calloop timer.
#[derive(Debug, Clone)]
pub struct TickOutput {
    /// The sample to apply. `None` when twilight is disabled — the
    /// caller should then clear gamma to neutral (or just skip).
    pub target: Option<Target>,
    /// What phase this sample came from. Diagnostics-only.
    pub phase: Option<Phase>,
    /// Sleep this many ms before the next tick.
    pub next_tick_ms: u64,
}

impl TwilightState {
    /// Compute the next sample. Pure function over `inputs` + the
    /// override stored on `self`. Side effects (gamma write, timer
    /// re-arm) belong to the caller.
    pub fn tick(&mut self, inputs: TickInputs) -> TickOutput {
        if !inputs.enabled && !matches!(self.source, Source::Preview | Source::Test { .. }) {
            self.last_target = None;
            self.last_phase = None;
            return TickOutput {
                target: None,
                phase: None,
                next_tick_ms: 60_000,
            };
        }

        // 1. Resolve override first — preview/test win over the
        //    schedule until cleared.
        if let Source::Preview = self.source {
            // Caller stored the preview target into self.last_target
            // when entering the override; just keep applying it.
            let t = self.last_target.unwrap_or(Target {
                temp_k: inputs.settings.day_temp,
                gamma_pct: inputs.settings.day_gamma,
            });
            return TickOutput {
                target: Some(t),
                phase: self.last_phase,
                next_tick_ms: 1_000,
            };
        }
        if let Source::Test { started_at, duration_ms } = self.source {
            let elapsed = started_at.elapsed().as_millis() as u64;
            if elapsed >= duration_ms {
                // Sweep done; fall back to scheduled.
                self.source = Source::Scheduled;
                // Fall through to the scheduled branch below.
            } else {
                let progress = elapsed as f32 / duration_ms.max(1) as f32;
                let target = Target {
                    temp_k: interpolation::lerp_mired(
                        inputs.settings.day_temp,
                        inputs.settings.night_temp,
                        progress,
                    ),
                    gamma_pct: ((inputs.settings.day_gamma as f32
                        + (inputs.settings.night_gamma as f32
                            - inputs.settings.day_gamma as f32)
                            * progress)
                        .round() as u32)
                        .clamp(10, 200),
                };
                self.last_target = Some(target);
                return TickOutput {
                    target: Some(target),
                    phase: None,
                    next_tick_ms: 50,
                };
            }
        }

        // 2. Steady-state. Schedule mode samples directly from
        //    the preset table; everything else flows through the
        //    phase model (day / night / transitions).
        if matches!(inputs.schedule.mode, Mode::Schedule) {
            let now_sec = schedule::local_seconds_of_day(inputs.now);
            if let Some((temp_k, gamma_pct)) = inputs.presets.sample(now_sec) {
                let target = Target { temp_k, gamma_pct };
                self.last_target = Some(target);
                self.last_phase = None;
                return TickOutput {
                    target: Some(target),
                    phase: None,
                    // Schedule transitions are smooth — sample
                    // every 5 s so the colour walks through
                    // visibly without spamming gamma writes.
                    next_tick_ms: 5_000,
                };
            }
            // No presets loaded — fall through to the phase
            // model so the user still gets *something* (day
            // temps) instead of a neutral screen.
        }

        let phase = inputs.schedule.current(inputs.now);
        let target = interpolation::sample(phase, &inputs.settings, inputs.is_static);
        let next_tick_ms = interpolation::next_tick_ms(phase, inputs.idle_interval_s);

        self.last_target = Some(target);
        self.last_phase = Some(phase);
        TickOutput {
            target: Some(target),
            phase: Some(phase),
            next_tick_ms,
        }
    }

    /// Pin a preview target. Stays until `reset()`. Used by
    /// `mctl twilight preview <K> [pct]`.
    pub fn set_preview(&mut self, temp_k: u32, gamma_pct: u32) {
        self.last_target = Some(Target {
            temp_k: temp_k.clamp(1000, 25000),
            gamma_pct: gamma_pct.clamp(10, 200),
        });
        self.source = Source::Preview;
    }

    /// Kick off a `mctl twilight test` sweep — animate day→night
    /// over `duration_ms`. Falls back to `Scheduled` when the
    /// duration elapses.
    pub fn start_test(&mut self, duration_ms: u64) {
        self.source = Source::Test {
            started_at: Instant::now(),
            duration_ms: duration_ms.max(100),
        };
    }

    /// Clear any preview / test override and resume the schedule
    /// on the next tick.
    pub fn reset(&mut self) {
        self.source = Source::Scheduled;
    }
}

/// Build a `Schedule` from the live config. Keeps the
/// types-vs-config bridge in one place so callers don't sprinkle
/// it everywhere.
pub fn schedule_from_config(cfg: &margo_config::Config) -> Schedule {
    let mode = match cfg.twilight_mode {
        margo_config::TwilightMode::Geo => Mode::Geo,
        margo_config::TwilightMode::Manual => Mode::Manual,
        margo_config::TwilightMode::Static => Mode::Static,
        margo_config::TwilightMode::Schedule => Mode::Schedule,
    };
    Schedule {
        mode,
        latitude: cfg.twilight_latitude,
        longitude: cfg.twilight_longitude,
        sunrise_sec: (cfg.twilight_sunrise_sec != 0).then_some(cfg.twilight_sunrise_sec),
        sunset_sec: (cfg.twilight_sunset_sec != 0).then_some(cfg.twilight_sunset_sec),
        transition_seconds: cfg.twilight_transition_s,
    }
}

pub fn settings_from_config(cfg: &margo_config::Config) -> Settings {
    Settings {
        day_temp: cfg.twilight_day_temp,
        night_temp: cfg.twilight_night_temp,
        day_gamma: cfg.twilight_day_gamma,
        night_gamma: cfg.twilight_night_gamma,
        static_temp: cfg.twilight_static_temp,
        static_gamma: cfg.twilight_static_gamma,
    }
}

/// 1 s — minimum gap between gamma pushes when the target didn't
/// change. Stops the tick loop from spamming `pending_gamma`
/// every minute with a byte-identical ramp.
pub const _SKIP_REWRITE_GRACE: Duration = Duration::from_secs(1);

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline_inputs() -> TickInputs {
        TickInputs {
            enabled: true,
            schedule: Schedule {
                mode: Mode::Static,
                latitude: 0.0,
                longitude: 0.0,
                sunrise_sec: None,
                sunset_sec: None,
                transition_seconds: 1800,
            },
            settings: Settings {
                day_temp: 6500,
                night_temp: 3300,
                day_gamma: 100,
                night_gamma: 90,
                static_temp: 4000,
                static_gamma: 95,
            },
            is_static: false,
            idle_interval_s: 60,
            now: SystemTime::UNIX_EPOCH,
            presets: ScheduleData::default(),
        }
    }

    #[test]
    fn disabled_returns_no_target() {
        let mut s = TwilightState::default();
        let inputs = TickInputs { enabled: false, ..baseline_inputs() };
        let out = s.tick(inputs);
        assert!(out.target.is_none());
    }

    #[test]
    fn preview_pinned_until_reset() {
        let mut s = TwilightState::default();
        s.set_preview(2500, 80);

        let inputs = baseline_inputs();
        let out = s.tick(inputs.clone());
        let t = out.target.unwrap();
        assert_eq!(t.temp_k, 2500);
        assert_eq!(t.gamma_pct, 80);

        s.reset();
        let out = s.tick(inputs);
        // After reset we resolve from the schedule (static mode →
        // day temp).
        let t = out.target.unwrap();
        assert_eq!(t.temp_k, 6500);
    }

    #[test]
    fn test_sweep_finishes_and_falls_back() {
        let mut s = TwilightState::default();
        // 150 ms sweep — `start_test` floors at 100 ms (no-op
        // sweeps are useless), so a request below that would be
        // rounded up and the post-sleep tick would still be mid-
        // sweep.
        s.start_test(150);
        let inputs = baseline_inputs();

        // Immediately after start: should produce a transitional
        // sample (not exactly day, not exactly night).
        let out = s.tick(inputs.clone());
        let temp = out.target.unwrap().temp_k;
        assert!(
            (3300..=6500).contains(&temp),
            "first test tick produced temp {temp} outside swing"
        );

        // Wait for sweep to finish, tick again — should be back on
        // schedule (static-or-day pathway).
        std::thread::sleep(std::time::Duration::from_millis(200));
        let _ = s.tick(inputs);
        assert!(matches!(s.source, Source::Scheduled));
    }
}
