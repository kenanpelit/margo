//! RSS / Atom feed notifier (Settings → Notifications → RSS feeds).
//!
//! No feeds (the default) = no network I/O at all.

use super::*;

/// One feed to watch.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct RssFeed {
    /// Shown as the notification's app line; blank = the feed's own title.
    pub name: String,
    /// The feed's `http(s)://` URL (RSS 2.0, RSS 1.0 / RDF or Atom).
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct RssConfig {
    pub feeds: Vec<RssFeed>,
    /// Minutes between checks of a feed (clamped 5..=1440 when used).
    pub interval_minutes: u32,
    /// Show a notification (with an Open action) per new item.
    pub notify: bool,
    /// At most this many item notifications per check; the rest are
    /// summed up in one more (clamped 1..=50 when used).
    pub max_per_poll: u32,
}

impl Default for RssConfig {
    fn default() -> Self {
        Self {
            feeds: Vec::new(),
            interval_minutes: 30,
            notify: true,
            max_per_poll: 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_defaults_watch_nothing_and_partial_yaml_fills_the_rest() {
        let cfg: Config = yaml_serde::from_str("{}").unwrap();
        assert!(cfg.rss.feeds.is_empty(), "no feeds = no I/O");
        let cfg: Config =
            yaml_serde::from_str("rss:\n  feeds:\n    - url: https://archlinux.org/feeds/news/\n")
                .unwrap();
        assert_eq!(cfg.rss.feeds[0].url, "https://archlinux.org/feeds/news/");
        assert_eq!(cfg.rss.feeds[0].name, "");
        assert_eq!((cfg.rss.interval_minutes, cfg.rss.max_per_poll), (30, 5));
        assert!(cfg.rss.notify);
    }
}
