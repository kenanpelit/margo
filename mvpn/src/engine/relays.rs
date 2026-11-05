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

pub use mvpn_proto::locations::{City, Country as Location};

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

/// A leading signed decimal (`-34.92123°N` → -34.92123).
fn leading_number(s: &str) -> Option<f64> {
    let s = s.trim();
    let end = s
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || (i == 0 && c == '-')))
        .map_or(s.len(), |(i, _)| i);
    s[..end].parse().ok()
}

/// `\tTirana (tia) @ 41.32795°N, 19.81902°W` → (name, code, lat, lon).
/// Mullvad prints the raw signed degrees with a fixed `°N` / `°W` suffix
/// (Tirana is at 19.8° *east*), so the sign alone is the hemisphere.
fn parse_city_line(line: &str) -> Option<(String, String, f64, f64)> {
    let (head, coords) = line.trim().split_once(" @ ")?;
    let open = head.rfind('(')?;
    let name = head[..open].trim().to_string();
    let code = head[open + 1..].trim_end_matches(')').trim().to_string();
    let (lat, lon) = coords.split_once(',')?;
    if name.is_empty() || code.is_empty() {
        return None;
    }
    Some((name, code, leading_number(lat)?, leading_number(lon)?))
}

/// Parse the relay list into the country → city tree (`mvpn locations`).
/// Countries / cities without a relay line are dropped: nothing to
/// connect to there.
pub fn parse_locations(s: &str) -> Vec<Location> {
    let mut out: Vec<Location> = Vec::new();
    for line in s.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with("\t\t") {
            let id = line.split_whitespace().next().unwrap_or("");
            if looks_like_relay(id)
                && let Some(city) = out.last_mut().and_then(|c| c.cities.last_mut())
            {
                city.relays += 1;
            }
        } else if line.starts_with('\t') {
            if let (Some(country), Some((name, code, latitude, longitude))) =
                (out.last_mut(), parse_city_line(line))
            {
                country.cities.push(City {
                    code,
                    name,
                    latitude,
                    longitude,
                    relays: 0,
                });
            }
        } else if let Some(open) = line.rfind('(') {
            let name = line[..open].trim().to_string();
            let code = line[open + 1..].trim_end_matches(')').trim().to_string();
            if !name.is_empty() && !code.is_empty() {
                out.push(Location {
                    code,
                    name,
                    cities: Vec::new(),
                });
            }
        }
    }
    for c in &mut out {
        c.cities.retain(|ci| ci.relays > 0);
    }
    out.retain(|c| !c.cities.is_empty());
    out
}

/// The live country → city tree.
pub fn locations() -> Vec<Location> {
    parse_locations(&sys::mullvad(&["relay", "list"]))
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

    const REAL: &str = include_str!("../../tests/fixtures/relay-list.txt");

    #[test]
    fn parses_the_location_tree_from_a_real_relay_list() {
        let t = parse_locations(REAL);
        let codes: Vec<&str> = t.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(codes, ["al", "au", "bg", "de", "se", "tr", "us"]);
        let au = &t[1];
        assert_eq!(au.name, "Australia");
        let cities: Vec<&str> = au.cities.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(cities, ["adl", "bne", "mel", "per", "syd"]);
        assert_eq!(au.relays(), 10);
        // Signed degrees: south / west are negative whatever the suffix says.
        let adl = &au.cities[0];
        assert!((adl.latitude - -34.92123).abs() < 1e-9);
        assert!((adl.longitude - 138.5995).abs() < 1e-9);
        let us = &t[6];
        assert_eq!(us.cities[0].name, "Ashburn, VA");
        assert!((us.cities[0].longitude - -77.48744).abs() < 1e-9);
        // Non-ASCII names survive.
        assert_eq!(t[4].cities[1].name, "Malmö");
        // The tree and the flat catalog agree on the counts.
        let flat = parse_countries(REAL);
        for (c, f) in t.iter().zip(&flat) {
            assert_eq!((c.code.as_str(), c.relays()), (f.code.as_str(), f.relays));
        }
    }

    #[test]
    fn drops_places_without_relays() {
        let t = parse_locations(
            "Nowhere (nw)\n\tGhost (gho) @ 1.0°N, 2.0°W\nGermany (de)\n\tBerlin (ber) @ 52.5°N, 13.4°W\n\t\tde-ber-wg-001 (1.2.3.4) - hosted by X (rented)\n\tEmpty (emp) @ 1°N, 1°W\n",
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].cities.len(), 1);
        assert_eq!(t[0].cities[0].relays, 1);
        assert!(parse_locations("").is_empty());
    }

    #[test]
    fn group_expands() {
        assert!(group_codes("europe").unwrap().contains(&"de"));
        assert!(group_codes("nope").is_none());
        assert!(group_codes("all").unwrap().len() > 30);
    }
}
