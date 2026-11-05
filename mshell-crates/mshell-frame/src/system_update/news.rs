//! Arch Linux news heads-up. Arch posts a news item when an upgrade
//! needs manual intervention (a config change, a split package, a
//! TPM2 unlock hook...). The panel fetches `archlinux.org/feeds/news/`
//! and flags every post published after the last full system upgrade
//! (or after the user marked the news read, whichever is later), so
//! the user sees them *before* pressing Update.
//!
//! Everything that decides "what is unread" is pure and tested; the
//! fetch and the file reads are thin wrappers.

use chrono::{DateTime, NaiveDateTime};
use serde::{Deserialize, Serialize};

pub(crate) const FEED_URL: &str = "https://archlinux.org/feeds/news/";
pub(crate) const NEWS_PAGE: &str = "https://archlinux.org/news/";
const PACMAN_LOG: &str = "/var/log/pacman.log";
/// Changes on every install/upgrade (pacman adds/removes a
/// `name-version` directory there); the fallback when pacman.log is
/// not readable by the user (it is `root:root 0640` on some setups).
const PACMAN_LOCAL_DB: &str = "/var/lib/pacman/local";

/// One post from the feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NewsItem {
    pub(crate) title: String,
    pub(crate) link: String,
    /// Unix seconds of `<pubDate>` (0 when it didn't parse).
    pub(crate) published: i64,
}

/// Parse the RSS 2.0 feed into posts, newest first (the feed order).
/// A hand-rolled scan over `<item>` blocks: the feed is small and
/// regular, and a malformed item is skipped rather than failing the
/// whole feed.
pub(crate) fn parse_feed(xml: &str) -> Vec<NewsItem> {
    let mut items = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<item>") {
        let after = &rest[start + "<item>".len()..];
        let Some(end) = after.find("</item>") else {
            break;
        };
        let block = &after[..end];
        rest = &after[end + "</item>".len()..];
        let Some(title) = tag_text(block, "title") else {
            continue;
        };
        let link = tag_text(block, "link").unwrap_or_default();
        let published = tag_text(block, "pubDate")
            .and_then(|d| DateTime::parse_from_rfc2822(d.trim()).ok())
            .map(|d| d.timestamp())
            .unwrap_or(0);
        items.push(NewsItem {
            title: unescape(title.trim()),
            link: unescape(link.trim()),
            published,
        });
    }
    items
}

/// Text between `<tag>` and `</tag>` (no attributes on these tags in
/// the Arch feed), unwrapping a CDATA section if present.
fn tag_text<'a>(block: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = block.find(&open)? + open.len();
    let e = block[s..].find(&close)? + s;
    let text = &block[s..e];
    Some(
        text.strip_prefix("<![CDATA[")
            .and_then(|t| t.strip_suffix("]]>"))
            .unwrap_or(text),
    )
}

/// Decode the XML entities the feed uses in titles/links.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Unix seconds of the last `starting full system upgrade` line in a
/// pacman.log. Handles both the current `[2026-11-04T18:49:12+0300]`
/// and the pre-5.1 `[2019-01-01 12:00]` (local time) stamps.
pub(crate) fn last_full_upgrade(log: &str) -> Option<i64> {
    log.lines()
        .rev()
        .find(|l| l.contains("starting full system upgrade"))
        .and_then(log_line_time)
}

/// The timestamp at the head of a pacman.log line.
pub(crate) fn log_line_time(line: &str) -> Option<i64> {
    let stamp = line.strip_prefix('[')?.split(']').next()?;
    if let Ok(t) = DateTime::parse_from_str(stamp, "%Y-%m-%dT%H:%M:%S%z") {
        return Some(t.timestamp());
    }
    NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M")
        .ok()
        .and_then(|n| n.and_local_timezone(chrono::Local).single())
        .map(|t| t.timestamp())
}

/// The posts the user hasn't seen: published after both the last
/// system upgrade and the last "mark read". With neither known (fresh
/// install, unreadable log, never marked) only posts from the last 14
/// days count, so the first open isn't a wall of ancient news.
pub(crate) fn unread(
    items: &[NewsItem],
    last_upgrade: Option<i64>,
    read_at: Option<i64>,
    now: i64,
) -> Vec<NewsItem> {
    let since = match (last_upgrade, read_at) {
        (None, None) => now - 14 * 24 * 3600,
        (a, b) => a.unwrap_or(0).max(b.unwrap_or(0)),
    };
    items
        .iter()
        .filter(|i| i.published > since)
        .cloned()
        .collect()
}

// ── State + IO ───────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct NewsState {
    /// Unix seconds of the last "Mark read".
    read_at: Option<i64>,
}

fn state_path() -> std::path::PathBuf {
    margo_fs::xdg::cache_home()
        .join("mshell")
        .join("system_update_news.json")
}

pub(crate) fn read_at() -> Option<i64> {
    let raw = std::fs::read_to_string(state_path()).ok()?;
    serde_json::from_str::<NewsState>(&raw).ok()?.read_at
}

pub(crate) fn mark_read(now: i64) {
    let path = state_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(&NewsState { read_at: Some(now) }) {
        let _ = std::fs::write(path, json);
    }
}

/// Best estimate of when the system was last fully upgraded: the
/// pacman.log stamp when readable, else the local package DB's mtime
/// (any install moves it, so it can only make the window shorter).
pub(crate) fn last_upgrade_time() -> Option<i64> {
    if let Ok(log) = std::fs::read_to_string(PACMAN_LOG)
        && let Some(t) = last_full_upgrade(&log)
    {
        return Some(t);
    }
    std::fs::metadata(PACMAN_LOCAL_DB)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

type FeedResponse = Option<(Option<String>, Vec<NewsItem>)>;

#[derive(Default)]
struct FeedMemo {
    checked: Option<u64>,
    etag: Option<String>,
    items: Vec<NewsItem>,
}
impl FeedMemo {
    fn get(
        &mut self,
        now: u64,
        fetch: impl FnOnce(Option<&str>) -> Result<FeedResponse, String>,
    ) -> Result<Vec<NewsItem>, String> {
        if self.checked.is_some_and(|t| now.saturating_sub(t) < 3600) {
            return Ok(self.items.clone());
        }
        if let Some((etag, items)) = fetch(self.etag.as_deref())? {
            self.etag = etag;
            self.items = items;
        }
        self.checked = Some(now);
        Ok(self.items.clone())
    }
}

/// Fetch + parse the feed off the GTK thread. All panels share an
/// hour-long TTL and ETag validator (304 keeps the parsed posts).
pub(crate) fn fetch_blocking() -> Result<Vec<NewsItem>, String> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<FeedMemo>> = std::sync::OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|e| e.to_string())?;
    cache.get(super::now_secs(), |etag| {
        let mut req = ureq::get(FEED_URL).timeout(std::time::Duration::from_secs(15));
        if let Some(etag) = etag {
            req = req.set("If-None-Match", etag);
        }
        let response = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(304, _)) => return Ok(None),
            Err(e) => return Err(format!("news: {e}")),
        };
        if response.status() == 304 {
            return Ok(None);
        }
        let etag = response.header("ETag").map(String::from);
        let body = response.into_string().map_err(|e| format!("news: {e}"))?;
        Ok(Some((etag, parse_feed(&body))))
    })
}

/// What the panel needs: the unread posts, computed off the GTK thread.
pub(crate) async fn unread_now() -> Result<Vec<NewsItem>, String> {
    tokio::task::spawn_blocking(|| {
        let items = fetch_blocking()?;
        let now = chrono::Utc::now().timestamp();
        Ok(unread(&items, last_upgrade_time(), read_at(), now))
    })
    .await
    .map_err(|e| format!("news: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = include_str!("testdata/arch-news.xml");

    #[test]
    fn news_is_cached_for_an_hour_then_revalidated() {
        let mut cache = FeedMemo::default();
        let mut calls = 0;
        for now in [100, 101, 3700] {
            let items = cache
                .get(now, |etag| {
                    calls += 1;
                    if calls == 1 {
                        assert!(etag.is_none());
                        Ok(Some((Some("v1".into()), parse_feed(FEED))))
                    } else {
                        assert_eq!(etag, Some("v1"));
                        Ok(None) // HTTP 304
                    }
                })
                .unwrap();
            assert_eq!(items.len(), 3);
        }
        assert_eq!(calls, 2);
    }

    #[test]
    fn parses_the_real_feed() {
        let items = parse_feed(FEED);
        assert_eq!(items.len(), 3);
        assert_eq!(
            items[0].title,
            "Update to php ≥ 8.5.12 and/or php-legacy ≥ 8.3.36 to prevent remote code execution"
        );
        assert!(
            items[0]
                .link
                .starts_with("https://archlinux.org/news/update-to-php")
        );
        // Wed, 21 Oct 2026 13:58:42 +0000
        assert_eq!(items[0].published, 1_792_591_122);
        // Entities in titles are decoded.
        assert_eq!(
            items[2].title,
            "Mkinitcpio >=42 requires manual intervention for TPM2-based unlocking of LUKS devices"
        );
    }

    #[test]
    fn malformed_items_are_skipped_not_fatal() {
        let xml = "<rss><item><link>x</link></item><item><title>ok</title></item><item><title>cut";
        let items = parse_feed(xml);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "ok");
        assert_eq!(items[0].published, 0);
        assert!(parse_feed("").is_empty());
    }

    #[test]
    fn cdata_titles_unwrap() {
        let items = parse_feed("<item><title><![CDATA[a & b]]></title></item>");
        assert_eq!(items[0].title, "a & b");
    }

    #[test]
    fn last_full_upgrade_picks_the_newest_start_line() {
        let log = "\
[2026-10-01T10:00:00+0300] [PACMAN] Running 'pacman -Syu'
[2026-10-01T10:00:01+0300] [PACMAN] starting full system upgrade
[2026-10-01T10:01:00+0300] [ALPM] upgraded linux (6.9.1-1 -> 6.9.2-1)
[2026-10-22T08:00:00+0300] [PACMAN] Running 'pacman -S foo'
[2026-10-22T09:00:00+0300] [PACMAN] starting full system upgrade
[2026-10-22T09:05:00+0300] [ALPM] upgraded mesa (1-1 -> 2-1)
";
        // 2026-10-22T06:00:00Z
        assert_eq!(last_full_upgrade(log), Some(1_792_648_800));
        assert_eq!(
            last_full_upgrade("[2026-10-22T09:00:00+0300] [ALPM] x"),
            None
        );
    }

    #[test]
    fn old_style_log_stamps_parse() {
        assert!(
            log_line_time("[2019-01-01 12:00] [PACMAN] starting full system upgrade").is_some()
        );
        assert_eq!(log_line_time("garbage"), None);
    }

    #[test]
    fn unread_is_after_the_later_of_upgrade_and_read_mark() {
        let items = parse_feed(FEED);
        let php = items[0].published;
        let ca = items[1].published;
        // Upgraded between the two Oct-21 posts: only php is new.
        let u = unread(&items, Some(ca + 1), None, php + 100);
        assert_eq!(u.len(), 1);
        assert!(u[0].title.starts_with("Update to php"));
        // A later read mark wins over an older upgrade.
        assert!(unread(&items, Some(0), Some(php), php + 100).is_empty());
        // Upgraded after everything: nothing unread.
        assert!(unread(&items, Some(php + 1), None, php + 100).is_empty());
    }

    #[test]
    fn unread_without_any_baseline_is_the_last_two_weeks() {
        let items = parse_feed(FEED);
        let php = items[0].published;
        // "now" = one day after the php post: both Oct-21 posts are in
        // the window, the September one is not.
        assert_eq!(unread(&items, None, None, php + 86_400).len(), 2);
    }
}
