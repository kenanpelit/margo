//! Bluetooth status via `bluetoothctl`.
//!
//! Returns:
//!   * `enabled` — adapter is `Powered = yes`
//!   * `connected_devices` — number of currently-connected devices

use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub enabled: bool,
    pub connected_devices: u32,
}

pub fn current() -> Snapshot {
    let enabled = adapter_powered();
    let connected_devices = connected_count();
    Snapshot {
        enabled,
        connected_devices,
    }
}

fn adapter_powered() -> bool {
    let Ok(out) = Command::new("bluetoothctl")
        .args(["show"])
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines()
        .find_map(|l| l.strip_prefix("\tPowered:"))
        .or_else(|| s.lines().find_map(|l| l.strip_prefix("Powered:")))
        .map(|rest| rest.trim() == "yes")
        .unwrap_or(false)
}

fn connected_count() -> u32 {
    let Ok(out) = Command::new("bluetoothctl")
        .args(["devices", "Connected"])
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return 0;
    };
    if !out.status.success() {
        return 0;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines().filter(|l| l.starts_with("Device ")).count() as u32
}

pub fn toggle_power() {
    // Use a quick check + the matching subcommand so we don't have
    // to spawn `bluetoothctl power toggle` (which isn't supported
    // on all bluez versions).
    let target = if adapter_powered() { "off" } else { "on" };
    let _ = Command::new("bluetoothctl")
        .args(["power", target])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
