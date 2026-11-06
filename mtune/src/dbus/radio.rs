// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded radio inputs at the external IPC boundary.

use crate::radio::{Station, api::Query, store::RadioLibrary};

pub fn station(name: &str, url: &str) -> Result<Station, String> {
    text(name, 512)?;
    text(url, 4096)?;
    let parsed = gtk::glib::Uri::parse(url.trim(), gtk::glib::UriFlags::NONE)
        .map_err(|_| "Invalid station URL".to_string())?;
    if !matches!(parsed.scheme().as_str(), "http" | "https")
        || parsed.host().is_none_or(|h| h.is_empty())
        || parsed.userinfo().is_some()
    {
        return Err("Use an HTTP(S) station URL with a host and no credentials".into());
    }
    RadioLibrary::default()
        .add_custom(name, url)
        .map_err(|e| format!("{e:?}"))
}

pub fn text(value: &str, max: usize) -> Result<(), String> {
    if value.len() > max || value.chars().any(char::is_control) {
        return Err("Radio input is too long or contains control characters".into());
    }
    Ok(())
}

pub fn query(name: &str, country: &str) -> Result<Query, String> {
    text(name, 256)?;
    if !country.is_empty()
        && (country.len() != 2 || !country.bytes().all(|b| b.is_ascii_alphabetic()))
    {
        return Err("Country must be an ISO two-letter code or empty".into());
    }
    Ok(Query {
        name: name.trim().into(),
        countrycode: country.to_ascii_uppercase(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radio_inputs_reject_unsafe_urls_and_oversized_values() {
        for url in [
            "file:///etc/passwd",
            "https://",
            "https://host/\0x",
            "https://user:pass@host/x",
            "https://:8000/x",
        ] {
            assert!(station("Radio", url).is_err(), "{url:?}");
        }
        assert!(station(&"a".repeat(513), "https://radio.example/live").is_err());
        assert!(station("Radio", &format!("https://host/{}", "a".repeat(4096))).is_err());
        assert!(query(&"a".repeat(257), "US").is_err());
        assert!(query("jazz", "USA").is_err());
        assert!(query("jazz", "12").is_err());
        assert_eq!(query(" jazz ", "tr").unwrap().countrycode, "TR");
        assert_eq!(
            station("", " https://radio.example/live ").unwrap().name,
            "radio.example"
        );
    }
}
