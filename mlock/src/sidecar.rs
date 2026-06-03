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
        let mut info = Self::default();
        let Some(path) = sidecar_path() else {
            return info;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return info;
        };
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
