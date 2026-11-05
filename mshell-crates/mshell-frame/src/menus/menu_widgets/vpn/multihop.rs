//! The VPN menu's multihop controls: a switch plus the entry location
//! (country, then optionally a city), from `mvpn toggles` (`multihop=`,
//! `entry=`) and set with `mvpn multihop on|off` / `mvpn entry …`.

use mshell_utils::mvpn::locations::Country;

/// The entry location as `mvpn toggles` prints it (`entry=`): `any`,
/// `se`, `se got`, `se got se-got-wg-001`, or a custom list's name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Entry {
    /// Empty = any country.
    pub country: String,
    /// Empty = any city.
    pub city: String,
}

impl Entry {
    pub(crate) fn parse(value: &str) -> Self {
        let mut it = value.split_whitespace();
        match (it.next(), it.next()) {
            (None | Some("any"), _) => Self::default(),
            (Some(cc), city) => Self {
                country: cc.to_string(),
                city: city.unwrap_or("").to_string(),
            },
        }
    }
}

/// Dropdown labels: "Any country" + country names.
pub(crate) fn country_labels(tree: &[Country]) -> Vec<String> {
    std::iter::once("Any country".to_string())
        .chain(tree.iter().map(|c| c.name.clone()))
        .collect()
}

/// Dropdown labels for a country's cities: "Any city" + city names.
pub(crate) fn city_labels(tree: &[Country], country_index: u32) -> Vec<String> {
    let cities = country_at(tree, country_index).map_or(&[][..], |c| c.cities.as_slice());
    std::iter::once("Any city".to_string())
        .chain(cities.iter().map(|c| c.name.clone()))
        .collect()
}

/// The country behind a country-dropdown index (0 = any).
fn country_at(tree: &[Country], index: u32) -> Option<&Country> {
    (index as usize).checked_sub(1).and_then(|i| tree.get(i))
}

/// Dropdown indices showing `entry`: (country, city), 0 = any. An entry
/// the tree does not know (a custom list, a relay list not loaded yet)
/// shows as any.
pub(crate) fn selection(tree: &[Country], entry: &Entry) -> (u32, u32) {
    let Some(ci) = tree.iter().position(|c| c.code == entry.country) else {
        return (0, 0);
    };
    let city = tree[ci]
        .cities
        .iter()
        .position(|c| c.code == entry.city)
        .map_or(0, |i| i as u32 + 1);
    (ci as u32 + 1, city)
}

/// The `mvpn` argv that sets the entry the dropdowns show.
pub(crate) fn entry_args(tree: &[Country], country_index: u32, city_index: u32) -> Vec<String> {
    let Some(country) = country_at(tree, country_index) else {
        return vec!["entry".into(), "any".into()];
    };
    let mut args = vec!["entry".to_string(), country.code.clone()];
    if let Some(city) = (city_index as usize)
        .checked_sub(1)
        .and_then(|i| country.cities.get(i))
    {
        args.push(city.code.clone());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_utils::mvpn::locations::City;

    fn tree() -> Vec<Country> {
        let city = |code: &str, name: &str| City {
            code: code.into(),
            name: name.into(),
            relays: 1,
            ..City::default()
        };
        vec![
            Country {
                code: "de".into(),
                name: "Germany".into(),
                cities: vec![city("ber", "Berlin"), city("fra", "Frankfurt")],
            },
            Country {
                code: "se".into(),
                name: "Sweden".into(),
                cities: vec![city("got", "Gothenburg"), city("sto", "Stockholm")],
            },
        ]
    }

    #[test]
    fn entry_from_toggles() {
        assert_eq!(Entry::parse("any"), Entry::default());
        assert_eq!(Entry::parse(""), Entry::default());
        assert_eq!(
            Entry::parse("se got"),
            Entry {
                country: "se".into(),
                city: "got".into()
            }
        );
        assert_eq!(Entry::parse("se got se-got-wg-001").city, "got");
    }

    #[test]
    fn dropdowns_show_the_entry_and_set_it_back() {
        let t = tree();
        assert_eq!(country_labels(&t), ["Any country", "Germany", "Sweden"]);
        assert_eq!(city_labels(&t, 2), ["Any city", "Gothenburg", "Stockholm"]);
        assert_eq!(city_labels(&t, 0), ["Any city"]);
        assert_eq!(city_labels(&t, 9), ["Any city"]);
        for (value, sel, args) in [
            ("any", (0, 0), vec!["entry", "any"]),
            ("se", (2, 0), vec!["entry", "se"]),
            ("se sto", (2, 2), vec!["entry", "se", "sto"]),
            ("de fra de-fra-wg-001", (1, 2), vec!["entry", "de", "fra"]),
        ] {
            let s = selection(&t, &Entry::parse(value));
            assert_eq!(s, sel, "{value}");
            assert_eq!(entry_args(&t, s.0, s.1), args, "{value}");
        }
        // Unknown to the tree (custom list / not loaded): any.
        assert_eq!(selection(&t, &Entry::parse("My list")), (0, 0));
        assert_eq!(selection(&[], &Entry::parse("se")), (0, 0));
    }
}
