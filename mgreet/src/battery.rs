//! Battery status read straight from sysfs (`/sys/class/power_supply`) — no
//! deps, no D-Bus. Returns `None` on a desktop with no battery, so the greeter
//! simply omits the indicator there.

use std::fs;
use std::path::PathBuf;

pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// The first power-supply whose `type` is `Battery` (skips AC adapters / USB).
fn battery_dir() -> Option<PathBuf> {
    for entry in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let dir = entry.path();
        let is_battery = fs::read_to_string(dir.join("type"))
            .map(|t| t.trim() == "Battery")
            .unwrap_or(false);
        if is_battery {
            return Some(dir);
        }
    }
    None
}

/// Current charge + whether it's charging. `None` if there is no battery.
pub fn read() -> Option<Battery> {
    let dir = battery_dir()?;
    let percent = fs::read_to_string(dir.join("capacity"))
        .ok()?
        .trim()
        .parse::<u8>()
        .ok()?
        .min(100);
    let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
    let status = status.trim();
    Some(Battery {
        percent,
        charging: status == "Charging" || status == "Full",
    })
}

/// A Nerd Font battery glyph for the level (charging bolt when plugged in). The
/// percentage is always shown alongside, so a missing icon font degrades to
/// just the number rather than losing the reading.
pub fn icon(b: &Battery) -> &'static str {
    if b.charging {
        return "\u{f0084}"; // nf-md-battery_charging
    }
    match b.percent {
        0..=9 => "\u{f008e}",   // battery_outline (empty)
        10..=29 => "\u{f007b}", // battery_20
        30..=49 => "\u{f007d}", // battery_40
        50..=69 => "\u{f007f}", // battery_60
        70..=89 => "\u{f0081}", // battery_80
        _ => "\u{f0079}",       // battery (full)
    }
}
