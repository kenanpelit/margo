// SPDX-License-Identifier: GPL-3.0-or-later

//! Subscriptions and per-episode progress, in
//! `$XDG_DATA_HOME/mtune/podcasts.json` (guarded load/save via
//! `margo_fs::side_file`, like the radio library).

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::Podcast;
use crate::audio::stream::episode_finished;

/// Where an episode stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Progress {
    #[serde(default)]
    pub played: bool,
    /// Resume point in seconds (0 = start).
    #[serde(default)]
    pub position: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PodcastLibrary {
    #[serde(default)]
    pub subscriptions: Vec<Podcast>,
    /// Episode key (`Episode::key`) -> progress. Only episodes the user
    /// touched are stored.
    #[serde(default)]
    pub progress: BTreeMap<String, Progress>,
}

/// Positions closer than this to the start are not worth resuming.
const RESUME_MIN: u64 = 15;

impl PodcastLibrary {
    pub fn path() -> PathBuf {
        margo_fs::xdg::data_home()
            .join("mtune")
            .join("podcasts.json")
    }

    /// The saved library (`false` with defaults when the file is broken;
    /// it is then kept and never saved over).
    pub fn load() -> (Self, bool) {
        match margo_fs::side_file::load(&Self::path(), |s| serde_json::from_str::<Self>(s)) {
            margo_fs::side_file::SideLoad::Loaded(l) => (l, true),
            margo_fs::side_file::SideLoad::Missing => (Self::default(), true),
            margo_fs::side_file::SideLoad::Broken { error, .. } => {
                log::warn!("mtune: {} does not parse: {error}", Self::path().display());
                (Self::default(), false)
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let body = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        margo_fs::side_file::save(&path, &body, |s| {
            serde_json::from_str::<Self>(s).map(|_| ())
        })
        .map_err(|e| e.to_string())
    }

    pub fn is_subscribed(&self, feed_url: &str) -> bool {
        self.subscriptions.iter().any(|p| p.feed_url == feed_url)
    }

    /// Subscribe (no-op when already subscribed). Returns `true` if added.
    pub fn subscribe(&mut self, podcast: Podcast) -> bool {
        if self.is_subscribed(&podcast.feed_url) {
            return false;
        }
        self.subscriptions.push(podcast);
        true
    }

    /// Drop a subscription and the progress of its episodes.
    pub fn unsubscribe(&mut self, feed_url: &str) {
        self.subscriptions.retain(|p| p.feed_url != feed_url);
        let prefix = format!("{feed_url}#");
        self.progress.retain(|k, _| !k.starts_with(&prefix));
    }

    /// Refresh a subscription's title/author/artwork from its feed.
    pub fn update_details(&mut self, feed_url: &str, title: &str, author: &str, image: &str) {
        if let Some(p) = self
            .subscriptions
            .iter_mut()
            .find(|p| p.feed_url == feed_url)
        {
            if !title.is_empty() {
                p.title = title.to_string();
            }
            if !author.is_empty() {
                p.author = author.to_string();
            }
            if !image.is_empty() {
                p.artwork = image.to_string();
            }
        }
    }

    pub fn progress(&self, key: &str) -> Progress {
        self.progress.get(key).copied().unwrap_or_default()
    }

    /// Mark played / unplayed by hand. Either way the resume point resets.
    pub fn set_played(&mut self, key: &str, played: bool) {
        if played {
            self.progress.insert(
                key.to_string(),
                Progress {
                    played: true,
                    position: 0,
                },
            );
        } else {
            self.progress.remove(key);
        }
    }

    /// Record playback reaching `position` of `duration`. Near the end the
    /// episode becomes played (and will restart from the top); otherwise
    /// the position is kept for resuming. Returns `true` when the stored
    /// state changed in a way worth saving.
    pub fn record_position(&mut self, key: &str, position: u64, duration: u64) -> bool {
        let old = self.progress(key);
        let new = if episode_finished(position, duration) {
            Progress {
                played: true,
                position: 0,
            }
        } else if position < RESUME_MIN {
            // Barely started: keep what was there (a replay of a played
            // episode stays played until it gets somewhere).
            return false;
        } else {
            Progress {
                played: false,
                position,
            }
        };
        if new == old {
            return false;
        }
        self.progress.insert(key.to_string(), new);
        true
    }

    /// Where to start an episode: its resume point, or 0.
    pub fn resume_at(&self, key: &str) -> u64 {
        let p = self.progress(key);
        if p.played { 0 } else { p.position }
    }

    /// Unplayed episode count among `keys`.
    pub fn unplayed<'a>(&self, keys: impl IntoIterator<Item = &'a str>) -> usize {
        keys.into_iter()
            .filter(|k| !self.progress(k).played)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(feed: &str) -> Podcast {
        Podcast {
            title: feed.into(),
            feed_url: feed.into(),
            ..Default::default()
        }
    }

    #[test]
    fn subscribe_is_idempotent_and_unsubscribe_forgets_progress() {
        let mut lib = PodcastLibrary::default();
        assert!(lib.subscribe(show("https://a/feed")));
        assert!(!lib.subscribe(show("https://a/feed")));
        lib.subscribe(show("https://b/feed"));
        lib.set_played("https://a/feed#1", true);
        lib.set_played("https://b/feed#1", true);
        lib.unsubscribe("https://a/feed");
        assert_eq!(lib.subscriptions.len(), 1);
        assert!(!lib.progress("https://a/feed#1").played);
        assert!(lib.progress("https://b/feed#1").played);
    }

    #[test]
    fn listening_saves_a_resume_point_then_marks_played() {
        let mut lib = PodcastLibrary::default();
        let k = "f#ep";
        assert!(!lib.record_position(k, 5, 3944), "too early to keep");
        assert!(lib.record_position(k, 600, 3944));
        assert_eq!(lib.resume_at(k), 600);
        assert!(!lib.record_position(k, 600, 3944), "unchanged");
        assert!(lib.record_position(k, 3930, 3944));
        assert!(lib.progress(k).played);
        assert_eq!(lib.resume_at(k), 0, "a played episode restarts");
        // Replaying it: the first seconds do not un-play it.
        assert!(!lib.record_position(k, 3, 3944));
        assert!(lib.progress(k).played);
        assert!(lib.record_position(k, 120, 3944));
        assert!(!lib.progress(k).played);
    }

    #[test]
    fn manual_played_toggle_resets_the_resume_point() {
        let mut lib = PodcastLibrary::default();
        lib.record_position("k", 900, 3600);
        lib.set_played("k", true);
        assert_eq!(
            lib.progress("k"),
            Progress {
                played: true,
                position: 0
            }
        );
        lib.set_played("k", false);
        assert_eq!(lib.progress("k"), Progress::default());
        assert!(
            lib.progress.is_empty(),
            "nothing stored for untouched episodes"
        );
    }

    #[test]
    fn unplayed_counts_untouched_episodes_too() {
        let mut lib = PodcastLibrary::default();
        lib.set_played("f#1", true);
        assert_eq!(lib.unplayed(["f#1", "f#2", "f#3"]), 2);
    }

    #[test]
    fn details_refresh_keeps_known_fields() {
        let mut lib = PodcastLibrary::default();
        lib.subscribe(Podcast {
            title: "Old".into(),
            author: "A".into(),
            feed_url: "https://f".into(),
            ..Default::default()
        });
        lib.update_details("https://f", "New", "", "https://f/art.jpg");
        let p = &lib.subscriptions[0];
        assert_eq!((p.title.as_str(), p.author.as_str()), ("New", "A"));
        assert_eq!(p.artwork, "https://f/art.jpg");
    }
}
