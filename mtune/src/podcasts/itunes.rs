// SPDX-License-Identifier: GPL-3.0-or-later

//! Podcast search through the iTunes Search API (no key needed; it also
//! returns each show's RSS feed URL).

use serde_json::Value;

use super::Podcast;
use crate::net::{self, NetError};

const ENDPOINT: &str = "https://itunes.apple.com/search";
pub const LIMIT: u32 = 30;

/// The search query pairs for `term`.
pub fn pairs(term: &str) -> Vec<(&'static str, String)> {
    vec![
        ("media", "podcast".to_string()),
        ("entity", "podcast".to_string()),
        ("limit", LIMIT.to_string()),
        ("term", term.trim().to_string()),
    ]
}

/// iTunes results -> shows that have an http(s) feed (some entries are
/// Apple-only and carry none).
pub fn parse(v: &Value) -> Vec<Podcast> {
    let s = |e: &Value, k: &str| {
        e.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    v.get("results")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|e| {
            let feed_url = net::validate_http_url(&s(e, "feedUrl"))?;
            let title = Some(s(e, "collectionName"))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| s(e, "trackName"));
            if title.is_empty() {
                return None;
            }
            let artwork = ["artworkUrl600", "artworkUrl100", "artworkUrl60"]
                .iter()
                .find_map(|k| net::validate_http_url(&s(e, k)))
                .unwrap_or_default();
            Some(Podcast {
                title,
                author: s(e, "artistName"),
                feed_url,
                artwork,
                genre: s(e, "primaryGenreName"),
                episodes: e
                    .get("trackCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(u32::MAX as u64) as u32,
            })
        })
        .collect()
}

/// Blocking: search shows by name / author.
pub fn search(term: &str) -> Result<Vec<Podcast>, NetError> {
    net::get_json(ENDPOINT, &pairs(term)).map(|v| parse(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_real_itunes_answer() {
        let v: Value = serde_json::from_str(include_str!("fixtures/itunes_search.json")).unwrap();
        let p = parse(&v);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].title, "99% Invisible");
        assert_eq!(p[0].author, "Roman Mars");
        assert_eq!(p[0].feed_url, "https://feeds.simplecast.com/BqbsxVfO");
        assert!(p[0].artwork.ends_with("/600x600bb.jpg"), "{}", p[0].artwork);
        assert_eq!(p[0].genre, "Design");
        assert_eq!(p[0].episodes, 848);
        assert_eq!(p[1].feed_url, "https://feeds.simplecast.com/6owr3EpB");
    }

    #[test]
    fn entries_without_a_feed_are_skipped() {
        let v = serde_json::json!({"results": [
            {"collectionName": "Apple only"},
            {"collectionName": "Bad", "feedUrl": "itpc://x/feed"},
            {"trackName": "Fallback name", "feedUrl": "https://x/feed"},
        ]});
        let p = parse(&v);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].title, "Fallback name");
        assert!(parse(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn the_term_is_trimmed_and_scoped_to_podcasts() {
        let p = pairs("  99% invisible ");
        assert!(p.contains(&("term", "99% invisible".to_string())));
        assert!(p.contains(&("media", "podcast".to_string())));
    }
}
