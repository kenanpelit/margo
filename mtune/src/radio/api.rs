// SPDX-License-Identifier: GPL-3.0-or-later

//! radio-browser.info: station search and the country list.
//!
//! The query/parse halves are pure (unit-tested against captured API
//! answers); [`search`], [`countries`] and [`count_click`] block and must
//! run off the GTK thread (see [`crate::net::off_main`]).

use serde_json::Value;

use super::Station;
use crate::net::{self, NetError};

/// Public API servers, tried in order on a transport failure. `all.` is the
/// DNS round-robin over every mirror.
const MIRRORS: &[&str] = &[
    "https://de1.api.radio-browser.info",
    "https://de2.api.radio-browser.info",
    "https://fi1.api.radio-browser.info",
    "https://all.api.radio-browser.info",
];

/// Stations fetched per page.
pub const PAGE: u32 = 50;

/// Genres offered in the browser (radio-browser tags are free-form; its own
/// top-tags list is dominated by noise such as "music", "radio" and "fm").
pub const GENRES: &[&str] = &[
    "pop",
    "rock",
    "jazz",
    "classical",
    "electronic",
    "dance",
    "hip hop",
    "ambient",
    "lofi",
    "chillout",
    "metal",
    "indie",
    "folk",
    "country",
    "blues",
    "soul",
    "reggae",
    "latin",
    "oldies",
    "80s",
    "90s",
    "news",
    "talk",
    "sports",
];

/// Result ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    #[default]
    Votes,
    /// Most played recently.
    Trending,
    Name,
}

impl Order {
    fn api(self) -> (&'static str, bool) {
        match self {
            Order::Votes => ("votes", true),
            Order::Trending => ("clicktrend", true),
            Order::Name => ("name", false),
        }
    }
}

/// One station search. Empty filters are left out of the request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    pub name: String,
    /// ISO 3166-1 alpha-2, e.g. `TR`.
    pub countrycode: String,
    pub tag: String,
    pub order: Order,
    pub offset: u32,
}

impl Query {
    /// The `/json/stations/search` query pairs.
    pub fn pairs(&self) -> Vec<(&'static str, String)> {
        let mut q = Vec::new();
        let name = self.name.trim();
        if !name.is_empty() {
            q.push(("name", name.to_string()));
        }
        let cc = self.countrycode.trim();
        if !cc.is_empty() {
            q.push(("countrycode", cc.to_ascii_uppercase()));
        }
        let tag = self.tag.trim();
        if !tag.is_empty() {
            q.push(("tag", tag.to_lowercase()));
            q.push(("tagExact", "false".to_string()));
        }
        let (order, reverse) = self.order.api();
        q.push(("order", order.to_string()));
        q.push(("reverse", reverse.to_string()));
        q.push(("hidebroken", "true".to_string()));
        q.push(("limit", PAGE.to_string()));
        q.push(("offset", self.offset.to_string()));
        q
    }

    /// The same search, one page further.
    pub fn next_page(&self) -> Self {
        Self {
            offset: self.offset + PAGE,
            ..self.clone()
        }
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

/// Station objects -> [`Station`]s. Entries without a usable http(s) stream
/// or a name are dropped; duplicates of the same stream are folded.
pub fn parse_stations(v: &Value) -> Vec<Station> {
    let mut seen = std::collections::HashSet::new();
    v.as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|e| {
            let resolved = s(e, "url_resolved");
            let raw = if resolved.is_empty() {
                s(e, "url")
            } else {
                resolved
            };
            let url = net::validate_http_url(&raw)?;
            let name = s(e, "name");
            if name.is_empty() || !seen.insert(url.clone()) {
                return None;
            }
            let favicon = net::validate_http_url(&s(e, "favicon")).unwrap_or_default();
            Some(Station {
                uuid: s(e, "stationuuid"),
                name,
                url,
                favicon,
                tags: s(e, "tags"),
                country: s(e, "country"),
                countrycode: s(e, "countrycode"),
                codec: s(e, "codec"),
                bitrate: n(e, "bitrate").min(u32::MAX as u64) as u32,
                votes: n(e, "votes"),
                hls: n(e, "hls") == 1,
                custom: false,
            })
        })
        .collect()
}

/// A country with stations: `(iso code, display name, station count)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Country {
    pub code: String,
    pub name: String,
    pub stations: u64,
}

/// `/json/countries` -> countries with at least one station, most stations
/// first.
pub fn parse_countries(v: &Value) -> Vec<Country> {
    let mut out: Vec<Country> = v
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|e| {
            let code = s(e, "iso_3166_1").to_ascii_uppercase();
            let name = s(e, "name");
            let stations = n(e, "stationcount");
            (code.len() == 2 && !name.is_empty() && stations > 0).then_some(Country {
                code,
                name,
                stations,
            })
        })
        .collect();
    out.sort_by(|a, b| b.stations.cmp(&a.stations).then(a.name.cmp(&b.name)));
    out
}

/// GET `path` on the first mirror that answers. A status error from a live
/// mirror is final (another mirror would say the same).
fn get(path: &str, query: &[(&str, String)]) -> Result<Value, NetError> {
    let mut last = NetError::Unreachable("no mirror".into());
    for base in MIRRORS {
        match net::get_json(&format!("{base}{path}"), query) {
            Ok(v) => return Ok(v),
            Err(e @ NetError::Unreachable(_)) => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// Blocking: one page of stations.
pub fn search(query: &Query) -> Result<Vec<Station>, NetError> {
    get("/json/stations/search", &query.pairs()).map(|v| parse_stations(&v))
}

/// Blocking: every country that has stations.
pub fn countries() -> Result<Vec<Country>, NetError> {
    get(
        "/json/countries",
        &[
            ("hidebroken", "true".to_string()),
            ("order", "stationcount".to_string()),
            ("reverse", "true".to_string()),
        ],
    )
    .map(|v| parse_countries(&v))
}

/// Blocking, best effort: tell the directory a station was played (its
/// popularity ranking runs on these clicks). Custom stations are skipped.
pub fn count_click(uuid: &str) {
    if uuid.is_empty() || !uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return;
    }
    let _ = get(&format!("/json/url/{uuid}"), &[]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let raw = match name {
            "search" => include_str!("fixtures/rb_search.json"),
            _ => include_str!("fixtures/rb_countries.json"),
        };
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn parses_a_real_search_answer() {
        let st = parse_stations(&fixture("search"));
        assert_eq!(st.len(), 3);
        let first = &st[0];
        assert_eq!(first.name, "Adroit Jazz Underground");
        assert_eq!(first.uuid, "ea8059be-d119-4de3-b27b-0d9bd6aedb17");
        assert_eq!(first.url, "https://icecast.walmradio.com:8443/jazz");
        assert_eq!(first.favicon, "https://icecast.walmradio.com:8443/jazz.jpg");
        assert_eq!(first.countrycode, "US");
        assert_eq!((first.codec.as_str(), first.bitrate), ("MP3", 320));
        assert_eq!(first.votes, 184895);
        assert!(!first.hls && !first.custom);
        assert_eq!(st[1].url, "http://jking.cdnstream1.com/b22139_128mp3");
    }

    #[test]
    fn drops_unplayable_and_duplicate_entries() {
        let v: Value = serde_json::json!([
            {"name": "ok", "url": "https://a/x", "url_resolved": ""},
            {"name": "dup", "url_resolved": "https://a/x"},
            {"name": "", "url_resolved": "https://b/x"},
            {"name": "rtsp", "url_resolved": "rtsp://c/x"},
            {"name": "hls", "url_resolved": "https://d/x.m3u8", "hls": 1,
             "favicon": "data:image/png;base64,AAAA"},
        ]);
        let st = parse_stations(&v);
        let names: Vec<_> = st.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["ok", "hls"]);
        assert!(st[1].hls);
        assert_eq!(st[1].favicon, "", "only http(s) logos are fetched");
        assert!(parse_stations(&serde_json::json!({"error": 1})).is_empty());
    }

    #[test]
    fn parses_real_countries_most_stations_first() {
        let c = parse_countries(&fixture("countries"));
        assert!(c.len() > 100, "{}", c.len());
        assert_eq!(c[0].code, "US");
        assert_eq!(c[0].name, "The United States Of America");
        assert!(c.windows(2).all(|w| w[0].stations >= w[1].stations));
        assert!(c.iter().any(|x| x.code == "TR"));
    }

    #[test]
    fn query_leaves_out_empty_filters() {
        let q = Query::default();
        let keys: Vec<_> = q.pairs().iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["order", "reverse", "hidebroken", "limit", "offset"]);
        let q = Query {
            name: "  jazz fm ".into(),
            countrycode: "tr".into(),
            tag: "Hip Hop".into(),
            order: Order::Name,
            offset: 0,
        };
        let p = q.pairs();
        let get = |k: &str| p.iter().find(|(x, _)| *x == k).map(|(_, v)| v.as_str());
        assert_eq!(get("name"), Some("jazz fm"));
        assert_eq!(get("countrycode"), Some("TR"));
        assert_eq!(get("tag"), Some("hip hop"));
        assert_eq!(get("order"), Some("name"));
        assert_eq!(get("reverse"), Some("false"));
        assert_eq!(get("hidebroken"), Some("true"));
    }

    #[test]
    fn votes_and_trending_sort_descending() {
        for (order, api) in [(Order::Votes, "votes"), (Order::Trending, "clicktrend")] {
            let p = Query {
                order,
                ..Default::default()
            }
            .pairs();
            assert!(p.contains(&("order", api.to_string())));
            assert!(p.contains(&("reverse", "true".to_string())));
        }
    }

    #[test]
    fn next_page_moves_the_offset_only() {
        let q = Query {
            tag: "jazz".into(),
            ..Default::default()
        };
        let n = q.next_page().next_page();
        assert_eq!(n.offset, 2 * PAGE);
        assert_eq!(n.tag, "jazz");
    }
}
