//! Connection + setting actions — thin, tested-parser-backed wrappers over the
//! `mullvad` CLI.

use super::{relays, status, sys};

pub fn connect() -> bool {
    sys::mullvad_ok(&["connect"])
}

pub fn disconnect() -> bool {
    sys::mullvad_ok(&["disconnect"])
}

pub fn reconnect() -> bool {
    sys::mullvad_ok(&["reconnect"])
}

/// Toggle: connect if down, disconnect if up.
pub fn toggle() -> bool {
    if status::query().connected {
        disconnect()
    } else {
        connect()
    }
}

/// Set relay location (country, optional city) then connect.
pub fn set_location(country: &str, city: Option<&str>) -> bool {
    let mut args = vec!["relay", "set", "location", country];
    if let Some(c) = city {
        args.push(c);
    }
    sys::mullvad_ok(&args) && connect()
}

/// Set a specific relay by id (`relay set location <cc> <city> <id>` form
/// accepts the full hostname as a single location token) then connect.
pub fn set_relay(id: &str) -> bool {
    sys::mullvad_ok(&["relay", "set", "location", id]) && connect()
}

/// Connect to a random relay matching the filters.
pub fn random(country: &str, city: &str, own: relays::Ownership) -> bool {
    match relays::random(country, city, own) {
        Some(id) => set_relay(&id),
        None => false,
    }
}

pub fn set_lockdown(on: bool) -> bool {
    sys::mullvad_ok(&["lockdown-mode", "set", if on { "on" } else { "off" }])
}

pub fn set_autoconnect(on: bool) -> bool {
    sys::mullvad_ok(&["auto-connect", "set", if on { "on" } else { "off" }])
}

/// Whether WireGuard quantum-resistant key exchange is on (`tunnel get`).
pub fn quantum_on() -> bool {
    parse_quantum(&sys::mullvad(&["tunnel", "get"]))
}

pub(crate) fn parse_quantum(s: &str) -> bool {
    for line in s.lines() {
        if let Some(v) = line.trim().strip_prefix("Quantum resistance:") {
            return v.trim().eq_ignore_ascii_case("on");
        }
    }
    false
}

/// Toggle WireGuard quantum-resistant key exchange and reconnect.
/// (Modern Mullvad is WireGuard-only — the old OpenVPN protocol toggle is gone,
/// so this chip now drives the meaningful WG security knob.)
pub fn toggle_quantum() -> bool {
    let next = if quantum_on() { "off" } else { "on" };
    sys::mullvad_ok(&["tunnel", "set", "quantum-resistant", next]) && reconnect()
}

#[cfg(test)]
mod tests {
    use super::parse_quantum;

    #[test]
    fn parses_quantum_state() {
        assert!(parse_quantum(
            "Tunnel\n    Quantum resistance:     on\n    DAITA: false"
        ));
        assert!(!parse_quantum("    Quantum resistance: off"));
        assert!(!parse_quantum("nothing"));
    }
}
