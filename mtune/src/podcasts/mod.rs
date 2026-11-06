// SPDX-License-Identifier: GPL-3.0-or-later

//! Podcasts: find shows ([`itunes`] search or a pasted feed URL), keep
//! subscriptions and per-episode played state / resume position
//! ([`store`]), and read episode lists from RSS ([`rss`]). Episodes play
//! through the normal queue as remote [`crate::audio::Song`]s.

pub mod itunes;
pub mod rss;
pub mod store;

use serde::{Deserialize, Serialize};

/// A show, as found or subscribed. Persisted in `podcasts.json`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Podcast {
    pub title: String,
    #[serde(default)]
    pub author: String,
    /// The RSS feed — the subscription's identity.
    pub feed_url: String,
    #[serde(default)]
    pub artwork: String,
    #[serde(default)]
    pub genre: String,
    /// Episode count from the directory (0 = unknown).
    #[serde(default)]
    pub episodes: u32,
}

/// One episode from a feed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Episode {
    /// `<guid>`, or the enclosure URL when the feed has none.
    pub guid: String,
    pub title: String,
    /// The audio file.
    pub url: String,
    pub mime: String,
    /// Seconds from `<itunes:duration>`; 0 when absent.
    pub duration: u64,
    /// "24 Apr 2026", from `<pubDate>`.
    pub date: String,
    /// Plain-text summary (tags stripped, whitespace collapsed).
    pub summary: String,
    /// Episode-specific artwork, else empty (use the show's).
    pub image: String,
}

impl Episode {
    /// The key for played state and resume position: unique across feeds.
    pub fn key(&self, feed_url: &str) -> String {
        format!("{feed_url}#{}", self.guid)
    }
}

/// "1 h 5 min", "12 min", "45 s" — an episode length for a row.
pub fn format_length(secs: u64) -> String {
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    match (h, m) {
        (0, 0) if secs == 0 => String::new(),
        (0, 0) => format!("{secs} s"),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_read_naturally() {
        assert_eq!(format_length(0), "");
        assert_eq!(format_length(45), "45 s");
        assert_eq!(format_length(754), "12 min");
        assert_eq!(format_length(3600), "1 h");
        assert_eq!(format_length(3944), "1 h 5 min");
    }

    #[test]
    fn episode_keys_are_scoped_to_their_feed() {
        let e = Episode {
            guid: "9".into(),
            ..Default::default()
        };
        assert_ne!(e.key("https://a/feed"), e.key("https://b/feed"));
    }
}
