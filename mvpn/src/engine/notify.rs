//! Desktop notifications via `notify-send` — best-effort, never blocks or
//! fails an action. Mirrors osc-mullvad's `notify()`: honours
//! `MVPN_NO_NOTIFY` / `OSC_MULLVAD_NO_NOTIFY` to silence (e.g. during sweeps),
//! and collapses successive toasts into one slot with the synchronous hint so
//! a burst of actions doesn't stack.

use std::process::Command;

/// True when notifications are suppressed via env.
fn muted() -> bool {
    let on = |k: &str| std::env::var(k).map(|v| v == "1").unwrap_or(false);
    on("MVPN_NO_NOTIFY") || on("OSC_MULLVAD_NO_NOTIFY")
}

/// Fire a notification. `icon` is an icon name (e.g. `network-vpn-symbolic`).
pub fn send(summary: &str, body: &str, icon: &str) {
    if muted() {
        return;
    }
    let _ = Command::new("notify-send")
        .args([
            "-a",
            "Mullvad VPN",
            "-i",
            icon,
            // Replace the previous mvpn toast instead of stacking.
            "-h",
            "string:x-canonical-private-synchronous:mvpn",
            summary,
            body,
        ])
        .spawn();
}

/// Connected → vpn icon; disconnected → the crossed-out icon.
pub fn icon_for(connected: bool) -> &'static str {
    if connected {
        "network-vpn-symbolic"
    } else {
        "network-vpn-disconnected-symbolic"
    }
}
