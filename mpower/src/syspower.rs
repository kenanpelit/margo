//! AC + battery state from `/sys/class/power_supply`.
//!
//! Deliberately sysfs-only (no UPower/D-Bus): the daemon ticks frequently,
//! so polling these files each tick catches plug/unplug within one interval
//! with zero async machinery. Mirrors the detection the retired
//! `ppp-auto-profile` script used.

use std::fs;
use std::path::Path;

const SUPPLY_DIR: &str = "/sys/class/power_supply";

fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// `true` if running on AC. When power-supply info is entirely absent
/// (desktop without the sysfs nodes) we assume AC — the safe default for a
/// machine that has no battery to protect.
pub fn on_ac() -> bool {
    let Ok(entries) = fs::read_dir(SUPPLY_DIR) else {
        return true;
    };
    let paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();

    // Prefer explicit "Mains" adapters.
    let mut mains_found = false;
    for p in &paths {
        if read_trim(p.join("type")).as_deref() == Some("Mains") {
            mains_found = true;
            if read_trim(p.join("online")).as_deref() == Some("1") {
                return true;
            }
        }
    }
    if mains_found {
        return false; // had a mains adapter, none online → on battery
    }

    // Fallback: any supply reporting online == 1.
    let mut saw_online = false;
    let mut online = false;
    for p in &paths {
        if let Some(v) = read_trim(p.join("online")) {
            saw_online = true;
            if v == "1" {
                online = true;
            }
        }
    }
    if saw_online {
        online
    } else {
        true // no information at all → assume AC
    }
}

/// First battery's charge percentage (0–100), or `None` when there is no
/// battery device.
pub fn battery_percent() -> Option<u32> {
    let entries = fs::read_dir(SUPPLY_DIR).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if read_trim(p.join("type")).as_deref() == Some("Battery")
            && let Some(cap) = read_trim(p.join("capacity"))
            && let Ok(v) = cap.parse::<u32>()
        {
            return Some(v.min(100));
        }
    }
    None
}
