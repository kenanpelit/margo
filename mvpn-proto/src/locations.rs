//! `mvpn locations` — Mullvad's relay locations as a country → city tree,
//! the one shape the VPN menu's country / city pickers (exit and multihop
//! entry) read.

use serde::{Deserialize, Serialize};

/// One country with relays, its cities in `mullvad relay list` order.
#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Country {
    /// Relay-list code (`de`).
    pub code: String,
    pub name: String,
    pub cities: Vec<City>,
}

/// One city with relays.
#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct City {
    /// Relay-list code (`fra`).
    pub code: String,
    pub name: String,
    /// Degrees; Mullvad prints signed values (a `°N` / `°W` suffix that
    /// does not flip the sign), so east and north are positive.
    pub latitude: f64,
    pub longitude: f64,
    pub relays: u32,
}

impl Country {
    /// Relays across all cities.
    pub fn relays(&self) -> u32 {
        self.cities.iter().map(|c| c.relays).sum()
    }
}

/// Parse `mvpn locations` (a JSON array); anything unreadable is empty.
pub fn parse(json: &str) -> Vec<Country> {
    serde_json::from_str(json.trim()).unwrap_or_default()
}

/// The countries (and, inside each, the cities) a search `needle` keeps:
/// a country whose name or code matches keeps all its cities; otherwise
/// only its matching cities, and it drops out when none match. An empty
/// needle keeps everything. The bool says the country was kept for a city
/// match (the menu then shows it expanded).
pub fn filter<'a>(
    countries: &'a [Country],
    needle: &str,
) -> Vec<(&'a Country, Vec<&'a City>, bool)> {
    let needle = needle.trim().to_lowercase();
    let hit = |s: &str| s.to_lowercase().contains(&needle);
    countries
        .iter()
        .filter_map(|c| {
            if needle.is_empty() || hit(&c.name) || c.code.eq_ignore_ascii_case(&needle) {
                return Some((c, c.cities.iter().collect(), false));
            }
            let cities: Vec<&City> = c
                .cities
                .iter()
                .filter(|ci| hit(&ci.name) || ci.code.eq_ignore_ascii_case(&needle))
                .collect();
            (!cities.is_empty()).then_some((c, cities, true))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Vec<Country> {
        let city = |code: &str, name: &str, relays| City {
            code: code.into(),
            name: name.into(),
            relays,
            ..City::default()
        };
        vec![
            Country {
                code: "de".into(),
                name: "Germany".into(),
                cities: vec![city("ber", "Berlin", 2), city("fra", "Frankfurt", 3)],
            },
            Country {
                code: "se".into(),
                name: "Sweden".into(),
                cities: vec![city("got", "Gothenburg", 1), city("sto", "Stockholm", 4)],
            },
        ]
    }

    #[test]
    fn json_round_trip() {
        let t = tree();
        let json = serde_json::to_string(&t).expect("json");
        assert_eq!(parse(&json), t);
        assert_eq!(t[0].relays(), 5);
        assert!(parse("garbage").is_empty());
        assert!(parse("").is_empty());
    }

    #[test]
    fn filter_keeps_countries_and_matching_cities() {
        let t = tree();
        let names = |v: Vec<(&Country, Vec<&City>, bool)>| {
            v.into_iter()
                .map(|(c, cities, by_city)| {
                    (
                        c.code.clone(),
                        cities.iter().map(|ci| ci.code.clone()).collect::<Vec<_>>(),
                        by_city,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(filter(&t, "").len(), 2);
        // A country match keeps every city.
        assert_eq!(
            names(filter(&t, "germ")),
            vec![("de".into(), vec!["ber".into(), "fra".into()], false)]
        );
        // A city match keeps that city only, flagged.
        assert_eq!(
            names(filter(&t, "stock")),
            vec![("se".into(), vec!["sto".into()], true)]
        );
        // Codes match exactly, not as substrings ("e" is in every name).
        assert_eq!(
            names(filter(&t, "FRA")),
            vec![("de".into(), vec!["fra".into()], true)]
        );
        assert!(filter(&t, "tokyo").is_empty());
    }
}
