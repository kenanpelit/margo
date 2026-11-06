// SPDX-License-Identifier: GPL-3.0-or-later

//! Internet radio: the radio-browser.info directory ([`api`]) and the
//! user's own stations — favourites and custom stream URLs ([`store`]).
//! Playing a station hands its stream to the normal player as a live
//! [`crate::audio::Song`].

pub mod api;
pub mod store;

use serde::{Deserialize, Serialize};

/// One station, from the directory or typed in by the user. Persisted in
/// `radio.json`, so every field but `name`/`url` defaults.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Station {
    /// radio-browser.info `stationuuid`; empty for a custom stream.
    #[serde(default)]
    pub uuid: String,
    pub name: String,
    /// The playable stream (`url_resolved` when the directory has one).
    pub url: String,
    #[serde(default)]
    pub favicon: String,
    /// Comma-separated genre tags.
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub countrycode: String,
    #[serde(default)]
    pub codec: String,
    /// kbit/s; 0 when unknown.
    #[serde(default)]
    pub bitrate: u32,
    #[serde(default)]
    pub votes: u64,
    /// HLS playlist rather than a plain Icecast/Shoutcast stream.
    #[serde(default)]
    pub hls: bool,
    /// Added by the user by URL, not from the directory.
    #[serde(default)]
    pub custom: bool,
}

impl Station {
    /// Identity across searches and the favourites file: the directory id,
    /// or the stream URL for a custom station.
    pub fn key(&self) -> String {
        if self.uuid.is_empty() {
            format!("url:{}", self.url)
        } else {
            format!("rb:{}", self.uuid)
        }
    }

    /// The first few genre tags, for a row subtitle.
    pub fn short_tags(&self, max: usize) -> String {
        self.tags
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .take(max)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// "Germany · jazz, swing · MP3 128 kbps" — whatever is known.
    pub fn subtitle(&self) -> String {
        let mut parts = Vec::new();
        if !self.country.is_empty() {
            parts.push(self.country.clone());
        } else if !self.countrycode.is_empty() {
            parts.push(self.countrycode.clone());
        }
        let tags = self.short_tags(3);
        if !tags.is_empty() {
            parts.push(tags);
        }
        let quality = match (self.codec.is_empty(), self.bitrate) {
            (false, 0) => self.codec.clone(),
            (false, b) => format!("{} {b} kbps", self.codec),
            (true, 0) => String::new(),
            (true, b) => format!("{b} kbps"),
        };
        if !quality.is_empty() {
            parts.push(quality);
        }
        if self.custom && parts.is_empty() {
            // The address itself (the name often is just its host).
            let bare = self
                .url
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            parts.push(bare.to_string());
        }
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_prefers_the_directory_id() {
        let mut s = Station {
            uuid: "ea8059be".into(),
            url: "https://x/a".into(),
            ..Default::default()
        };
        assert_eq!(s.key(), "rb:ea8059be");
        s.uuid.clear();
        assert_eq!(s.key(), "url:https://x/a");
    }

    #[test]
    fn subtitle_lists_what_is_known() {
        let s = Station {
            country: "Germany".into(),
            tags: "jazz, swing,,lounge,bebop".into(),
            codec: "MP3".into(),
            bitrate: 128,
            ..Default::default()
        };
        assert_eq!(s.subtitle(), "Germany · jazz, swing, lounge · MP3 128 kbps");
        let custom = Station {
            url: "https://stream.example:8000/live".into(),
            custom: true,
            ..Default::default()
        };
        assert_eq!(custom.subtitle(), "stream.example:8000/live");
    }
}
