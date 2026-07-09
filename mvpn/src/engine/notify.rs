//! Desktop notifications via `notify-send` — best-effort, never blocks or
//! fails an action. Mirrors osc-mullvad's `notify()`: honours
//! `MVPN_NO_NOTIFY` / `OSC_MULLVAD_NO_NOTIFY` to silence (e.g. during sweeps),
//! collapses successive toasts into one slot with the synchronous hint so a
//! burst of actions doesn't stack, and asks for a short [`EXPIRE_MS`] life —
//! a connect/disconnect confirmation is read at a glance and shouldn't hold the
//! corner for the daemon's full default.

use std::process::Command;

/// How long the toast stays on screen, in ms — libnotify's `expire_timeout`.
/// Matches the 3 s the shell already gives its other transient toasts
/// (`mshell-launcher`'s activation feedback).
///
/// A daemon takes `min(its own popup duration, this)`, so the value can only
/// ever shorten the toast, never make it outstay the duration the user
/// configured. Without it libnotify sends `-1` ("server decides") and a
/// connect/disconnect confirmation sat on screen for the full default — 5 s
/// under mshell.
///
/// Never make this `0`: freedesktop reads 0 as "never expire", and the toast
/// would stick until dismissed by hand.
const EXPIRE_MS: &str = "3000";

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
            "-t",
            EXPIRE_MS,
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
