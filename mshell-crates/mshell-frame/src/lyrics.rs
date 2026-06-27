//! Synced-lyrics engine.
//!
//! Resolves time-synced lyrics for the now-playing track and turns a
//! playback position into the active line. Lookup order: on-disk cache →
//! lrclib.net (`/api/get` exact match, then `/api/search` fuzzy). Results —
//! including a definitive "no lyrics" — are cached under
//! `~/.cache/mshell/lyrics/` so a re-play is offline and instant; a *transient*
//! network error is never cached, so the next play retries.
//!
//! Blocking by design: [`fetch`] does network I/O with `ureq` and MUST run off
//! the GTK main thread (the bar pill + the menu call it through
//! `tokio::task::spawn_blocking`). [`parse_lrc`] / [`index_for_time`] are cheap
//! pure string work and run anywhere.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// One lyric line: a timestamp (ms from track start) and its text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

/// Resolved lyrics for a track.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Lyrics {
    /// Time-synced lines, sorted ascending by `time_ms`.
    Synced(Vec<LyricLine>),
    /// Unsynced plain text, one entry per source line.
    Plain(Vec<String>),
    /// Looked up and there are none (instrumental, or genuinely absent).
    None,
}

impl Lyrics {
    /// Whether there is anything to show.
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Lyrics::Synced(v) => v.is_empty(),
            Lyrics::Plain(v) => v.iter().all(|l| l.trim().is_empty()),
            Lyrics::None => true,
        }
    }
}

/// Track identity — the lrclib query key and the cache key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrackKey {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration_secs: u64,
}

impl TrackKey {
    /// Whether this names a real track we can look up (lrclib needs both).
    pub(crate) fn is_valid(&self) -> bool {
        !self.title.trim().is_empty() && !self.artist.trim().is_empty()
    }

    /// Stable, collision-resistant cache filename stem. Case-insensitive so
    /// "Daft Punk" and "daft punk" share one entry.
    fn cache_id(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.artist.to_lowercase().hash(&mut h);
        self.title.to_lowercase().hash(&mut h);
        self.album.to_lowercase().hash(&mut h);
        self.duration_secs.hash(&mut h);
        format!("{:016x}", h.finish())
    }
}

const LRCLIB_BASE: &str = "https://lrclib.net/api";
const USER_AGENT: &str = concat!(
    "mshell-lyrics/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/kenanpelit/margo)"
);
const TIMEOUT: Duration = Duration::from_secs(12);

/// Resolve lyrics for `key`: cache first, then lrclib. Blocking — run off the
/// main thread. A transient network failure returns [`Lyrics::None`] without
/// caching it, so the caller can retry on the next track change.
pub(crate) fn fetch(key: &TrackKey) -> Lyrics {
    if !key.is_valid() {
        return Lyrics::None;
    }
    if let Some(cached) = load_cache(key) {
        return cached;
    }
    match fetch_lrclib(key) {
        Some(lyrics) => {
            save_cache(key, &lyrics);
            lyrics
        }
        // Transport error — don't cache; let a later play retry.
        None => Lyrics::None,
    }
}

/// `Some` = a definitive answer from lrclib (content or "nothing exists");
/// `None` = a transport/parse error we should not cache.
fn fetch_lrclib(key: &TrackKey) -> Option<Lyrics> {
    // Exact match first — artist + title + album + duration.
    let get = ureq::get(&format!("{LRCLIB_BASE}/get"))
        .query("artist_name", &key.artist)
        .query("track_name", &key.title)
        .query("album_name", &key.album)
        .query("duration", &key.duration_secs.to_string());
    match call_json(get) {
        Ok(Some(v)) => return Some(lyrics_from_json(&v)),
        Ok(None) => {} // 404 — fall through to the fuzzy search.
        Err(()) => return None,
    }

    // Fuzzy fallback — pick the first hit that actually has synced lyrics.
    let search = ureq::get(&format!("{LRCLIB_BASE}/search"))
        .query("artist_name", &key.artist)
        .query("track_name", &key.title);
    match call_json(search) {
        Ok(Some(v)) => {
            let arr = v.as_array().cloned().unwrap_or_default();
            let best = arr
                .iter()
                .find(|e| {
                    e.get("syncedLyrics")
                        .and_then(|s| s.as_str())
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false)
                })
                .or_else(|| arr.first());
            Some(best.map(lyrics_from_json).unwrap_or(Lyrics::None))
        }
        Ok(None) => Some(Lyrics::None),
        Err(()) => None,
    }
}

/// Run a GET and parse JSON. `Ok(None)` for a 404, `Err` for any
/// transport/parse failure.
fn call_json(req: ureq::Request) -> Result<Option<serde_json::Value>, ()> {
    match req.timeout(TIMEOUT).set("User-Agent", USER_AGENT).call() {
        Ok(r) => {
            let body = r.into_string().map_err(|_| ())?;
            serde_json::from_str(&body).map(Some).map_err(|_| ())
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(_) => Err(()),
    }
}

/// Turn one lrclib track object into [`Lyrics`]: synced if present, else plain,
/// else none (instrumental tracks report none).
fn lyrics_from_json(v: &serde_json::Value) -> Lyrics {
    if v.get("instrumental")
        .and_then(|b| b.as_bool())
        .unwrap_or(false)
    {
        return Lyrics::None;
    }
    if let Some(synced) = v.get("syncedLyrics").and_then(|s| s.as_str())
        && !synced.trim().is_empty()
    {
        let lines = parse_lrc(synced);
        if !lines.is_empty() {
            return Lyrics::Synced(lines);
        }
    }
    if let Some(plain) = v.get("plainLyrics").and_then(|s| s.as_str()) {
        let lines: Vec<String> = plain.lines().map(|l| l.trim().to_string()).collect();
        if lines.iter().any(|l| !l.is_empty()) {
            return Lyrics::Plain(lines);
        }
    }
    Lyrics::None
}

/// Parse an LRC body into sorted, timestamped lines. Handles multiple
/// timestamps per line (`[00:12.00][01:45.00]chorus`) and skips metadata tags
/// (`[ar:…]`, `[length:…]`) — anything whose bracket isn't a real `mm:ss`.
pub(crate) fn parse_lrc(raw: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw_line in raw.lines() {
        let mut rest = raw_line;
        let mut stamps = Vec::new();
        while rest.starts_with('[') {
            let Some(close) = rest.find(']') else { break };
            if let Some(ms) = parse_timestamp(&rest[1..close]) {
                stamps.push(ms);
            }
            rest = &rest[close + 1..];
        }
        if stamps.is_empty() {
            continue;
        }
        let text = rest.trim().to_string();
        for ms in stamps {
            lines.push(LyricLine {
                time_ms: ms,
                text: text.clone(),
            });
        }
    }
    lines.sort_by_key(|l| l.time_ms);
    lines
}

/// Parse an LRC timestamp tag (`mm:ss`, `mm:ss.xx`, `mm:ss.xxx`) to ms.
/// Returns `None` for metadata tags so they're dropped by the caller.
fn parse_timestamp(tag: &str) -> Option<u64> {
    let (mm, rest) = tag.split_once(':')?;
    let minutes: u64 = mm.trim().parse().ok()?;
    let (ss, frac) = rest.split_once('.').unwrap_or((rest, ""));
    let seconds: u64 = ss.trim().parse().ok()?;
    let frac_ms = if frac.is_empty() {
        0
    } else {
        let mut f = frac.trim().to_string();
        while f.len() < 3 {
            f.push('0');
        }
        f.truncate(3);
        f.parse::<u64>().ok()?
    };
    Some((minutes * 60 + seconds) * 1000 + frac_ms)
}

/// Index of the active line at `position_ms`: the last line whose timestamp has
/// passed. `None` before the first stamp (intro) or for empty input.
pub(crate) fn index_for_time(lines: &[LyricLine], position_ms: u64) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    match lines.binary_search_by_key(&position_ms, |l| l.time_ms) {
        Ok(i) => Some(i),
        Err(0) => None,
        Err(i) => Some(i - 1),
    }
}

// ── Disk cache ───────────────────────────────────────────────────────────

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("mshell").join("lyrics"))
}

fn cache_path(key: &TrackKey) -> Option<PathBuf> {
    Some(cache_dir()?.join(format!("{}.json", key.cache_id())))
}

fn load_cache(key: &TrackKey) -> Option<Lyrics> {
    let text = std::fs::read_to_string(cache_path(key)?).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_cache(key: &TrackKey, lyrics: &Lyrics) {
    let Some(path) = cache_path(key) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(lyrics) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synced_lines_in_order() {
        let lrc = "[00:12.50]second\n[00:01.00]first\n[ar:Someone]\n[00:30.00]third";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].time_ms, 1000);
        assert_eq!(lines[0].text, "first");
        assert_eq!(lines[1].time_ms, 12_500);
        assert_eq!(lines[2].time_ms, 30_000);
    }

    #[test]
    fn parses_repeated_timestamps() {
        let lines = parse_lrc("[00:10.00][01:10.00]chorus");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time_ms, 10_000);
        assert_eq!(lines[1].time_ms, 70_000);
        assert_eq!(lines[1].text, "chorus");
    }

    #[test]
    fn skips_metadata_only_lines() {
        assert!(parse_lrc("[ar:Artist]\n[ti:Title]\n[length:03:00]").is_empty());
    }

    #[test]
    fn active_index_tracks_position() {
        let lines = parse_lrc("[00:01.00]a\n[00:05.00]b\n[00:10.00]c");
        assert_eq!(index_for_time(&lines, 0), None);
        assert_eq!(index_for_time(&lines, 1_000), Some(0));
        assert_eq!(index_for_time(&lines, 4_999), Some(0));
        assert_eq!(index_for_time(&lines, 5_000), Some(1));
        assert_eq!(index_for_time(&lines, 99_000), Some(2));
    }
}
