//! yt-dlp listings: the command line for a search / channel page and the
//! parser for its `--flat-playlist --dump-single-json` answer.
//!
//! Flat listings are cheap (one page request, no per-video extraction)
//! and carry everything a row needs: id, title, channel, duration, live
//! state, thumbnails. Image URLs are only kept for YouTube's own image
//! hosts ([`image_url`]); a video's thumbnail is rebuilt as the small,
//! stable `i.ytimg.com/vi/<id>/mqdefault.jpg` instead of the signed
//! `hq720` URLs the listing carries.

use serde_json::Value;

/// What one listing row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Video,
    Channel,
    Playlist,
}

/// One row of a listing.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: Kind,
    pub id: String,
    pub title: String,
    pub channel: String,
    pub channel_id: String,
    /// Seconds.
    pub duration: Option<f64>,
    pub live: bool,
    /// Channel rows: subscriber count.
    pub followers: Option<u64>,
    /// Allow-listed image URL (video thumbnail / channel avatar).
    pub image: Option<String>,
}

pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(id_byte)
}

pub fn valid_channel_id(id: &str) -> bool {
    id.len() == 24 && id.starts_with("UC") && id.bytes().all(id_byte)
}

fn valid_playlist_id(id: &str) -> bool {
    (13..=64).contains(&id.len()) && id.bytes().all(id_byte)
}

fn id_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Collapse whitespace runs (titles carry stray newlines / NBSPs).
pub fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Keep `raw` only on YouTube's image hosts (`i.ytimg.com` /
/// `img.youtube.com` for videos, `yt3.*` for channel avatars), upgrading
/// protocol-relative `//…` URLs to https. Anything else is dropped — the
/// launcher never fetches from a host a listing names.
pub fn image_url(raw: &str, channel: bool) -> Option<String> {
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let url = match raw.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => raw.to_string(),
    };
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    let allowed = if channel {
        matches!(host, "yt3.ggpht.com" | "yt3.googleusercontent.com")
    } else {
        matches!(host, "i.ytimg.com" | "img.youtube.com")
    };
    (allowed && !path.is_empty()).then_some(url)
}

/// The small, stable thumbnail of a video.
pub fn video_thumbnail(id: &str) -> String {
    format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")
}

/// Smallest allow-listed avatar at least 64px wide (they come
/// smallest-first), else the largest one.
fn channel_avatar(item: &Value) -> Option<String> {
    let thumbs = item.get("thumbnails")?.as_array()?;
    let urls: Vec<(u64, String)> = thumbs
        .iter()
        .filter_map(|t| {
            let url = image_url(t.get("url")?.as_str()?, true)?;
            Some((t.get("width").and_then(Value::as_u64).unwrap_or(0), url))
        })
        .collect();
    urls.iter()
        .find(|(w, _)| *w >= 64)
        .or(urls.last())
        .map(|(_, u)| u.clone())
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Parse a flat listing. Channel pages don't repeat the channel on every
/// entry, so the page's own `channel` / `channel_id` fill the gaps.
pub fn parse_listing(json: &str) -> Result<Vec<Entry>, String> {
    let page: Value =
        serde_json::from_str(json).map_err(|e| format!("unexpected yt-dlp output: {e}"))?;
    let entries = page
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("unexpected yt-dlp output: no entries")?;
    let page_channel = clean(str_field(&page, "channel"));
    let page_channel_id = str_field(&page, "channel_id").to_string();
    let mut out: Vec<Entry> = Vec::new();
    for item in entries {
        let id = str_field(item, "id");
        let ie = str_field(item, "ie_key");
        let url = str_field(item, "url");
        let mut channel = clean(str_field(item, "channel"));
        if channel.is_empty() {
            channel = clean(str_field(item, "uploader"));
        }
        let mut channel_id = str_field(item, "channel_id").to_string();
        if channel.is_empty() {
            channel = page_channel.clone();
        }
        if !valid_channel_id(&channel_id) && valid_channel_id(&page_channel_id) {
            channel_id = page_channel_id.clone();
        }
        let title = clean(str_field(item, "title"));
        let entry = if ie == "Youtube" && valid_video_id(id) {
            Entry {
                kind: Kind::Video,
                id: id.into(),
                title: if title.is_empty() {
                    "Untitled video".into()
                } else {
                    title
                },
                channel,
                channel_id,
                duration: item
                    .get("duration")
                    .and_then(Value::as_f64)
                    .filter(|d| *d > 0.0),
                live: str_field(item, "live_status") == "is_live",
                followers: None,
                image: Some(video_thumbnail(id)),
            }
        } else if ie == "YoutubeTab" && valid_channel_id(id) {
            Entry {
                kind: Kind::Channel,
                id: id.into(),
                title: if title.is_empty() {
                    channel.clone()
                } else {
                    title
                },
                channel_id: id.into(),
                channel,
                duration: None,
                live: false,
                followers: item.get("channel_follower_count").and_then(Value::as_u64),
                image: channel_avatar(item),
            }
        } else if ie == "YoutubeTab" && url.contains("/playlist?list=") && valid_playlist_id(id) {
            Entry {
                kind: Kind::Playlist,
                id: id.into(),
                title,
                channel,
                channel_id,
                duration: None,
                live: false,
                followers: None,
                image: None,
            }
        } else {
            continue; // shorts shelves, mixes, unknown shapes
        };
        if !out.iter().any(|e| e.kind == entry.kind && e.id == entry.id) {
            out.push(entry);
        }
    }
    Ok(out)
}

/// `application/x-www-form-urlencoded` component encoding.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// What to list.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Request {
    /// Site search (videos, channels, playlists).
    Search(String),
    /// A channel's latest uploads.
    Channel(String),
    /// Search inside a channel.
    ChannelSearch(String, String),
}

impl Request {
    pub fn url(&self) -> String {
        match self {
            Request::Search(q) => format!(
                "https://www.youtube.com/results?search_query={}",
                url_encode(q)
            ),
            Request::Channel(id) => format!("https://www.youtube.com/channel/{id}/videos"),
            Request::ChannelSearch(id, q) => format!(
                "https://www.youtube.com/channel/{id}/search?query={}",
                url_encode(q)
            ),
        }
    }
}

/// yt-dlp argv (without the binary) for the first `limit` rows of `req`.
/// `--ignore-config` keeps a user's download-oriented config out of it;
/// short socket timeout, no retries — a stalled search is retried by the
/// user typing, not by yt-dlp.
pub fn ytdlp_args(req: &Request, limit: usize) -> Vec<String> {
    [
        "--ignore-config",
        "--no-cache-dir",
        "--no-warnings",
        "--flat-playlist",
        "--dump-single-json",
        "--socket-timeout",
        "5",
        "--retries",
        "0",
        "--extractor-retries",
        "0",
        "--playlist-items",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([format!("1:{limit}"), "--".into(), req.url()])
    .collect()
}

/// `m:ss` / `h:mm:ss`.
pub fn format_duration(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// `70.4K` / `1.2M` subscriber counts.
pub fn format_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1e3).replace(".0K", "K"),
        _ => format!("{:.1}M", n as f64 / 1e6).replace(".0M", "M"),
    }
}

/// The id of a pasted YouTube video URL (watch / youtu.be / shorts /
/// live / embed), for the `yt <url>` rows.
pub fn video_id_from_url(url: &str) -> Option<String> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.strip_prefix("www.").unwrap_or(host);
    let id = match host {
        "youtu.be" => path.split(['?', '#', '/']).next(),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" => {
            match path.strip_prefix("watch?") {
                Some(q) => q
                    .split(['&', '#'])
                    .filter_map(|kv| kv.split_once('='))
                    .find(|(k, _)| *k == "v")
                    .map(|(_, v)| v),
                None => ["shorts/", "live/", "embed/"].iter().find_map(|p| {
                    path.strip_prefix(p)
                        .and_then(|r| r.split(['?', '#', '/']).next())
                }),
            }
        }
        _ => None,
    }?;
    valid_video_id(id).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `yt-dlp --flat-playlist --dump-single-json` of a site search,
    /// captured on this machine (yt-dlp 2026.10.26).
    const SEARCH: &str = include_str!("../../../tests/fixtures/yt-search.json");
    /// The same for a channel's `/videos` tab.
    const CHANNEL: &str = include_str!("../../../tests/fixtures/yt-channel.json");

    #[test]
    fn search_yields_videos_channels_and_playlists() {
        let rows = parse_listing(SEARCH).unwrap();
        let kinds: Vec<Kind> = rows.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&Kind::Video));
        assert!(kinds.contains(&Kind::Channel));
        assert!(kinds.contains(&Kind::Playlist));
        let v = rows.iter().find(|r| r.id == "5C_HPTJg5ek").unwrap();
        assert_eq!(v.title, "Rust in 100 Seconds");
        assert_eq!(v.channel, "Fireship");
        assert_eq!(v.channel_id, "UCsBjURrPoezykLs9EqgamOA");
        assert_eq!(v.duration, Some(149.0));
        assert_eq!(
            v.image.as_deref(),
            Some("https://i.ytimg.com/vi/5C_HPTJg5ek/mqdefault.jpg")
        );
        let c = rows.iter().find(|r| r.kind == Kind::Channel).unwrap();
        assert_eq!(c.id, "UCaYhcUwRBNscFNUKTjgPFiA");
        assert_eq!(c.title, "Rust Programming Language");
        assert_eq!(c.followers, Some(70400));
        // The protocol-relative 88px avatar, upgraded to https.
        assert!(
            c.image
                .as_deref()
                .unwrap()
                .starts_with("https://yt3.ggpht.com/")
        );
    }

    #[test]
    fn channel_page_fills_in_the_channel() {
        let rows = parse_listing(CHANNEL).unwrap();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.kind == Kind::Video));
        assert!(
            rows.iter()
                .all(|r| r.channel == "Rust Programming Language")
        );
        assert!(
            rows.iter()
                .all(|r| r.channel_id == "UCaYhcUwRBNscFNUKTjgPFiA")
        );
    }

    #[test]
    fn junk_and_hostile_values_are_dropped() {
        assert!(parse_listing("not json").is_err());
        assert!(parse_listing(r#"{"id":"x"}"#).is_err());
        let rows = parse_listing(
            r#"{"entries":[
                {"ie_key":"Youtube","id":"short"},
                {"ie_key":"Youtube","id":"5C_HPTJg5ek","title":"  a\n b  "},
                {"ie_key":"Youtube","id":"5C_HPTJg5ek","title":"dup"},
                {"ie_key":"YoutubeTab","id":"UCx","url":"https://www.youtube.com/channel/UCx"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "a b");
        assert_eq!(image_url("https://evil.test/x.jpg", false), None);
        assert_eq!(image_url("https://i.ytimg.com/vi/x/a.jpg", true), None);
        assert_eq!(image_url("https://i.ytimg.com/ x", false), None);
        assert_eq!(
            image_url("//yt3.ggpht.com/abc=s88", true).as_deref(),
            Some("https://yt3.ggpht.com/abc=s88")
        );
    }

    #[test]
    fn request_urls_and_argv() {
        assert_eq!(
            Request::Search("rust & go".into()).url(),
            "https://www.youtube.com/results?search_query=rust+%26+go"
        );
        assert_eq!(
            Request::ChannelSearch("UCaYhcUwRBNscFNUKTjgPFiA".into(), "async".into()).url(),
            "https://www.youtube.com/channel/UCaYhcUwRBNscFNUKTjgPFiA/search?query=async"
        );
        let args = ytdlp_args(&Request::Channel("UCaYhcUwRBNscFNUKTjgPFiA".into()), 20);
        assert_eq!(args[0], "--ignore-config");
        let n = args.len();
        assert_eq!(args[n - 3], "1:20");
        assert_eq!(args[n - 2], "--");
        assert!(args[n - 1].ends_with("/videos"));
    }

    #[test]
    fn formatting() {
        assert_eq!(format_duration(149.0), "2:29");
        assert_eq!(format_duration(3725.0), "1:02:05");
        assert_eq!(format_count(70400), "70.4K");
        assert_eq!(format_count(2_000_000), "2M");
        assert_eq!(format_count(999), "999");
    }

    #[test]
    fn pasted_urls() {
        let id = Some("5C_HPTJg5ek".to_string());
        assert_eq!(
            video_id_from_url("https://www.youtube.com/watch?v=5C_HPTJg5ek&t=3"),
            id
        );
        assert_eq!(video_id_from_url("https://youtu.be/5C_HPTJg5ek"), id);
        assert_eq!(
            video_id_from_url("https://m.youtube.com/shorts/5C_HPTJg5ek"),
            id
        );
        assert_eq!(
            video_id_from_url("https://example.com/watch?v=5C_HPTJg5ek"),
            None
        );
        assert_eq!(video_id_from_url("rust lang"), None);
    }
}
