//! Network status via `nmcli`.
//!
//! Returns whatever is enough to drive the bar widget:
//!   * SSID + signal strength of the active wifi connection, or
//!     `None` when disconnected.
//!   * Whether a VPN connection (`type=vpn` or `wireguard`) is
//!     currently active — used to swap the wifi icon for a shield.
//!
//! Subprocess + parse instead of NetworkManager D-Bus for the same
//! reason audio uses wpctl: tiny dep tree, easy to reason about,
//! easy to swap for a native zbus client in a later stage.

use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub ssid: Option<String>,
    /// 0..=100 signal strength as reported by nmcli. 0 when not connected.
    pub signal: u8,
    pub vpn: bool,
}

pub fn current() -> Snapshot {
    let (ssid, signal) = wifi_active().unwrap_or((None, 0));
    let vpn = vpn_active();
    Snapshot { ssid, signal, vpn }
}

/// Pick the wifi line with `ACTIVE = yes`. nmcli's `-t` mode emits
/// colon-separated fields and escapes literal colons in values so a
/// plain `splitn(3, ':')` is safe for `ACTIVE,SSID,SIGNAL`.
fn wifi_active() -> Option<(Option<String>, u8)> {
    let out = Command::new("nmcli")
        .args(["-t", "-f", "ACTIVE,SSID,SIGNAL", "device", "wifi"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        let mut parts = line.splitn(3, ':');
        let active = parts.next()?;
        if active != "yes" {
            continue;
        }
        let ssid = parts.next().unwrap_or("").to_string();
        let signal: u8 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        return Some((Some(ssid).filter(|s| !s.is_empty()), signal));
    }
    None
}

fn vpn_active() -> bool {
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "TYPE,STATE", "connection", "show", "--active"])
        .output()
    else {
        return false;
    };
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines().any(|l| {
        let mut p = l.splitn(2, ':');
        matches!(p.next(), Some("vpn") | Some("wireguard"))
    })
}

/// Nerd-Font glyph picked by signal strength.
pub fn glyph(signal: u8, connected: bool, vpn: bool) -> &'static str {
    if vpn {
        return "\u{f023}"; // nf-fa-lock
    }
    if !connected {
        return "\u{f6ab}"; // nf-md-wifi_off (rough match)
    }
    match signal {
        0..=33 => "\u{f1eb}",  // wifi (we use a single glyph; per-strength glyphs come later)
        34..=66 => "\u{f1eb}",
        _ => "\u{f1eb}",
    }
}
