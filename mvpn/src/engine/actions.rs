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

/// Toggle the tunnel protocol (WireGuard ↔ OpenVPN) and reconnect.
pub fn toggle_protocol() -> bool {
    let next = if status::query()
        .tunnel_type
        .to_lowercase()
        .contains("wireguard")
    {
        "openvpn"
    } else {
        "wireguard"
    };
    sys::mullvad_ok(&["relay", "set", "tunnel-protocol", next]) && reconnect()
}
