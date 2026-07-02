//! Read the laptop battery state from `/sys/class/power_supply`.
//!
//! Cheap: two `read_to_string` calls per refresh, only enabled when a
//! `BATn` directory exists. Desktops skip silently.

use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct BatteryInfo {
    pub percent: u8,
    pub charging: bool,
}

pub fn read() -> Option<BatteryInfo> {
    for n in 0..=3 {
        let dir = format!("/sys/class/power_supply/BAT{n}");
        if !Path::new(&dir).is_dir() {
            continue;
        }
        let cap = fs::read_to_string(format!("{dir}/capacity")).ok()?;
        let status = fs::read_to_string(format!("{dir}/status")).ok()?;
        return classify(&cap, &status);
    }
    None
}

/// Turn the raw `capacity` + `status` sysfs strings into a [`BatteryInfo`].
/// Split out from [`read`] so the percent parse + charging classification
/// are unit-testable without a real `/sys/class/power_supply` node.
/// `None` when the capacity isn't a `u8`.
pub(crate) fn classify(capacity: &str, status: &str) -> Option<BatteryInfo> {
    let percent = capacity.trim().parse::<u8>().ok()?;
    let charging = matches!(status.trim(), "Charging" | "Full" | "Not charging");
    Some(BatteryInfo { percent, charging })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_percent_and_trims_sysfs_newlines() {
        // sysfs values carry a trailing newline — must be trimmed.
        let info = classify("87\n", "Discharging\n").expect("valid capacity");
        assert_eq!(info.percent, 87);
        assert!(!info.charging);
    }

    #[test]
    fn charging_states_are_classified_as_charging() {
        // logind/sysfs report these three as "power is coming in / topped up".
        for s in ["Charging", "Full", "Not charging"] {
            assert!(
                classify("50", s).unwrap().charging,
                "`{s}` must count as charging"
            );
        }
    }

    #[test]
    fn discharging_and_unknown_states_are_not_charging() {
        for s in ["Discharging", "Unknown", ""] {
            assert!(
                !classify("50", s).unwrap().charging,
                "`{s}` must not count as charging"
            );
        }
    }

    #[test]
    fn non_numeric_or_out_of_range_capacity_is_rejected() {
        assert!(classify("", "Full").is_none());
        assert!(classify("full", "Full").is_none());
        // 256 overflows a u8 — parse fails, no panic.
        assert!(classify("256", "Full").is_none());
    }
}
