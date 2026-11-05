//! The `mvpn status --json` type, shared by `mvpn` (writer) and mshell
//! (reader): one field set, one parse of Mullvad's "Visible location"
//! string, so the two sides can never disagree about the exit city.

use serde::{Deserialize, Serialize};

pub mod health;
pub mod locations;

/// One tunnel status (`mvpn status --json`, one line per change with
/// `--follow`). Every field defaults, so an older or newer `mvpn` that
/// lacks / adds fields still parses.
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub connected: bool,
    pub connecting: bool,
    /// First status line verbatim (e.g. "Connected", "Disconnected").
    pub state: String,
    pub relay: String,
    /// Raw "Country, City. IPv4: …" visible-location string.
    pub location: String,
    pub ipv4: String,
    pub country: String,
    pub city: String,
    /// "WireGuard" / "OpenVPN", inferred from the tunnel interface / relay id.
    pub tunnel_type: String,
}

impl Status {
    /// Parse `mvpn status --json`; unparseable or empty output (no `mvpn`,
    /// daemon down) reads as disconnected. A status from an mvpn that only
    /// wrote the raw `location` gets its country / city split here, by the
    /// same rule mvpn uses.
    pub fn parse(json: &str) -> Self {
        let mut st: Self = serde_json::from_str(json.trim()).unwrap_or_default();
        if st.country.is_empty() && st.city.is_empty() && !st.location.is_empty() {
            let location = st.location.clone();
            st.set_location(&location);
        }
        st
    }

    /// Store `location` and the country / city / IPv4 split out of it.
    pub fn set_location(&mut self, location: &str) {
        let parts = VisibleLocation::parse(location);
        self.location = location.trim().to_string();
        self.country = parts.country;
        self.city = parts.city;
        self.ipv4 = parts.ipv4;
    }

    /// Where the tunnel exits, for a compact label: the city, else the
    /// country, else empty.
    pub fn place(&self) -> &str {
        if self.city.is_empty() {
            &self.country
        } else {
            &self.city
        }
    }

    /// `Connected · <relay>[ · <location>]` or `Disconnected`.
    pub fn summary(&self) -> String {
        if !self.connected {
            return "Disconnected".to_string();
        }
        let mut s = format!("Connected · {}", self.relay);
        if !self.location.is_empty() {
            s.push_str(" · ");
            s.push_str(&self.location);
        }
        s
    }
}

/// Mullvad's "Visible location" (`Country, City[. IPv4: a.b.c.d][, IPv6: …]`)
/// split into its parts. The city may itself hold a comma
/// (`USA, New York, NY`); without a comma the whole place is the country.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct VisibleLocation {
    pub country: String,
    pub city: String,
    pub ipv4: String,
}

impl VisibleLocation {
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        let ip_at = ["IPv4:", "IPv6:"]
            .iter()
            .filter_map(|k| raw.find(k))
            .min()
            .unwrap_or(raw.len());
        let place = raw[..ip_at].trim().trim_end_matches([',', '.']).trim();
        let ipv4 = raw
            .find("IPv4:")
            .map(|i| {
                raw[i + "IPv4:".len()..]
                    .trim_start()
                    .split([',', ' '])
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_default();
        let (country, city) = match place.split_once(',') {
            Some((country, city)) => (country.trim(), city.trim()),
            None => (place, ""),
        };
        Self {
            country: country.to_string(),
            city: city.to_string(),
            ipv4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_the_visible_location() {
        let p = |s: &str| {
            let v = VisibleLocation::parse(s);
            (v.country, v.city, v.ipv4)
        };
        let t = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
        assert_eq!(
            p("Germany, Frankfurt. IPv4: 185.213.155.228"),
            t("Germany", "Frankfurt", "185.213.155.228")
        );
        assert_eq!(p("Sweden, Stockholm"), t("Sweden", "Stockholm", ""));
        assert_eq!(
            p("USA, New York, NY. IPv4: 1.2.3.4"),
            t("USA", "New York, NY", "1.2.3.4")
        );
        assert_eq!(
            p("Germany, Frankfurt. IPv4: 1.2.3.4, IPv6: 2001:db8::1"),
            t("Germany", "Frankfurt", "1.2.3.4")
        );
        assert_eq!(
            p("Japan, Tokyo. IPv6: 2001:db8::1"),
            t("Japan", "Tokyo", "")
        );
        assert_eq!(p("Germany"), t("Germany", "", ""));
        assert_eq!(p(""), t("", "", ""));
    }

    #[test]
    fn place_is_the_city_else_the_country() {
        let mut s = Status::default();
        s.set_location("Germany, Frankfurt. IPv4: 1.2.3.4");
        assert_eq!(s.place(), "Frankfurt");
        s.set_location("Germany");
        assert_eq!(s.place(), "Germany");
    }

    #[test]
    fn json_round_trip_and_tolerant_parse() {
        let mut s = Status {
            connected: true,
            relay: "de-fra-wg-002".into(),
            ..Status::default()
        };
        s.set_location("Germany, Frankfurt. IPv4: 1.2.3.4");
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(Status::parse(&json), s);
        // An older mvpn wrote three fields only; spacing / escapes are fine.
        let old = Status::parse(
            r#"{ "connected" : true, "relay": "se-got-wg-001", "location": "Sweden, \"Gothenburg\"" }"#,
        );
        assert!(old.connected);
        assert_eq!(old.relay, "se-got-wg-001");
        assert_eq!(old.city, "\"Gothenburg\"", "split from the raw location");
        assert!(Status::parse("").relay.is_empty());
        assert!(!Status::parse("garbage").connected);
    }

    #[test]
    fn summary_line() {
        assert_eq!(Status::default().summary(), "Disconnected");
        let s = Status {
            connected: true,
            relay: "de-fra-wg-002".into(),
            location: "Germany, Frankfurt".into(),
            ..Status::default()
        };
        assert_eq!(
            s.summary(),
            "Connected · de-fra-wg-002 · Germany, Frankfurt"
        );
    }
}
