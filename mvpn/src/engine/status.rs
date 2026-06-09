//! Parse `mullvad status` / `mullvad status -v` into a structured [`Status`].

use super::sys;

#[derive(Default, Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    pub connected: bool,
    pub connecting: bool,
    /// First status line verbatim (e.g. "Connected", "Disconnected").
    pub state: String,
    pub relay: String,
    /// Raw "Country, City" visible-location string.
    pub location: String,
    pub ipv4: String,
    pub country: String,
    pub city: String,
    /// "WireGuard" / "OpenVPN", inferred from the tunnel interface / relay id.
    pub tunnel_type: String,
}

/// Query the daemon and parse. Plain `status` already carries the Relay +
/// Visible-location lines and gives a clean relay id (no address suffix).
pub fn query() -> Status {
    parse(&sys::mullvad(&["status"]))
}

/// Pure parser (testable without the daemon).
///
/// Real `mullvad status` layout:
/// ```text
/// Connected
///     Relay:            de-fra-wg-002
///     Tunnel interface: wg0-mullvad
///     Visible location: Germany, Frankfurt. IPv4: 1.2.3.4
/// ```
/// Note the visible location is **Country, City** (country first), and there
/// is no "Tunnel:" line — the protocol is inferred from the interface / relay
/// id (`wg` → WireGuard, `ovpn` → OpenVPN).
pub fn parse(s: &str) -> Status {
    let mut st = Status {
        state: "Disconnected".into(),
        ..Default::default()
    };
    for (i, line) in s.lines().enumerate() {
        let t = line.trim();
        if i == 0 {
            st.state = t.to_string();
            let lc = t.to_lowercase();
            st.connected = lc.starts_with("connected");
            st.connecting = lc.starts_with("connecting");
        }
        if let Some(r) = t.strip_prefix("Relay:") {
            // Strip any trailing " (address:port/proto)" → bare relay id.
            st.relay = r.split_whitespace().next().unwrap_or("").to_string();
        }
        if let Some(iface) = t.strip_prefix("Tunnel interface:") {
            let iface = iface.trim().to_lowercase();
            if iface.starts_with("wg") {
                st.tunnel_type = "WireGuard".into();
            } else if !iface.is_empty() {
                st.tunnel_type = "OpenVPN".into();
            }
        }
        if let Some(l) = t.strip_prefix("Visible location:") {
            st.location = l.trim().to_string();
            // "Country, City. IPv4: 1.2.3.4"
            if let Some((before_ip, after_ip)) = l.split_once("IPv4:") {
                st.ipv4 = after_ip.trim().to_string();
                let trimmed = before_ip.trim().trim_end_matches('.').trim().to_string();
                if let Some((country, city)) = trimmed.split_once(',') {
                    st.country = country.trim().to_string();
                    st.city = city.trim().to_string();
                } else {
                    st.country = trimmed;
                }
            }
        }
    }
    // Fall back to the relay id for the protocol when no interface line.
    if st.tunnel_type.is_empty() && !st.relay.is_empty() {
        if st.relay.contains("-wg-") {
            st.tunnel_type = "WireGuard".into();
        } else if st.relay.contains("-ovpn-") {
            st.tunnel_type = "OpenVPN".into();
        }
    }
    st
}

/// A boolean `mullvad <sub> get` setting whose output ends in "on" / "off".
pub fn setting_on(sub: &str) -> bool {
    sys::mullvad(&[sub, "get"])
        .to_lowercase()
        .trim()
        .ends_with("on")
}

/// Account expiry date (YYYY-MM-DD), or "—".
pub fn account_expiry() -> String {
    parse_expiry(&sys::mullvad(&["account", "get"]))
}

pub fn parse_expiry(s: &str) -> String {
    for line in s.lines() {
        let t = line.trim();
        if let Some(e) = t.strip_prefix("Expires at") {
            return e.trim_start_matches(':').trim().chars().take(10).collect();
        }
        if let Some(e) = t.strip_prefix("Expires:") {
            return e.trim().chars().take(10).collect();
        }
    }
    "—".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_connected_real_format() {
        // Real `mullvad status`: Country, City; relay may carry an address;
        // protocol from the interface line.
        let s = "Connected\n\
                 Relay: de-fra-wg-002 (185.213.155.74:34609/UDP)\n\
                 Tunnel interface: wg0-mullvad\n\
                 Visible location: Germany, Frankfurt. IPv4: 1.2.3.4";
        let st = parse(s);
        assert!(st.connected);
        assert_eq!(st.relay, "de-fra-wg-002");
        assert_eq!(st.country, "Germany");
        assert_eq!(st.city, "Frankfurt");
        assert_eq!(st.ipv4, "1.2.3.4");
        assert_eq!(st.tunnel_type, "WireGuard");
    }

    #[test]
    fn protocol_falls_back_to_relay_id() {
        let st = parse("Connected\nRelay: us-nyc-ovpn-101\n");
        assert_eq!(st.tunnel_type, "OpenVPN");
    }

    #[test]
    fn parses_disconnected() {
        let st = parse("Disconnected");
        assert!(!st.connected);
        assert!(!st.connecting);
        assert_eq!(st.state, "Disconnected");
    }

    #[test]
    fn parses_connecting() {
        let st = parse("Connecting to de-ber-wg-006...");
        assert!(st.connecting);
        assert!(!st.connected);
    }

    #[test]
    fn expiry_both_forms() {
        assert_eq!(
            parse_expiry("Expires at: 2027-01-02 12:00:00 UTC"),
            "2027-01-02"
        );
        assert_eq!(parse_expiry("Expires: 2027-01-02"), "2027-01-02");
        assert_eq!(parse_expiry("nope"), "—");
    }
}
