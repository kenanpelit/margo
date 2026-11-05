//! Multihop: route the tunnel through an entry relay before the exit
//! relay. Reads the relay constraints (`mullvad relay get`) and sets the
//! multihop state and the entry location (`mullvad relay set multihop` /
//! `relay set entry location`).

use super::sys;

/// A location constraint as `mullvad relay get` prints it (mullvad-types'
/// `LocationConstraintFormatter`): `any`, `country se`, `city got, se`,
/// `city got, se, hostname se-got-wg-001`, or a custom list's name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Constraint {
    #[default]
    Any,
    Country(String),
    City {
        country: String,
        city: String,
    },
    Hostname {
        country: String,
        city: String,
        hostname: String,
    },
    /// A custom list or anything newer than this parser: kept verbatim.
    Other(String),
}

impl Constraint {
    pub fn parse(value: &str) -> Self {
        let v = value.trim();
        if v.is_empty() || v == "any" {
            return Self::Any;
        }
        if let Some(cc) = v.strip_prefix("country ")
            && !cc.contains([',', ' '])
        {
            return Self::Country(cc.to_string());
        }
        if let Some(rest) = v.strip_prefix("city ") {
            let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
            match parts.as_slice() {
                [city, cc] => {
                    return Self::City {
                        country: cc.to_string(),
                        city: city.to_string(),
                    };
                }
                [city, cc, host] => {
                    if let Some(h) = host.strip_prefix("hostname ") {
                        return Self::Hostname {
                            country: cc.to_string(),
                            city: city.to_string(),
                            hostname: h.trim().to_string(),
                        };
                    }
                }
                _ => {}
            }
        }
        Self::Other(v.to_string())
    }

    /// The `mvpn toggles` form: `any`, `se`, `se got`, `se got
    /// se-got-wg-001`, or the raw text for anything else.
    pub fn short(&self) -> String {
        match self {
            Self::Any => "any".into(),
            Self::Country(cc) => cc.clone(),
            Self::City { country, city } => format!("{country} {city}"),
            Self::Hostname {
                country,
                city,
                hostname,
            } => format!("{country} {city} {hostname}"),
            Self::Other(raw) => raw.clone(),
        }
    }
}

/// The relay constraints multihop cares about.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelaySettings {
    pub location: Constraint,
    pub multihop: bool,
    pub entry: Constraint,
}

/// Parse `mullvad relay get`.
pub fn parse_relay_get(s: &str) -> RelaySettings {
    let mut out = RelaySettings::default();
    for line in s.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        match key.trim() {
            "Location" => out.location = Constraint::parse(value),
            "Multihop state" => {
                out.multihop = matches!(value.trim(), "enabled" | "on" | "true");
            }
            "Multihop entry" => out.entry = Constraint::parse(value),
            _ => {}
        }
    }
    out
}

pub fn relay_settings() -> RelaySettings {
    parse_relay_get(&sys::mullvad(&["relay", "get"]))
}

/// A relay-list code (`se`, `got`, `us-nyc` never): lowercase ASCII
/// letters / digits, 2-8 long. Anything else is refused before it can
/// reach `mullvad` as an argument.
pub fn valid_code(code: &str) -> bool {
    (2..=8).contains(&code.len())
        && code
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// The `mullvad` argv that sets the multihop entry: `any`, a country, or
/// a country + city. `None` for a code that is not a relay-list code.
pub fn entry_args(country: &str, city: Option<&str>) -> Option<Vec<String>> {
    let mut args: Vec<String> = ["relay", "set", "entry", "location"]
        .map(String::from)
        .to_vec();
    if country == "any" {
        return city.is_none().then(|| {
            args.push("any".into());
            args
        });
    }
    if !valid_code(country) || !city.is_none_or(valid_code) {
        return None;
    }
    args.push(country.into());
    args.extend(city.map(String::from));
    Some(args)
}

/// Turn multihop on or off (the daemon reconnects by itself).
pub fn set_multihop(on: bool) -> bool {
    sys::mullvad_ok(&["relay", "set", "multihop", if on { "on" } else { "off" }])
}

/// Set the multihop entry location.
pub fn set_entry(country: &str, city: Option<&str>) -> Result<(), String> {
    let args = entry_args(country, city).ok_or_else(|| {
        format!(
            "not a relay location: {}",
            [country, city.unwrap_or("")].join(" ").trim()
        )
    })?;
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = sys::run("mullvad", &argv, None);
    if out.success {
        Ok(())
    } else if !out.spawned {
        Err("the mullvad command is not installed".into())
    } else {
        Err(out
            .stderr
            .lines()
            .next()
            .unwrap_or("mullvad relay set entry failed")
            .trim_start_matches("Error: ")
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `mullvad relay get` captured on this machine (mullvad-cli 2026.5).
    const OFF: &str = include_str!("../../tests/fixtures/relay-get.txt");
    /// The same with multihop on: the community mullvad plugin's capture
    /// (mullvad-cli 2026.5; turning multihop on here would change the
    /// owner's live tunnel).
    const ON: &str = include_str!("../../tests/fixtures/relay-get-multihop.txt");

    #[test]
    fn parses_relay_get() {
        let r = parse_relay_get(OFF);
        assert!(!r.multihop);
        assert_eq!(
            r.location,
            Constraint::Hostname {
                country: "de".into(),
                city: "fra".into(),
                hostname: "de-fra-wg-002".into()
            }
        );
        assert_eq!(r.entry, Constraint::Country("se".into()));
        let r = parse_relay_get(ON);
        assert!(r.multihop);
        assert_eq!(r.location, Constraint::Country("ro".into()));
        assert_eq!(r.entry, Constraint::Country("se".into()));
        let r = parse_relay_get(&ON.replace("country se", "city got, se"));
        assert_eq!(
            r.entry,
            Constraint::City {
                country: "se".into(),
                city: "got".into()
            }
        );
        assert_eq!(parse_relay_get(""), RelaySettings::default());
    }

    #[test]
    fn constraint_forms() {
        assert_eq!(Constraint::parse("any"), Constraint::Any);
        assert_eq!(Constraint::parse("My list").short(), "My list");
        assert_eq!(Constraint::parse("country se").short(), "se");
        assert_eq!(Constraint::parse("city got, se").short(), "se got");
        assert_eq!(
            Constraint::parse("city got, se, hostname se-got-wg-001").short(),
            "se got se-got-wg-001"
        );
        assert_eq!(
            Constraint::parse("country a b"),
            Constraint::Other("country a b".into())
        );
    }

    #[test]
    fn entry_argv_is_validated() {
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            entry_args("se", None),
            Some(s(&["relay", "set", "entry", "location", "se"]))
        );
        assert_eq!(
            entry_args("se", Some("got")),
            Some(s(&["relay", "set", "entry", "location", "se", "got"]))
        );
        assert_eq!(
            entry_args("any", None),
            Some(s(&["relay", "set", "entry", "location", "any"]))
        );
        assert_eq!(entry_args("any", Some("got")), None);
        assert_eq!(entry_args("SE", None), None);
        assert_eq!(entry_args("se", Some("--help")), None);
        assert_eq!(entry_args("s", None), None);
        assert_eq!(entry_args("se;rm", None), None);
    }
}
