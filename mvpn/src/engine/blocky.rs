//! Blocky DNS guard — a local ad-blocking resolver that must yield to the VPN's
//! DNS while connected and take over (fail-safe) when the tunnel is down.
//!
//! All privileged steps go through non-interactive sudo; if sudo would prompt
//! they fail fast rather than hang.

use super::{status, sys};

const UNIT: &str = "blocky.service";

pub fn unit_exists() -> bool {
    sys::ok("systemctl", &["list-unit-files", UNIT])
}

pub fn is_active() -> bool {
    sys::ok("systemctl", &["is-active", "--quiet", UNIT])
}

/// Stop blocky (so it doesn't fight the VPN's DNS).
pub fn stop() -> bool {
    if !unit_exists() || !is_active() {
        return true;
    }
    sys::sudo_n(&["systemctl", "stop", UNIT])
}

/// Start blocky + point the resolver at localhost (DNS ad-block while off-VPN).
pub fn start() -> bool {
    if !unit_exists() {
        return true;
    }
    if !is_active() && !sys::sudo_n(&["systemctl", "start", UNIT]) {
        return false;
    }
    set_resolver_local()
}

/// Rewrite `/etc/resolv.conf` to localhost (blocky listens there).
fn set_resolver_local() -> bool {
    sys::sudo_n(&[
        "sh",
        "-c",
        "printf 'nameserver 127.0.0.1\\nnameserver ::1\\n' > /etc/resolv.conf",
    ])
}

/// Fail-safe: VPN healthy → blocky off (VPN owns DNS); VPN down → blocky on.
/// Returns the state it drove blocky into ("on"/"off"/"n/a").
pub fn ensure() -> &'static str {
    if !unit_exists() {
        return "n/a";
    }
    if status::query().connected {
        stop();
        "off"
    } else {
        start();
        "on"
    }
}
