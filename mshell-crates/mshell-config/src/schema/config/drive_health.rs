//! `drive_health.*` — background drive monitoring for the CPU dashboard's
//! Drives tab: alert notifications, trend history and the thresholds both
//! use. Everything is off by default; the Drives tab works without it.

use reactive_stores::{Patch, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct DriveHealthConfig {
    /// Watch the internal drives in the background and send a desktop
    /// notification when an issue appears or gets worse: temperature
    /// (sysfs, every minute) and, after each full SMART read, wear, spare,
    /// media errors, bad sectors, critical-warning flags and a failed
    /// self-assessment. Off by default.
    pub alerts: bool,
    /// Also notify when an issue clears.
    pub notify_recovery: bool,
    /// Alert when an established internal drive disappears.
    pub missing_alerts: bool,
    /// Minute-scans a drive may be absent before the missing-drive alert
    /// (1–20).
    pub missing_grace_scans: u32,
    /// Drive temperature (°C) that counts as hot (amber in the tab).
    pub temp_warning_c: u32,
    /// Drive temperature (°C) that counts as critical (red in the tab).
    pub temp_critical_c: u32,
    /// Rated endurance used (%, NVMe "Percentage Used") that needs
    /// attention.
    pub wear_warning_pct: u32,
    /// Keep a bounded trend history (temperature every
    /// `history_interval_minutes`, SMART counters at every full read) for
    /// the per-drive charts. Off by default.
    pub history: bool,
    /// Minutes between saved temperature samples (15–1440).
    pub history_interval_minutes: u32,
    /// Days of history kept (1–365).
    pub history_retention_days: u32,
}

impl Default for DriveHealthConfig {
    fn default() -> Self {
        Self {
            alerts: false,
            notify_recovery: true,
            missing_alerts: true,
            missing_grace_scans: 3,
            temp_warning_c: 70,
            temp_critical_c: 85,
            wear_warning_pct: 80,
            history: false,
            history_interval_minutes: 60,
            history_retention_days: 30,
        }
    }
}

impl DriveHealthConfig {
    /// `missing_grace_scans` within its range.
    pub fn grace_scans(&self) -> u32 {
        self.missing_grace_scans.clamp(1, 20)
    }

    /// `history_interval_minutes` within its range, in seconds.
    pub fn history_interval_secs(&self) -> u64 {
        u64::from(self.history_interval_minutes.clamp(15, 1440)) * 60
    }

    /// `history_retention_days` within its range, in seconds.
    pub fn retention_secs(&self) -> u64 {
        u64::from(self.history_retention_days.clamp(1, 365)) * 86_400
    }

    /// The temperature thresholds, warning below critical (a swapped pair
    /// is put back in order).
    pub fn temp_thresholds(&self) -> (i64, i64) {
        let (w, c) = (
            i64::from(self.temp_warning_c),
            i64::from(self.temp_critical_c),
        );
        (w.min(c), w.max(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_inert_and_ranges_clamp() {
        let d = DriveHealthConfig::default();
        assert!(!d.alerts && !d.history);
        assert_eq!(d.temp_thresholds(), (70, 85));
        assert_eq!(d.history_interval_secs(), 3600);
        assert_eq!(d.retention_secs(), 30 * 86_400);
        let odd = DriveHealthConfig {
            missing_grace_scans: 0,
            history_interval_minutes: 1,
            history_retention_days: 9999,
            temp_warning_c: 90,
            temp_critical_c: 60,
            ..d
        };
        assert_eq!(odd.grace_scans(), 1);
        assert_eq!(odd.history_interval_secs(), 15 * 60);
        assert_eq!(odd.retention_secs(), 365 * 86_400);
        assert_eq!(odd.temp_thresholds(), (60, 90));
    }
}
