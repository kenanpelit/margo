//! RSS 2.0 / RSS 1.0 (RDF) / Atom parsing — pure, tested against feeds
//! captured from archlinux.org, github.com and slashdot.org.
//!
//! Only what the notifier needs is read: the feed title and, per item,
//! a stable id, the title and the link. Elements are matched by their
//! local name, so namespace prefixes (`atom:`, `rdf:`, a default
//! namespace) don't matter.

use quick_xml::events::{BytesStart, Event};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Item {
    /// `<guid>` / Atom `<id>` / RDF `rdf:about`, else the link, else the
    /// title: whatever identifies the item across polls.
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) link: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Feed {
    pub(crate) title: String,
    /// In document order (feeds list newest first).
    pub(crate) items: Vec<Item>,
}

/// Feed bodies are decoded by the XML declaration's encoding (or the
/// HTTP `charset`): UTF-8, or Latin-1 / Windows-1252, which some old
/// feeds still use. Anything else is read as UTF-8, lossily.
pub(crate) fn decode_body(bytes: &[u8], http_charset: Option<&str>) -> String {
    let declared = declared_encoding(bytes);
    let enc = declared
        .as_deref()
        .or(http_charset)
        .unwrap_or("utf-8")
        .to_ascii_lowercase();
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match enc.as_str() {
        "iso-8859-1" | "latin1" | "latin-1" | "iso8859-1" | "windows-1252" | "cp1252"
            if std::str::from_utf8(bytes).is_err() =>
        {
            bytes.iter().map(|&b| char::from(b)).collect()
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// `encoding="…"` from `<?xml … ?>`, if the body starts with one.
fn declared_encoding(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(200)];
    let head = String::from_utf8_lossy(head);
    let decl = head.trim_start_matches('\u{feff}').trim_start();
    let decl = decl.strip_prefix("<?xml")?;
    let decl = &decl[..decl.find("?>")?];
    let at = decl.find("encoding")?;
    let rest = decl[at + "encoding".len()..]
        .trim_start()
        .strip_prefix('=')?;
    let rest = rest.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let rest = &rest[1..];
    Some(rest[..rest.find(quote)?].to_string())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Link,
    Id,
}

fn local(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|b| *b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

fn attr(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| local(a.key.as_ref()) == name)
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

/// Whitespace collapsed, at most `max` characters.
fn tidy(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

/// Parse a feed. `Err` when it isn't XML or has neither a channel/feed
/// nor any item.
pub(crate) fn parse_feed(xml: &str) -> Result<Feed, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut feed = Feed::default();
    let mut root_seen = false;
    // Item being read, and which of its fields the text goes to.
    let mut item: Option<Item> = None;
    let mut field: Option<Field> = None;
    let mut text = String::new();
    // Depth inside the current item, so a nested <title> (e.g. Atom
    // <source><title>) doesn't overwrite the item's own.
    let mut depth = 0usize;
    let mut item_depth = 0usize;
    // The feed-level <title> (directly in <channel> / <feed>).
    let mut in_feed_title = false;
    let mut feed_title_depth = 0usize;
    let mut atom_link_rank = 0u8;

    loop {
        let ev = reader
            .read_event()
            .map_err(|e| format!("not a valid feed: {e}"))?;
        match ev {
            Event::Start(e) => {
                depth += 1;
                let name = local(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"rss" | b"RDF" | b"feed" | b"channel" => root_seen = true,
                    _ => {}
                }
                if matches!(name.as_slice(), b"item" | b"entry") && item.is_none() {
                    item = Some(Item {
                        id: attr(&e, b"about").unwrap_or_default(),
                        ..Default::default()
                    });
                    item_depth = depth;
                    atom_link_rank = 0;
                    continue;
                }
                if let Some(it) = item.as_mut() {
                    if depth != item_depth + 1 {
                        continue;
                    }
                    field = match name.as_slice() {
                        b"title" => Some(Field::Title),
                        b"guid" | b"id" => Some(Field::Id),
                        b"link" => {
                            // Atom: <link href="…" rel="…">…</link>.
                            if let Some(href) = attr(&e, b"href") {
                                take_atom_link(it, &e, href, &mut atom_link_rank);
                                None
                            } else {
                                Some(Field::Link)
                            }
                        }
                        _ => None,
                    };
                    text.clear();
                } else if name == b"title" && feed.title.is_empty() && depth <= 3 {
                    in_feed_title = true;
                    feed_title_depth = depth;
                    text.clear();
                }
            }
            Event::Empty(e) => {
                if let Some(it) = item.as_mut()
                    && depth == item_depth
                    && local(e.name().as_ref()) == b"link"
                    && let Some(href) = attr(&e, b"href")
                {
                    take_atom_link(it, &e, href, &mut atom_link_rank);
                }
            }
            Event::Text(t) if field.is_some() || in_feed_title => {
                text.push_str(&t.xml_content().unwrap_or_default());
            }
            Event::CData(t) if field.is_some() || in_feed_title => {
                text.push_str(&String::from_utf8_lossy(&t.into_inner()));
            }
            Event::GeneralRef(r) if field.is_some() || in_feed_title => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    text.push(c);
                } else if let Ok(name) = r.decode() {
                    match quick_xml::escape::resolve_predefined_entity(&name) {
                        Some(s) => text.push_str(s),
                        None => {
                            text.push('&');
                            text.push_str(&name);
                            text.push(';');
                        }
                    }
                }
            }
            Event::End(e) => {
                let name = local(e.name().as_ref()).to_vec();
                if item.is_some()
                    && depth == item_depth
                    && matches!(name.as_slice(), b"item" | b"entry")
                {
                    if let Some(mut it) = item.take() {
                        it.title = tidy(&it.title, 200);
                        it.link = it.link.trim().to_string();
                        it.id = it.id.trim().to_string();
                        if it.id.is_empty() {
                            it.id = if it.link.is_empty() {
                                it.title.clone()
                            } else {
                                it.link.clone()
                            };
                        }
                        if !it.id.is_empty() {
                            feed.items.push(it);
                        }
                    }
                } else if let (Some(f), Some(it)) = (field, item.as_mut())
                    && depth == item_depth + 1
                {
                    let value = std::mem::take(&mut text);
                    match f {
                        Field::Title => it.title = value,
                        Field::Id => it.id = value,
                        Field::Link if it.link.is_empty() => it.link = value,
                        Field::Link => {}
                    }
                    field = None;
                } else if in_feed_title && depth == feed_title_depth {
                    feed.title = tidy(&std::mem::take(&mut text), 120);
                    in_feed_title = false;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen && feed.items.is_empty() {
        return Err("not an RSS or Atom feed".to_string());
    }
    Ok(feed)
}

/// Atom links: `rel="alternate"` (or no rel) beats anything else; the
/// first of equal rank wins.
fn take_atom_link(it: &mut Item, e: &BytesStart<'_>, href: String, rank: &mut u8) {
    let r = match attr(e, b"rel").as_deref() {
        None | Some("alternate") => 2,
        Some("self" | "edit" | "replies") => 0,
        Some(_) => 1,
    };
    if r > *rank || it.link.is_empty() && r > 0 {
        it.link = href;
        *rank = r;
    }
}

/// An item link the notification may open: `http(s)` only (a feed can't
/// make us open `file:` or a custom scheme); a root-relative link is
/// resolved against the feed's origin.
pub(crate) fn openable_link(link: &str, feed_url: &str) -> Option<String> {
    let link = link.trim();
    let lower = link.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return (!link.chars().any(char::is_whitespace)).then(|| link.to_string());
    }
    if link.starts_with('/') && !link.starts_with("//") {
        let origin = origin(feed_url)?;
        return Some(format!("{origin}{link}"));
    }
    None
}

/// `scheme://host[:port]` of an http(s) URL.
pub(crate) fn origin(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if host_end == 0 {
        return None;
    }
    Some(&url[..url.len() - rest.len() + host_end])
}

/// A feed URL we fetch: `http(s)` with a host, no whitespace.
pub(crate) fn valid_feed_url(url: &str) -> bool {
    origin(url.trim()).is_some() && !url.trim().chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARCH: &str = include_str!("testdata/arch-news.xml");
    const GITHUB: &str = include_str!("testdata/github-releases.atom");
    const SLASHDOT: &[u8] = include_bytes!("testdata/slashdot.rdf");

    #[test]
    fn rss2_items_with_entities_and_utf8() {
        let f = parse_feed(ARCH).unwrap();
        assert_eq!(f.title, "Arch Linux: Recent news updates");
        assert_eq!(f.items.len(), 3);
        let first = &f.items[0];
        assert!(
            first.title.starts_with("Update to php ≥ 8.5.12"),
            "{}",
            first.title
        );
        assert!(
            first
                .link
                .starts_with("https://archlinux.org/news/update-to-php")
        );
        assert_eq!(
            first.id,
            "tag:archlinux.org,2026-10-21:/news/update-to-php-8512-andor-php-legacy-8336-to-prevent-remote-code-execution/",
            "the <guid>"
        );
        // Ids are unique per item.
        let mut ids: Vec<&str> = f.items.iter().map(|i| i.id.as_str()).collect();
        ids.dedup();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn atom_entries_use_id_and_the_alternate_link() {
        let f = parse_feed(GITHUB).unwrap();
        assert_eq!(f.title, "Release notes from niri");
        assert_eq!(f.items.len(), 3);
        let e = &f.items[0];
        assert_eq!(e.id, "tag:github.com,2008:Repository/676934005/v26.04");
        assert_eq!(e.title, "v26.04");
        assert_eq!(
            e.link,
            "https://github.com/niri-wm/niri/releases/tag/v26.04"
        );
    }

    #[test]
    fn rdf_items_in_latin1_use_rdf_about() {
        let body = decode_body(SLASHDOT, None);
        let f = parse_feed(&body).unwrap();
        assert_eq!(f.title, "Slashdot");
        assert_eq!(
            f.items.len(),
            2,
            "the <items> table of contents is not an item"
        );
        assert!(f.items[0].id.contains("great-pacific-garbage-patch"));
        assert!(f.items[0].link.contains("utm_medium=feed"), "&amp; decoded");
        assert!(!f.items[0].link.contains("&amp;"));
        assert!(f.items[1].title.starts_with("Applebee's Exposed"));
    }

    #[test]
    fn latin1_bytes_decode_and_utf8_is_kept() {
        let latin = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><rss><channel><title>Caf\xE9</title></channel></rss>";
        assert_eq!(parse_feed(&decode_body(latin, None)).unwrap().title, "Café");
        // Mislabelled: declared Latin-1 but really UTF-8 — read as UTF-8.
        let utf = "<?xml version='1.0' encoding='iso-8859-1'?><rss><channel><title>Café</title></channel></rss>";
        assert_eq!(
            parse_feed(&decode_body(utf.as_bytes(), None))
                .unwrap()
                .title,
            "Café"
        );
        // No declaration: the HTTP charset decides.
        assert_eq!(
            decode_body(b"<rss>\xE9</rss>", Some("windows-1252")),
            "<rss>é</rss>"
        );
        assert_eq!(
            decode_body(b"\xEF\xBB\xBF<rss/>", None),
            "<rss/>",
            "BOM dropped"
        );
    }

    #[test]
    fn cdata_nested_titles_and_fallback_ids() {
        let xml = r#"<rss><channel><title>T</title>
            <item><title><![CDATA[A <b>bold</b>   move]]></title><link>https://x.test/a</link></item>
            <item><title>Only a title</title></item>
            <item><link>  </link></item>
            </channel></rss>"#;
        let f = parse_feed(xml).unwrap();
        assert_eq!(f.items[0].title, "A <b>bold</b> move");
        assert_eq!(f.items[0].id, "https://x.test/a", "no guid: the link");
        assert_eq!(f.items[1].id, "Only a title", "no link: the title");
        assert_eq!(
            f.items.len(),
            2,
            "an item with nothing to key on is dropped"
        );
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>F</title>
            <entry><id>1</id><title>E</title><source><title>Other</title></source>
            <link rel="self" href="https://x.test/self"/><link href="https://x.test/e"/></entry></feed>"#;
        let f = parse_feed(atom).unwrap();
        assert_eq!(f.items[0].title, "E", "a nested <title> doesn't win");
        assert_eq!(f.items[0].link, "https://x.test/e");
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse_feed("<html><body>Not a feed</body></html>").is_err());
        assert!(parse_feed("<rss><channel><title>x</unclosed>").is_err());
        assert!(parse_feed("").is_err());
    }

    #[test]
    fn only_web_links_open() {
        let feed = "https://archlinux.org/feeds/news/";
        assert_eq!(
            openable_link(" https://archlinux.org/news/x/ ", feed).as_deref(),
            Some("https://archlinux.org/news/x/")
        );
        assert_eq!(
            openable_link("/news/x/", feed).as_deref(),
            Some("https://archlinux.org/news/x/")
        );
        assert_eq!(openable_link("file:///etc/passwd", feed), None);
        assert_eq!(openable_link("javascript:alert(1)", feed), None);
        assert_eq!(openable_link("//evil.test/x", feed), None);
        assert_eq!(openable_link("relative/x", feed), None);
        assert_eq!(openable_link("https://a.test/x y", feed), None);
        assert_eq!(origin("http://h.test:8080/a?b"), Some("http://h.test:8080"));
        assert!(valid_feed_url(
            "https://github.com/niri-wm/niri/releases.atom"
        ));
        assert!(!valid_feed_url("ftp://x.test/feed"));
        assert!(!valid_feed_url("https:///feed"));
        assert!(!valid_feed_url("https://x.test/a b"));
    }
}
