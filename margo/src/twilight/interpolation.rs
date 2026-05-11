//! Phase + config → (temp_k, gamma_pct) sample.
//!
//! Two interpolations live here:
//!
//!   * **Temperature** is interpolated in *mired space* (1/Kelvin),
//!     not Kelvin. Mireds are perceptually closer to "how warm
//!     does this look" than absolute Kelvin — a 1000 K jump at
//!     3000 K is way more visible than the same jump at 9000 K.
//!     This is the same trick redshift / gammastep / sunsetr use.
//!
//!   * **Gamma** (brightness percentage) is plain linear, because
//!     the gamma curve already lives downstream in `gamma_lut.rs`'s
//!     sRGB encode — adding a second non-linearity here would
//!     double-encode.

use crate::twilight::schedule::Phase;

/// Final colour-and-brightness sample the LUT builder wants.
#[derive(Debug, Clone, Copy)]
pub struct Target {
    pub temp_k: u32,
    pub gamma_pct: u32,
}

/// Inputs the interpolator needs from config to resolve a phase
/// into a concrete `Target`.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub day_temp: u32,
    pub night_temp: u32,
    pub day_gamma: u32,
    pub night_gamma: u32,
    pub static_temp: u32,
    pub static_gamma: u32,
}

/// Sample the `phase` against `settings` and produce the final
/// temperature + gamma the LUT builder will turn into a ramp.
pub fn sample(phase: Phase, settings: &Settings, is_static: bool) -> Target {
    if is_static {
        return Target {
            temp_k: settings.static_temp,
            gamma_pct: settings.static_gamma,
        };
    }
    match phase {
        Phase::Day => Target {
            temp_k: settings.day_temp,
            gamma_pct: settings.day_gamma,
        },
        Phase::Night => Target {
            temp_k: settings.night_temp,
            gamma_pct: settings.night_gamma,
        },
        Phase::TransitionToDay { progress } => Target {
            temp_k: lerp_mired(settings.night_temp, settings.day_temp, progress),
            gamma_pct: lerp_linear(settings.night_gamma, settings.day_gamma, progress),
        },
        Phase::TransitionToNight { progress } => Target {
            temp_k: lerp_mired(settings.day_temp, settings.night_temp, progress),
            gamma_pct: lerp_linear(settings.day_gamma, settings.night_gamma, progress),
        },
    }
}

/// Interpolate between two Kelvin values *in mired space*.
/// 1 mired = 1_000_000 / Kelvin. Working in mireds means a 500 K
/// step at 3000 K is treated as the same perceptual jump as a
/// 5500 K step at 10000 K — which matches what the eye actually
/// sees on a colour-temperature swing.
pub fn lerp_mired(from_k: u32, to_k: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let m_from = 1_000_000.0 / from_k.max(1) as f32;
    let m_to = 1_000_000.0 / to_k.max(1) as f32;
    let m = m_from + (m_to - m_from) * t;
    (1_000_000.0 / m.max(1.0)).round() as u32
}

fn lerp_linear(from: u32, to: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    (from as f32 + (to as f32 - from as f32) * t).round() as u32
}

/// How long to sleep before the next tick. Adaptive:
///
///   * **Stable Day or Night** → use the configured idle interval
///     (default 60 s). Nothing to interpolate, no need to wake
///     more often than once a minute.
///
///   * **In a transition** → much finer ticks so the perceived
///     ramp is smooth. We aim for ~250 ms — anything finer is
///     wasted (the user can't perceive sub-quarter-second steps
///     in a 30-minute ramp).
///
///   * **Forced sweep (preview/test)** → caller picks; this
///     function isn't called.
pub fn next_tick_ms(phase: Phase, idle_seconds: u32) -> u64 {
    match phase {
        Phase::Day | Phase::Night => (idle_seconds.max(1) as u64) * 1000,
        Phase::TransitionToDay { .. } | Phase::TransitionToNight { .. } => 250,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            day_temp: 6500,
            night_temp: 3300,
            day_gamma: 100,
            night_gamma: 90,
            static_temp: 4000,
            static_gamma: 95,
        }
    }

    #[test]
    fn day_returns_day_settings() {
        let t = sample(Phase::Day, &settings(), false);
        assert_eq!(t.temp_k, 6500);
        assert_eq!(t.gamma_pct, 100);
    }

    #[test]
    fn night_returns_night_settings() {
        let t = sample(Phase::Night, &settings(), false);
        assert_eq!(t.temp_k, 3300);
        assert_eq!(t.gamma_pct, 90);
    }

    #[test]
    fn static_mode_overrides_phase() {
        // Even if phase says day, static config should win.
        let t = sample(Phase::Day, &settings(), true);
        assert_eq!(t.temp_k, 4000);
        assert_eq!(t.gamma_pct, 95);
    }

    #[test]
    fn transition_endpoints_match_day_night() {
        let start = sample(Phase::TransitionToDay { progress: 0.0 }, &settings(), false);
        let end = sample(Phase::TransitionToDay { progress: 1.0 }, &settings(), false);
        // At progress 0 we should be at night temp; at 1, day temp.
        assert_eq!(start.temp_k, 3300);
        assert_eq!(end.temp_k, 6500);
        assert_eq!(start.gamma_pct, 90);
        assert_eq!(end.gamma_pct, 100);
    }

    #[test]
    fn mired_midpoint_is_warmer_than_arithmetic_midpoint() {
        // Mired interp between 3000 K and 6000 K at t=0.5:
        //   m_from = 333.33, m_to = 166.67  → m_mid = 250
        //   k_mid  = 1e6 / 250 = 4000 K
        // Arithmetic midpoint would be 4500 K. The mired path
        // sits warmer (lower K) at the same t, which is what we
        // want — perceptually "halfway through the ramp" feels
        // like 4000 K, not 4500 K.
        let mid = lerp_mired(3000, 6000, 0.5);
        assert!((3900..=4100).contains(&mid), "mired midpoint = {mid}");
    }

    #[test]
    fn transition_tick_is_far_finer_than_idle() {
        let idle =
            next_tick_ms(Phase::Day, 60);
        let trans = next_tick_ms(Phase::TransitionToDay { progress: 0.5 }, 60);
        assert!(idle >= 60_000);
        assert!(trans <= 500);
    }
}
