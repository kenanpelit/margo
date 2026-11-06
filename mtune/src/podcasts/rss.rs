// SPDX-License-Identifier: GPL-3.0-or-later

//! A small, forgiving RSS 2.0 podcast-feed reader (quick-xml events, no
//! DOM): the show's title/author/artwork and its episodes with their audio
//! enclosures. Items without audio are skipped.

use quick_xml::{Reader, events::Event};

use super::Episode;
use crate::net::{self, NetError};

/// A parsed feed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Feed {
    pub title: String,
    pub author: String,
    pub image: String,
    pub episodes: Vec<Episode>,
}

/// Longest summary kept per episode (chars).
const SUMMARY_CHARS: usize = 280;

/// `"01:05:44"`, `"65:44"`, `"3944"`, `"3944.6"` -> seconds (0 when unreadable).
pub fn parse_duration(raw: &str) -> u64 {
    let raw = raw.trim();
    if raw.is_empty() {
        return 0;
    }
    if !raw.contains(':') {
        return raw.parse::<f64>().map(|f| f.max(0.0) as u64).unwrap_or(0);
    }
    let mut total = 0u64;
    for part in raw.split(':') {
        let Ok(n) = part.trim().parse::<f64>() else {
            return 0;
        };
        total = total * 60 + n.max(0.0) as u64;
    }
    total
}

/// `"Fri, 24 Apr 2026 07:00:00 +0000"` -> `"24 Apr 2026"`; anything else
/// is returned trimmed.
pub fn short_date(raw: &str) -> String {
    let raw = raw.trim();
    let body = raw.split_once(',').map(|(_, b)| b).unwrap_or(raw);
    let parts: Vec<&str> = body.split_whitespace().take(3).collect();
    if parts.len() == 3 && parts[0].parse::<u8>().is_ok() && parts[2].parse::<u16>().is_ok() {
        format!(
            "{} {} {}",
            parts[0].trim_start_matches('0'),
            parts[1],
            parts[2]
        )
    } else {
        raw.to_string()
    }
}

/// HTML show notes -> one line of plain text, at most `SUMMARY_CHARS`.
pub fn plain_summary(html: &str) -> String {
    let mut out = String::with_capacity(html.len().min(1024));
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= SUMMARY_CHARS {
        return collapsed;
    }
    let cut: String = collapsed.chars().take(SUMMARY_CHARS).collect();
    format!("{}…", cut.trim_end())
}

fn entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        _ => return None,
    })
}

fn attr(e: &quick_xml::events::BytesStart<'_>, key: &str) -> String {
    e.try_get_attribute(key)
        .ok()
        .flatten()
        .and_then(|a| a.unescape_value().ok().map(|v| v.trim().to_string()))
        .unwrap_or_default()
}

fn is_audio(mime: &str, url: &str) -> bool {
    let mime = mime.to_ascii_lowercase();
    if mime.starts_with("audio/") {
        return true;
    }
    if mime.starts_with("video/") || mime.starts_with("image/") || mime.contains("pdf") {
        return false;
    }
    // Many feeds omit or mislabel the type: judge by extension.
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    [
        ".mp3", ".m4a", ".aac", ".ogg", ".oga", ".opus", ".flac", ".wav",
    ]
    .iter()
    .any(|ext| path.ends_with(ext))
        || mime.is_empty() && !path.is_empty()
}

/// Parse a feed body.
pub fn parse(xml: &str) -> Result<Feed, NetError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut feed = Feed::default();
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut item: Option<Episode> = None;
    let mut item_desc = String::new();
    let mut item_summary = String::new();
    let mut saw_rss = false;

    loop {
        let ev = match reader.read_event() {
            Ok(ev) => ev,
            Err(e) => {
                // A truncated or slightly broken feed: keep what parsed.
                if feed.episodes.is_empty() {
                    return Err(NetError::Malformed(format!("feed XML: {e}")));
                }
                log::warn!("mtune: podcast feed stopped parsing early: {e}");
                break;
            }
        };
        match ev {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                if path.is_empty() {
                    saw_rss = name == "rss" || name.ends_with(":RDF");
                }
                text.clear();
                if name == "item" {
                    item = Some(Episode::default());
                    item_desc.clear();
                    item_summary.clear();
                }
                handle_attrs(&name, &e, &path, item.as_mut(), &mut feed);
                path.push(name);
            }
            Event::Empty(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                handle_attrs(&name, &e, &path, item.as_mut(), &mut feed);
            }
            Event::Text(t) => {
                if let Ok(s) = t.decode() {
                    text.push_str(&s);
                }
            }
            Event::CData(c) => {
                if let Ok(s) = c.decode() {
                    text.push_str(&s);
                }
            }
            Event::GeneralRef(r) => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    text.push(c);
                } else if let Ok(name) = r.decode()
                    && let Some(c) = entity(&name)
                {
                    text.push(c);
                }
            }
            Event::End(_) => {
                let Some(name) = path.pop() else { continue };
                let value = std::mem::take(&mut text);
                let value = value.trim();
                let parent = path.last().map(String::as_str).unwrap_or("");
                if let Some(ep) = item.as_mut() {
                    match name.as_str() {
                        "title" if parent == "item" => ep.title = collapse(value),
                        "guid" => ep.guid = value.to_string(),
                        "pubDate" => ep.date = short_date(value),
                        "itunes:duration" => ep.duration = parse_duration(value),
                        "description" => item_desc = value.to_string(),
                        "itunes:summary" | "itunes:subtitle" if item_summary.is_empty() => {
                            item_summary = value.to_string()
                        }
                        "item" => {
                            if let Some(mut ep) = item.take() {
                                let notes = if item_summary.is_empty() {
                                    &item_desc
                                } else {
                                    &item_summary
                                };
                                ep.summary = plain_summary(notes);
                                if !ep.url.is_empty() {
                                    if ep.guid.is_empty() {
                                        ep.guid = ep.url.clone();
                                    }
                                    if ep.title.is_empty() {
                                        ep.title = ep.date.clone();
                                    }
                                    feed.episodes.push(ep);
                                }
                            }
                        }
                        _ => {}
                    }
                } else if parent == "channel" {
                    match name.as_str() {
                        "title" => feed.title = collapse(value),
                        "itunes:author" => feed.author = collapse(value),
                        _ => {}
                    }
                } else if name == "url"
                    && parent == "image"
                    && feed.image.is_empty()
                    && let Some(u) = net::validate_http_url(value)
                {
                    feed.image = u;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !saw_rss {
        return Err(NetError::Malformed("not an RSS feed".into()));
    }
    Ok(feed)
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Attribute-carrying elements: `<enclosure>`, `<itunes:image>`.
fn handle_attrs(
    name: &str,
    e: &quick_xml::events::BytesStart<'_>,
    path: &[String],
    item: Option<&mut Episode>,
    feed: &mut Feed,
) {
    match (name, item) {
        ("enclosure", Some(ep)) if ep.url.is_empty() => {
            let url = attr(e, "url");
            let mime = attr(e, "type");
            if let Some(url) = net::validate_http_url(&url)
                && is_audio(&mime, &url)
            {
                ep.url = url;
                ep.mime = mime;
            }
        }
        ("itunes:image", Some(ep)) => {
            if let Some(u) = net::validate_http_url(&attr(e, "href")) {
                ep.image = u;
            }
        }
        ("itunes:image", None) if path.last().map(String::as_str) == Some("channel") => {
            // Prefer the iTunes square artwork over `<image><url>`.
            if let Some(u) = net::validate_http_url(&attr(e, "href")) {
                feed.image = u;
            }
        }
        _ => {}
    }
}

/// Blocking: fetch and parse a feed.
pub fn fetch(feed_url: &str) -> Result<Feed, NetError> {
    let url = net::validate_http_url(feed_url)
        .ok_or_else(|| NetError::Malformed("not an http(s) feed URL".into()))?;
    parse(&net::get_text(&url, net::FEED_CAP)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = include_str!("fixtures/podcast_feed.xml");

    #[test]
    fn parses_a_real_simplecast_feed() {
        // https://feeds.simplecast.com/6owr3EpB, trimmed to three items.
        let f = parse(FEED).unwrap();
        assert_eq!(f.title, "The 99% Invisible Breakdown");
        assert_eq!(f.author, "99% Invisible");
        assert!(
            f.image
                .contains("99picoverthe_power_broker3000x3000final.jpg"),
            "{}",
            f.image
        );
        assert_eq!(f.episodes.len(), 3);
        let e = &f.episodes[0];
        assert_eq!(e.guid, "621e51e9-2a46-4d5a-a96f-97e42a383743");
        assert_eq!(e.title, "The Constitution #9: Alondra Nelson");
        assert_eq!(e.date, "24 Apr 2026");
        assert_eq!(e.duration, 3944);
        assert_eq!(e.mime, "audio/mpeg");
        // `&amp;` in the attribute is unescaped.
        assert!(
            e.url.starts_with("https://dts.podtrac.com/redirect.mp3/"),
            "{}",
            e.url
        );
        assert!(e.url.contains("aid=rss_feed&awCollectionId="), "{}", e.url);
        // The plain itunes:summary wins over the HTML description.
        assert!(
            e.summary
                .starts_with("Roman and Elizabeth discuss Article VI"),
            "{}",
            e.summary
        );
        assert!(e.image.ends_with("aid=rss_feed"), "{}", e.image);
        assert_eq!(f.episodes[1].title, "The Constitution #8: Jill Lepore");
    }

    #[test]
    fn items_without_audio_are_skipped_and_gaps_filled() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel>
<title>Show &amp; Tell</title>
<image><url>https://x/cover.png</url><title>ignored</title></image>
<item><title>Blog post</title><link>https://x/p</link></item>
<item><title>Video</title><enclosure url="https://x/v.mp4" type="video/mp4"/></item>
<item><pubDate>Mon, 02 Mar 2026 10:00:00 GMT</pubDate>
  <enclosure url="https://x/e.mp3?a=1&amp;b=2" length="1"/>
  <description><![CDATA[<p>Hello <b>world</b></p>]]></description></item>
</channel></rss>"#;
        let f = parse(xml).unwrap();
        assert_eq!(f.title, "Show & Tell");
        assert_eq!(f.image, "https://x/cover.png");
        assert_eq!(f.episodes.len(), 1);
        let e = &f.episodes[0];
        assert_eq!(e.url, "https://x/e.mp3?a=1&b=2");
        assert_eq!(e.guid, e.url, "no <guid>: the URL is the identity");
        assert_eq!(e.title, "2 Mar 2026", "no <title>: the date stands in");
        assert_eq!(e.summary, "Hello world");
    }

    #[test]
    fn non_rss_and_garbage_are_errors() {
        assert!(parse("<html><body>404</body></html>").is_err());
        assert!(parse("not xml at all").is_err());
        assert!(parse(r#"<feed xmlns="http://www.w3.org/2005/Atom"></feed>"#).is_err());
    }

    #[test]
    fn durations_in_every_feed_spelling() {
        assert_eq!(parse_duration("01:05:44"), 3944);
        assert_eq!(parse_duration("65:44"), 3944);
        assert_eq!(parse_duration("3944"), 3944);
        assert_eq!(parse_duration(" 3944.6 "), 3944);
        assert_eq!(parse_duration(""), 0);
        assert_eq!(parse_duration("about an hour"), 0);
        assert_eq!(parse_duration("1:xx"), 0);
    }

    #[test]
    fn dates_shorten_to_day_month_year() {
        assert_eq!(short_date("Fri, 24 Apr 2026 07:00:00 +0000"), "24 Apr 2026");
        assert_eq!(short_date("05 Jan 2025 10:00:00 GMT"), "5 Jan 2025");
        assert_eq!(short_date("2026-04-24"), "2026-04-24");
    }

    #[test]
    fn summaries_are_plain_and_bounded() {
        let long = format!("<p>{}</p>", "word ".repeat(200));
        let s = plain_summary(&long);
        assert!(s.chars().count() <= SUMMARY_CHARS + 1);
        assert!(s.ends_with('…'));
        assert_eq!(plain_summary("a<br/>b"), "a b");
    }
}
