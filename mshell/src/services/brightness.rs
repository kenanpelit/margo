//! Display backlight via sysfs + brightnessctl.
//!
//! Reads `/sys/class/backlight/*/{brightness, max_brightness}` for
//! the current value (no extra deps, fast), writes through
//! `brightnessctl set N%` which already handles the polkit prompt /
//! group permission so we don't have to add a udev rule of our own.

use std::path::PathBuf;
use std::process::Command;

pub fn current_percent() -> Option<u8> {
    let dev = find_backlight()?;
    let cur = read_u64(&dev.join("brightness"))?;
    let max = read_u64(&dev.join("max_brightness"))?;
    if max == 0 {
        return None;
    }
    Some(((cur as f64 / max as f64) * 100.0).round().clamp(0.0, 100.0) as u8)
}

pub fn set_percent(percent: u8) {
    let _ = Command::new("brightnessctl")
        .args(["set", &format!("{}%", percent.min(100))])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn find_backlight() -> Option<PathBuf> {
    let root = PathBuf::from("/sys/class/backlight");
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        return Some(entry.path());
    }
    None
}

fn read_u64(p: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}
