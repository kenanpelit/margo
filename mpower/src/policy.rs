//! Pure decision policy — the testable heart of mpower.
//!
//! The stateful streak/cooldown bookkeeping lives in the daemon
//! (`src/main.rs`); everything here is a pure function of the current
//! reading + config, so it can be unit-tested without touching the system.

use crate::config::Config;

pub const PERFORMANCE: &str = "performance";
pub const BALANCED: &str = "balanced";
pub const POWER_SAVER: &str = "power-saver";

/// Which load band a CPU reading falls into, on AC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// Asking for performance (aggregate or a single hot core is high).
    High,
    /// Clearly idle (both aggregate and hottest core are low).
    Low,
    /// In between — hold the current profile.
    Mid,
}

/// Classify a CPU busy reading. High wins over Low (a hot core forces
/// performance even if the average is modest); the Low band requires *both*
/// the aggregate and the hottest core to be calm.
pub fn classify(avg: f64, max: f64, cfg: &Config) -> Band {
    if avg >= cfg.high_avg_percent as f64 || max >= cfg.high_max_percent as f64 {
        Band::High
    } else if avg <= cfg.low_avg_percent as f64 && max <= cfg.low_max_percent as f64 {
        Band::Low
    } else {
        Band::Mid
    }
}

/// Desired profile while on battery: power-saver below the configured charge
/// floor (when enabled), otherwise balanced. Performance is never chosen on
/// battery.
pub fn battery_target(cfg: &Config, battery_percent: Option<u32>) -> &'static str {
    if cfg.battery_saver_below > 0
        && let Some(b) = battery_percent
        && b <= cfg.battery_saver_below
    {
        return POWER_SAVER;
    }
    BALANCED
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::default() // high 35/85, low 18/70, saver ≤ 20
    }

    #[test]
    fn high_average_is_high_band() {
        assert_eq!(classify(40.0, 50.0, &cfg()), Band::High);
    }

    #[test]
    fn hot_single_core_forces_high_even_with_low_average() {
        assert_eq!(classify(10.0, 90.0, &cfg()), Band::High);
    }

    #[test]
    fn calm_on_both_axes_is_low_band() {
        assert_eq!(classify(10.0, 40.0, &cfg()), Band::Low);
    }

    #[test]
    fn low_average_but_warm_core_is_mid() {
        // avg below low_avg but hottest core above low_max → neither → Mid.
        assert_eq!(classify(10.0, 75.0, &cfg()), Band::Mid);
    }

    #[test]
    fn battery_drops_to_power_saver_below_floor() {
        assert_eq!(battery_target(&cfg(), Some(15)), POWER_SAVER);
        assert_eq!(battery_target(&cfg(), Some(20)), POWER_SAVER); // at-or-below
        assert_eq!(battery_target(&cfg(), Some(50)), BALANCED);
    }

    #[test]
    fn battery_saver_disabled_stays_balanced() {
        let mut c = cfg();
        c.battery_saver_below = 0;
        assert_eq!(battery_target(&c, Some(5)), BALANCED);
    }

    #[test]
    fn battery_with_unknown_charge_stays_balanced() {
        assert_eq!(battery_target(&cfg(), None), BALANCED);
    }
}
