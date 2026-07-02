//! Live desktop info the locker can't compute itself — notification
//! count, current weather, and now-playing media — published by the
//! running shell to `$XDG_CACHE_HOME/margo/lock-info` (or
//! `~/.cache/margo/lock-info`).
//!
//! mshell rewrites this tiny key=value file on a short timer (see
//! `mshell-core/src/lock_info.rs`); the locker re-reads it each tick so a
//! notification arriving / a track changing while locked updates within a
//! couple of seconds. Hand-parsed — no serde in the locker.
//!
//! Format (all keys optional):
//! ```text
//! notifications = 3
//! weather = 18°C · Partly cloudy
//! media_title = Song name
//! media_artist = Artist
//! media_playing = 1
//! ```

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct LockInfo {
    pub notifications: u32,
    pub weather: String,
    pub media_title: String,
    pub media_artist: String,
    pub media_playing: bool,
}

impl LockInfo {
    pub fn has_media(&self) -> bool {
        !self.media_title.is_empty() || !self.media_artist.is_empty()
    }

    /// Read the sidecar file. Missing / unreadable → an empty `LockInfo`
    /// (the locker simply renders nothing for those widgets).
    pub fn load() -> Self {
        let Some(path) = sidecar_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        Self::parse(&text)
    }

    /// Pure parse of the sidecar key=value text — split out from
    /// [`Self::load`] so the hand-rolled parsing is unit-testable without
    /// touching `~/.cache`. Unknown / malformed lines are skipped.
    pub(crate) fn parse(text: &str) -> Self {
        let mut info = Self::default();
        for line in text.lines() {
            let Some((key, val)) = line.split_once('=') else {
                continue;
            };
            let val = val.trim();
            match key.trim() {
                "notifications" => info.notifications = val.parse().unwrap_or(0),
                "weather" => info.weather = val.to_string(),
                "media_title" => info.media_title = val.to_string(),
                "media_artist" => info.media_artist = val.to_string(),
                "media_playing" => info.media_playing = matches!(val, "1" | "true" | "yes" | "on"),
                _ => {}
            }
        }
        info
    }
}

fn sidecar_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))?;
    Some(base.join("margo").join("lock-info"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_the_default_empty_info() {
        assert_eq!(LockInfo::parse(""), LockInfo::default());
    }

    #[test]
    fn full_sidecar_parses_every_field() {
        let text = "\
notifications = 3
weather = 18°C · Partly cloudy
media_title = Song name
media_artist = Artist
media_playing = 1
";
        let info = LockInfo::parse(text);
        assert_eq!(info.notifications, 3);
        assert_eq!(info.weather, "18°C · Partly cloudy");
        assert_eq!(info.media_title, "Song name");
        assert_eq!(info.media_artist, "Artist");
        assert!(info.media_playing);
    }

    #[test]
    fn bad_notification_count_falls_back_to_zero() {
        // A non-numeric count must not panic — it degrades to 0.
        assert_eq!(LockInfo::parse("notifications = lots").notifications, 0);
        assert_eq!(LockInfo::parse("notifications = -1").notifications, 0);
    }

    #[test]
    fn media_playing_only_true_for_truthy_spellings() {
        for v in ["1", "true", "yes", "on"] {
            assert!(
                LockInfo::parse(&format!("media_playing = {v}")).media_playing,
                "`{v}` must read as playing"
            );
        }
        for v in ["0", "false", "paused", ""] {
            assert!(
                !LockInfo::parse(&format!("media_playing = {v}")).media_playing,
                "`{v}` must read as not playing"
            );
        }
    }

    #[test]
    fn has_media_tracks_title_or_artist_presence() {
        assert!(!LockInfo::default().has_media());
        assert!(LockInfo::parse("media_title = X").has_media());
        assert!(LockInfo::parse("media_artist = Y").has_media());
        // Weather alone is not "media".
        assert!(!LockInfo::parse("weather = sunny").has_media());
    }

    #[test]
    fn lines_without_equals_are_skipped() {
        let info = LockInfo::parse("garbage line\nweather = ok\n");
        assert_eq!(info.weather, "ok");
    }
}
