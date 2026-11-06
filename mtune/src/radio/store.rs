// SPDX-License-Identifier: GPL-3.0-or-later

//! The user's stations: favourites from the directory and custom stream
//! URLs, in `$XDG_DATA_HOME/mtune/radio.json`.
//!
//! Loaded and saved through `margo_fs::side_file`, so a hand-edited file
//! that no longer parses is never overwritten by an empty list.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::Station;
use crate::net;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RadioLibrary {
    /// Favourites and custom stations, in the user's order.
    #[serde(default)]
    pub stations: Vec<Station>,
    /// The country filter last used in the browser (ISO code; empty = all).
    #[serde(default)]
    pub country: String,
}

/// Why a custom station was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomError {
    /// Not an http(s) URL with a host.
    BadUrl,
    /// That stream is already in the list.
    Duplicate,
}

impl RadioLibrary {
    pub fn path() -> PathBuf {
        margo_fs::xdg::data_home().join("mtune").join("radio.json")
    }

    /// The saved library; empty when missing or unreadable (a broken file
    /// is kept, with a `.bak` copy, and `false` is returned with it).
    pub fn load() -> (Self, bool) {
        match margo_fs::side_file::load(&Self::path(), |s| serde_json::from_str::<Self>(s)) {
            margo_fs::side_file::SideLoad::Loaded(l) => (l, true),
            margo_fs::side_file::SideLoad::Missing => (Self::default(), true),
            margo_fs::side_file::SideLoad::Broken { error, .. } => {
                log::warn!("mtune: {} does not parse: {error}", Self::path().display());
                (Self::default(), false)
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let body = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        margo_fs::side_file::save(&path, &body, |s| {
            serde_json::from_str::<Self>(s).map(|_| ())
        })
        .map_err(|e| e.to_string())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.stations.iter().any(|s| s.key() == key)
    }

    /// Star or un-star a directory station. Returns whether it is now a
    /// favourite.
    pub fn toggle_favourite(&mut self, station: &Station) -> bool {
        let key = station.key();
        if let Some(i) = self.stations.iter().position(|s| s.key() == key) {
            self.stations.remove(i);
            false
        } else {
            self.stations.push(station.clone());
            true
        }
    }

    /// Add a stream by URL. An empty `name` is guessed from the host.
    pub fn add_custom(&mut self, name: &str, url: &str) -> Result<Station, CustomError> {
        let url = net::validate_http_url(url).ok_or(CustomError::BadUrl)?;
        if self.stations.iter().any(|s| s.url == url) {
            return Err(CustomError::Duplicate);
        }
        let station = Station {
            name: custom_name(name, &url),
            url,
            custom: true,
            ..Default::default()
        };
        self.stations.push(station.clone());
        Ok(station)
    }

    /// Rename / re-point a custom station (directory favourites keep the
    /// directory's data and are only removable).
    pub fn edit_custom(&mut self, key: &str, name: &str, url: &str) -> Result<(), CustomError> {
        let url = net::validate_http_url(url).ok_or(CustomError::BadUrl)?;
        if self.stations.iter().any(|s| s.url == url && s.key() != key) {
            return Err(CustomError::Duplicate);
        }
        if let Some(s) = self
            .stations
            .iter_mut()
            .find(|s| s.custom && s.key() == key)
        {
            s.name = custom_name(name, &url);
            s.url = url;
        }
        Ok(())
    }

    pub fn remove(&mut self, key: &str) {
        self.stations.retain(|s| s.key() != key);
    }
}

fn custom_name(name: &str, url: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        net::url_host(url).unwrap_or(url).to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(uuid: &str, url: &str) -> Station {
        Station {
            uuid: uuid.into(),
            name: uuid.into(),
            url: url.into(),
            ..Default::default()
        }
    }

    #[test]
    fn favourite_toggles_on_and_off() {
        let mut lib = RadioLibrary::default();
        let s = station("a1", "https://a/x");
        assert!(lib.toggle_favourite(&s));
        assert!(lib.contains(&s.key()));
        assert!(!lib.toggle_favourite(&s));
        assert!(lib.stations.is_empty());
    }

    #[test]
    fn custom_streams_are_validated_named_and_deduplicated() {
        let mut lib = RadioLibrary::default();
        let s = lib
            .add_custom("", " https://stream.example:8000/live ")
            .unwrap();
        assert_eq!(s.name, "stream.example");
        assert_eq!(s.url, "https://stream.example:8000/live");
        assert!(s.custom && s.uuid.is_empty());
        assert_eq!(
            lib.add_custom("Again", "https://stream.example:8000/live"),
            Err(CustomError::Duplicate)
        );
        assert_eq!(
            lib.add_custom("x", "file:///etc/passwd"),
            Err(CustomError::BadUrl)
        );
        assert_eq!(lib.stations.len(), 1);
    }

    #[test]
    fn editing_a_custom_station_changes_its_key_with_its_url() {
        let mut lib = RadioLibrary::default();
        let s = lib.add_custom("Old", "https://a/old").unwrap();
        lib.add_custom("Other", "https://a/other").unwrap();
        assert_eq!(
            lib.edit_custom(&s.key(), "New", "https://a/other"),
            Err(CustomError::Duplicate)
        );
        lib.edit_custom(&s.key(), " New ", "https://a/new").unwrap();
        assert_eq!(lib.stations[0].name, "New");
        assert_eq!(lib.stations[0].url, "https://a/new");
        // A directory favourite is not editable.
        let fav = station("rb1", "https://dir/x");
        lib.toggle_favourite(&fav);
        lib.edit_custom(&fav.key(), "Hijack", "https://evil/x")
            .unwrap();
        assert_eq!(lib.stations[2].url, "https://dir/x");
    }

    #[test]
    fn remove_drops_by_key() {
        let mut lib = RadioLibrary::default();
        for (u, url) in [("a", "https://a/"), ("b", "https://b/")] {
            lib.toggle_favourite(&station(u, url));
        }
        lib.remove("rb:a");
        let left: Vec<_> = lib.stations.iter().map(|s| s.uuid.as_str()).collect();
        assert_eq!(left, ["b"]);
    }

    #[test]
    fn an_old_or_partial_file_still_loads() {
        let lib: RadioLibrary =
            serde_json::from_str(r#"{"stations":[{"name":"X","url":"https://x/"}]}"#).unwrap();
        assert_eq!(lib.stations[0].key(), "url:https://x/");
        assert_eq!(lib.country, "");
        let empty: RadioLibrary = serde_json::from_str("{}").unwrap();
        assert!(empty.stations.is_empty());
    }
}
