//! What the notifier remembers between polls (`$XDG_STATE_HOME/margo/
//! rss-seen.json`): per feed the ids already seen, the HTTP validators for
//! the next conditional GET and when it was last checked. Pure + tested;
//! the service in `mod.rs` loads / saves it off the GTK thread.

use super::parse::{Feed, Item, openable_link};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Seen ids kept per feed at least (more when the feed itself lists more,
/// so an old item can't come back as "new" after falling off the cap).
pub(crate) const MAX_SEEN: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FeedState {
    /// Newest first.
    pub(crate) ids: Vec<String>,
    pub(crate) etag: Option<String>,
    pub(crate) last_modified: Option<String>,
    /// Unix seconds of the last check (successful or not).
    pub(crate) last_checked: i64,
    /// The feed's own title, for notifications of an unnamed feed.
    pub(crate) title: String,
    /// A fetch succeeded once: from now on unseen items are new.
    pub(crate) primed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SeenStore {
    /// Keyed by feed URL.
    pub(crate) feeds: BTreeMap<String, FeedState>,
}

impl SeenStore {
    pub(crate) fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub(crate) fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Whether `url` should be checked at `now`: never checked, or the
    /// interval has passed (a clock that jumped back counts as due).
    pub(crate) fn due(&self, url: &str, now: i64, interval_secs: i64) -> bool {
        self.feeds
            .get(url)
            .is_none_or(|f| now - f.last_checked >= interval_secs || f.last_checked > now)
    }

    /// Seconds until the next feed in `urls` is due (0 = now).
    pub(crate) fn next_due_in(&self, urls: &[&str], now: i64, interval_secs: i64) -> i64 {
        urls.iter()
            .map(|u| match self.feeds.get(*u) {
                Some(f) if f.last_checked <= now => (f.last_checked + interval_secs - now).max(0),
                _ => 0,
            })
            .min()
            .unwrap_or(interval_secs)
    }

    /// A check failed or returned 304: only the time moves.
    pub(crate) fn checked(&mut self, url: &str, now: i64) {
        if let Some(f) = self.feeds.get_mut(url) {
            f.last_checked = now;
        } else {
            // A first check that failed: remember nothing seen yet, so the
            // first good fetch still only marks items seen (no burst).
            self.feeds.insert(
                url.to_string(),
                FeedState {
                    last_checked: now,
                    ..Default::default()
                },
            );
        }
    }

    /// A fetched feed: returns the items not seen before, in feed order.
    /// The first successful fetch of a feed only marks everything seen
    /// (adding a feed must not dump its whole history as notifications).
    pub(crate) fn observe(
        &mut self,
        url: &str,
        feed: &Feed,
        validators: (Option<String>, Option<String>),
        now: i64,
    ) -> Vec<Item> {
        let st = self.feeds.entry(url.to_string()).or_default();
        let first = !st.primed;
        st.primed = true;
        let fresh: Vec<Item> = feed
            .items
            .iter()
            .filter(|i| !st.ids.contains(&i.id))
            .cloned()
            .collect();
        let mut ids: Vec<String> = fresh.iter().map(|i| i.id.clone()).collect();
        ids.dedup();
        ids.extend(std::mem::take(&mut st.ids));
        let mut seen = std::collections::HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));
        ids.truncate(MAX_SEEN.max(feed.items.len() * 2));
        st.ids = ids;
        (st.etag, st.last_modified) = validators;
        st.last_checked = now;
        if !feed.title.is_empty() {
            st.title = feed.title.clone();
        }
        if first { Vec::new() } else { fresh }
    }

    /// Forget feeds no longer configured. Returns whether any went.
    pub(crate) fn retain_feeds(&mut self, urls: &[&str]) -> bool {
        let n = self.feeds.len();
        self.feeds.retain(|u, _| urls.contains(&u.as_str()));
        self.feeds.len() != n
    }
}

/// One desktop notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    /// The feed's name (the notification's app line / body).
    pub(crate) feed: String,
    pub(crate) title: String,
    /// What *Open* opens (`http(s)` only).
    pub(crate) link: Option<String>,
}

/// New items of one poll → notifications: one per item up to `max`, then
/// a single "N more" summary naming the feeds.
pub(crate) fn plan_notices(new: &[(String, String, Vec<Item>)], max: usize) -> Vec<Notice> {
    let mut out = Vec::new();
    let mut rest = 0usize;
    let mut rest_feeds: Vec<&str> = Vec::new();
    for (feed_name, feed_url, items) in new {
        for item in items {
            if out.len() < max {
                out.push(Notice {
                    feed: feed_name.clone(),
                    title: if item.title.is_empty() {
                        "New item".to_string()
                    } else {
                        item.title.clone()
                    },
                    link: openable_link(&item.link, feed_url),
                });
            } else {
                rest += 1;
                if !rest_feeds.contains(&feed_name.as_str()) {
                    rest_feeds.push(feed_name);
                }
            }
        }
    }
    if rest > 0 {
        out.push(Notice {
            feed: rest_feeds.join(", "),
            title: format!("{rest} more new item{}", if rest == 1 { "" } else { "s" }),
            link: None,
        });
    }
    out
}

/// The name a feed's notifications carry: the configured one, else the
/// feed's title, else its host.
pub(crate) fn display_name(configured: &str, title: &str, url: &str) -> String {
    let configured = configured.trim();
    if !configured.is_empty() {
        return configured.to_string();
    }
    if !title.trim().is_empty() {
        return title.trim().to_string();
    }
    super::parse::origin(url)
        .and_then(|o| o.split("://").nth(1))
        .unwrap_or(url)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(ids: &[&str]) -> Feed {
        Feed {
            title: "Arch Linux news".into(),
            items: ids
                .iter()
                .map(|i| Item {
                    id: i.to_string(),
                    title: format!("Item {i}"),
                    link: format!("https://x.test/{i}"),
                })
                .collect(),
        }
    }

    const U: &str = "https://x.test/feed";

    #[test]
    fn the_first_fetch_marks_seen_then_only_new_items_count() {
        let mut s = SeenStore::default();
        assert!(
            s.observe(U, &feed(&["3", "2", "1"]), (None, None), 10)
                .is_empty()
        );
        assert_eq!(s.feeds[U].ids, ["3", "2", "1"]);
        let new = s.observe(
            U,
            &feed(&["5", "4", "3", "2"]),
            (Some("\"e1\"".into()), None),
            20,
        );
        let ids: Vec<&str> = new.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["5", "4"]);
        assert_eq!(s.feeds[U].ids, ["5", "4", "3", "2", "1"], "old ids kept");
        assert_eq!(s.feeds[U].etag.as_deref(), Some("\"e1\""));
        assert!(
            s.observe(U, &feed(&["5", "4"]), (None, None), 30)
                .is_empty()
        );
        // An item that drops out and comes back is not new.
        assert!(
            s.observe(U, &feed(&["5", "4", "1"]), (None, None), 40)
                .is_empty()
        );
    }

    #[test]
    fn a_failed_first_check_still_suppresses_the_backlog() {
        let mut s = SeenStore::default();
        s.checked(U, 10);
        assert!(!s.due(U, 11, 60));
        assert!(
            s.observe(U, &feed(&["2", "1"]), (None, None), 100)
                .is_empty()
        );
        // A feed that started empty still notifies its first items.
        let mut s = SeenStore::default();
        assert!(s.observe(U, &feed(&[]), (None, None), 1).is_empty());
        assert_eq!(s.observe(U, &feed(&["1"]), (None, None), 2).len(), 1);
    }

    #[test]
    fn seen_ids_are_capped_but_cover_the_whole_feed() {
        let mut s = SeenStore::default();
        let many: Vec<String> = (0..400).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        s.observe(U, &feed(&refs[..10]), (None, None), 1);
        for chunk in refs.chunks(50) {
            s.observe(U, &feed(chunk), (None, None), 2);
        }
        assert_eq!(s.feeds[U].ids.len(), MAX_SEEN);
        let big: Vec<String> = (0..500).map(|i| format!("b{i}")).collect();
        let big: Vec<&str> = big.iter().map(String::as_str).collect();
        s.observe(U, &feed(&big), (None, None), 3);
        assert_eq!(s.feeds[U].ids.len(), 800, "twice the feed's own length");
        assert!(s.observe(U, &feed(&big), (None, None), 4).is_empty());
    }

    #[test]
    fn scheduling_and_pruning() {
        let mut s = SeenStore::default();
        assert!(s.due(U, 0, 1800), "never checked");
        s.checked(U, 1000);
        assert!(!s.due(U, 1500, 1800));
        assert!(s.due(U, 2800, 1800));
        assert!(s.due(U, 500, 1800), "clock went back");
        assert_eq!(s.next_due_in(&[U], 1500, 1800), 1300);
        assert_eq!(s.next_due_in(&[U, "https://new.test/"], 1500, 1800), 0);
        assert_eq!(s.next_due_in(&[], 1500, 1800), 1800);
        assert!(!s.retain_feeds(&[U]));
        assert!(s.retain_feeds(&[]));
        assert!(s.feeds.is_empty());
    }

    #[test]
    fn store_round_trips_and_tolerates_missing_fields() {
        let mut s = SeenStore::default();
        s.observe(
            U,
            &feed(&["1"]),
            (
                Some("W/\"x\"".into()),
                Some("Wed, 21 Oct 2026 19:29:36 GMT".into()),
            ),
            5,
        );
        assert_eq!(SeenStore::from_json(&s.to_json()).unwrap(), s);
        let old =
            SeenStore::from_json(r#"{"feeds": {"https://a.test/": {"ids": ["1"]}}}"#).unwrap();
        assert_eq!(old.feeds["https://a.test/"].last_checked, 0);
        assert!(SeenStore::from_json("[1,2]").is_err());
    }

    #[test]
    fn notices_are_capped_with_one_summary() {
        let a = feed(&["1", "2", "3"]).items;
        let b = vec![Item {
            id: "x".into(),
            title: String::new(),
            link: "javascript:alert(1)".into(),
        }];
        let plan = plan_notices(
            &[
                ("Arch".into(), U.into(), a),
                ("Blog".into(), "https://b.test/".into(), b),
            ],
            2,
        );
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0].title, "Item 1");
        assert_eq!(plan[0].link.as_deref(), Some("https://x.test/1"));
        assert_eq!(plan[2].title, "2 more new items");
        assert_eq!(plan[2].feed, "Arch, Blog");
        assert_eq!(plan[2].link, None);
        let plan = plan_notices(
            &[(
                "Blog".into(),
                "https://b.test/".into(),
                vec![Item {
                    id: "x".into(),
                    title: String::new(),
                    link: "javascript:alert(1)".into(),
                }],
            )],
            5,
        );
        assert_eq!(plan[0].title, "New item");
        assert_eq!(plan[0].link, None, "never a non-web link");
        assert!(plan_notices(&[], 5).is_empty());
    }

    #[test]
    fn names_fall_back_to_title_then_host() {
        assert_eq!(display_name(" Arch ", "x", U), "Arch");
        assert_eq!(
            display_name("", "Release notes from niri", U),
            "Release notes from niri"
        );
        assert_eq!(
            display_name("", " ", "https://x.test:8443/feed"),
            "x.test:8443"
        );
    }
}
