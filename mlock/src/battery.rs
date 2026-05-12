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
        let percent = cap.trim().parse::<u8>().ok()?;
        let charging = matches!(status.trim(), "Charging" | "Full" | "Not charging");
        return Some(BatteryInfo { percent, charging });
    }
    None
}
