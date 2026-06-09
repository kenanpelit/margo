//! Parse `mullvad relay list` into a country catalog + relay picking.
//!
//! `relay list` layout:
//! ```text
//! Germany (de)
//! \tBerlin (ber) @ 52.5°N, 13.4°E
//! \t\tde-ber-wg-006 (1.2.3.4, …) - hosted by X (rented)
//! ```
//! Top-level = country, one tab = city, two tabs = a relay. Relay lines end in
//! `(rented)` or `(Mullvad-owned)`.

use super::sys;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Country {
    pub name: String,
    pub code: String,
    pub relays: u32,
}

/// Parse the catalog: (name, code, relay-count) per country.
pub fn parse_countries(s: &str) -> Vec<Country> {
    let mut out: Vec<Country> = Vec::new();
    for line in s.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with("\t\t") {
            if let Some(last) = out.last_mut() {
                last.relays += 1;
            }
        } else if !line.starts_with('\t')
            && let Some(open) = line.rfind('(')
        {
            let name = line[..open].trim().to_string();
            let code = line[open + 1..].trim_end_matches(')').trim().to_string();
            if !name.is_empty() && !code.is_empty() {
                out.push(Country {
                    name,
                    code,
                    relays: 0,
                });
            }
        }
    }
    out
}

pub fn countries() -> Vec<Country> {
    parse_countries(&sys::mullvad(&["relay", "list"]))
}

/// Ownership filter for relay picking.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    Any,
    Owned,
    Rented,
}

/// Collect relay IDs matching country/city/ownership from a `relay list` dump.
/// Empty `country` = all; empty `city` = any city in the country.
pub fn pick_relays(list: &str, country: &str, city: &str, own: Ownership) -> Vec<String> {
    let mut relays = Vec::new();
    for line in list.lines() {
        if !line.starts_with("\t\t") {
            continue;
        }
        let t = line.trim();
        // Relay id is the first token, like `de-ber-wg-006`.
        let Some(id) = t.split_whitespace().next() else {
            continue;
        };
        if !looks_like_relay(id) {
            continue;
        }
        if !country.is_empty() && !id.starts_with(&format!("{country}-")) {
            continue;
        }
        if !city.is_empty() && !id.starts_with(&format!("{country}-{city}-")) {
            continue;
        }
        let owned = t.ends_with("(Mullvad-owned)");
        let rented = t.ends_with("(rented)");
        let keep = match own {
            Ownership::Any => true,
            Ownership::Owned => owned,
            Ownership::Rented => rented,
        };
        if keep {
            relays.push(id.to_string());
        }
    }
    relays
}

/// The public IPv4 of a relay, from its `relay list` line:
/// `us-nyc-wg-803 (23.234.101.3, 2607:…) - hosted by …` → `23.234.101.3`.
pub fn relay_ipv4(list: &str, relay: &str) -> Option<String> {
    for line in list.lines() {
        if !line.starts_with("\t\t") {
            continue;
        }
        let t = line.trim();
        let mut it = t.split_whitespace();
        if it.next() != Some(relay) {
            continue;
        }
        // Next token is "(23.234.101.3," — strip the punctuation.
        let raw = it.next().unwrap_or("");
        let ip: String = raw
            .trim_matches(|c| c == '(' || c == ')' || c == ',')
            .to_string();
        if ip.split('.').count() == 4 && ip.split('.').all(|o| o.parse::<u8>().is_ok()) {
            return Some(ip);
        }
    }
    None
}

/// `de-ber-wg-006` / `us-nyc-ovpn-101` shape: cc-city-(wg|ovpn)-NNN.
fn looks_like_relay(id: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    parts.len() >= 4 && (parts.contains(&"wg") || parts.contains(&"ovpn"))
}

/// Pick a pseudo-random relay id from the live list. Uses a cheap time-seeded
/// index (no rand dep — selection randomness here is non-cryptographic).
pub fn random(country: &str, city: &str, own: Ownership) -> Option<String> {
    let list = sys::mullvad(&["relay", "list"]);
    let relays = pick_relays(&list, country, city, own);
    if relays.is_empty() {
        return None;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    Some(relays[nanos % relays.len()].clone())
}

/// Expand a country-group keyword into ISO codes (mirrors osc-mullvad).
pub fn group_codes(group: &str) -> Option<Vec<&'static str>> {
    const EUROPE: &[&str] = &[
        "at", "be", "bg", "ch", "cz", "de", "dk", "ee", "es", "fi", "fr", "gb", "gr", "hr", "hu",
        "ie", "it", "nl", "no", "pl", "pt", "ro", "rs", "se", "si", "sk", "tr", "ua",
    ];
    const AMERICAS: &[&str] = &["br", "ca", "cl", "co", "mx", "pe", "us"];
    const ASIA: &[&str] = &["au", "hk", "id", "jp", "my", "ph", "sg", "th"];
    const AFRICA_ME: &[&str] = &["il", "ng", "za"];
    const OTHER: &[&str] = &["nz"];
    match group {
        "europe" | "eu" => Some(EUROPE.to_vec()),
        "americas" | "na" => Some(AMERICAS.to_vec()),
        "asia" | "apac" | "asia/pacific" | "asia-pacific" => Some(ASIA.to_vec()),
        "africa" | "africa/me" | "africa-me" => Some(AFRICA_ME.to_vec()),
        "other" => Some(OTHER.to_vec()),
        "all" => {
            let mut v = EUROPE.to_vec();
            v.extend_from_slice(AMERICAS);
            v.extend_from_slice(ASIA);
            v.extend_from_slice(AFRICA_ME);
            v.extend_from_slice(OTHER);
            Some(v)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Germany (de)\n\
\tBerlin (ber) @ 52.5°N, 13.4°E\n\
\t\tde-ber-wg-006 (1.2.3.4) - hosted by X (rented)\n\
\t\tde-ber-wg-007 (1.2.3.5) - hosted by Mullvad (Mullvad-owned)\n\
\tFrankfurt (fra) @ 50°N, 8°E\n\
\t\tde-fra-wg-001 (1.2.3.6) - hosted by Y (rented)\n\
USA (us)\n\
\tNew York (nyc) @ 40°N, 74°W\n\
\t\tus-nyc-wg-101 (2.2.2.2) - hosted by Z (rented)\n";

    #[test]
    fn counts_relays_per_country() {
        let c = parse_countries(SAMPLE);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].code, "de");
        assert_eq!(c[0].relays, 3);
        assert_eq!(c[1].code, "us");
        assert_eq!(c[1].relays, 1);
    }

    #[test]
    fn picks_by_country_city_ownership() {
        assert_eq!(pick_relays(SAMPLE, "de", "", Ownership::Any).len(), 3);
        assert_eq!(pick_relays(SAMPLE, "de", "ber", Ownership::Any).len(), 2);
        assert_eq!(
            pick_relays(SAMPLE, "de", "", Ownership::Owned),
            vec!["de-ber-wg-007"]
        );
        assert_eq!(pick_relays(SAMPLE, "de", "", Ownership::Rented).len(), 2);
        assert_eq!(pick_relays(SAMPLE, "", "", Ownership::Any).len(), 4);
    }

    #[test]
    fn extracts_relay_ipv4() {
        assert_eq!(relay_ipv4(SAMPLE, "de-ber-wg-006"), Some("1.2.3.4".into()));
        assert_eq!(relay_ipv4(SAMPLE, "nope"), None);
    }

    #[test]
    fn group_expands() {
        assert!(group_codes("europe").unwrap().contains(&"de"));
        assert!(group_codes("nope").is_none());
        assert!(group_codes("all").unwrap().len() > 30);
    }
}
