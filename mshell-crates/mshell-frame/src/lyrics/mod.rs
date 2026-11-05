//! Synced-lyrics engine.
//!
//! Resolves time-synced lyrics for the now-playing track and turns a
//! playback position into the active line (and, for word-timed lyrics, how
//! far into it the singer is). Lookup order: the playing file's embedded
//! lyrics (mtune) → a local `.lrc` folder when set ([`local`]) → on-disk
//! cache → lrclib.net (`/api/get` exact match, then
//! `/api/search` fuzzy) → NetEase Cloud Music when the user enabled it
//! (`bars.widgets.lyrics.netease_fallback`) and lrclib had nothing synced.
//! Results — including a definitive "no lyrics" — are cached under
//! `~/.cache/mshell/lyrics/` so a re-play is offline and instant; a
//! *transient* network error is never cached and comes back as
//! [`Resolution::transient`], so the caller retries with [`retry_delay`].
//!
//! Blocking by design: [`fetch`] does network I/O with `ureq` and MUST run off
//! the GTK main thread (the bar pill + the menu call it through
//! `tokio::task::spawn_blocking`). Parsing ([`lrc`]), [`index_for_time`],
//! [`lrc::karaoke_split`] and [`PlayClock`] are cheap pure work.

mod local;
pub(crate) mod lrc;
mod netease;
pub(crate) mod offsets;
pub(crate) mod ui;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// One timed word of a word-synced (karaoke) line. `text` keeps its trailing
/// space, so a line's words concatenate to exactly [`LyricLine::text`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Word {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// One lyric line: a timestamp (ms from track start) and its text, plus the
/// optional word timings and translation / romanization layers. The extra
/// fields default on read so caches written before they existed still load.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LyricLine {
    pub time_ms: u64,
    pub text: String,
    /// Word timings (enhanced LRC / NetEase YRC). Empty = line-synced only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<Word>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub romanization: Option<String>,
}

impl LyricLine {
    /// The secondary line shown under this one when the user enabled it:
    /// the translation, else the romanization.
    pub(crate) fn secondary(&self) -> Option<&str> {
        self.translation
            .as_deref()
            .or(self.romanization.as_deref())
            .filter(|s| !s.trim().is_empty())
    }
}

/// Resolved lyrics for a track.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Lyrics {
    /// Time-synced lines, sorted ascending by `time_ms`.
    Synced(Vec<LyricLine>),
    /// Unsynced plain text, one entry per source line.
    Plain(Vec<String>),
    /// The source flagged the track instrumental — no lyrics exist by design.
    Instrumental,
    /// Looked up and there are none (genuinely absent).
    None,
}

/// Where a resolved [`Lyrics`] value came from — drives the status badge.
/// Always returned alongside a `Lyrics`, never inferred after the fact: a
/// `Lyrics::Plain` from an embedded tag and one from lrclib are
/// structurally identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum LyricsSource {
    /// Read from the playing file's own tags (mtune only).
    Embedded,
    /// Resolved via lrclib.net (disk-cached or fresh).
    Lrclib,
    /// Resolved via NetEase Cloud Music (the opt-in fallback).
    NetEase,
    /// A `.lrc` / `.txt` file in the user's local lyrics folder.
    Local,
}

impl LyricsSource {
    /// Short name for the status badge (`"Synced · lrclib.net"`).
    pub(crate) fn label(self) -> &'static str {
        match self {
            LyricsSource::Embedded => "embedded",
            LyricsSource::Lrclib => "lrclib.net",
            LyricsSource::NetEase => "NetEase",
            LyricsSource::Local => "local file",
        }
    }
}

/// One finished lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resolution {
    pub lyrics: Lyrics,
    pub source: LyricsSource,
    /// Every source that could have answered failed on the network — not
    /// cached; retry later ([`retry_delay`]).
    pub transient: bool,
}

/// Which sources a lookup may use (from `bars.widgets.lyrics`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FetchOptions {
    /// Ask NetEase when lrclib has no synced lyrics.
    pub netease: bool,
    /// Folder of the user's own `.lrc` files, asked before everything but
    /// embedded lyrics.
    pub local_dir: Option<PathBuf>,
}

impl FetchOptions {
    /// Options from the raw config values (`local_dir` may use `~`).
    pub(crate) fn new(netease: bool, local_dir: &str) -> Self {
        Self {
            netease,
            local_dir: local::expand_dir(local_dir),
        }
    }
}

impl Lyrics {
    /// Whether there is anything to show.
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Lyrics::Synced(v) => v.is_empty(),
            Lyrics::Plain(v) => v.iter().all(|l| l.trim().is_empty()),
            Lyrics::Instrumental | Lyrics::None => true,
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
    /// Whether this names a real track we can look up. lrclib matches on the
    /// title alone (artist refines it), so a title is enough — players like mpv
    /// and browsers often leave the artist blank.
    pub(crate) fn is_valid(&self) -> bool {
        !self.title.trim().is_empty()
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

/// Build a lookup key from raw MPRIS metadata, cleaning the noise browsers /
/// YouTube bake into titles and splitting an "Artist - Title" when no separate
/// artist is given — without this, lrclib can't match and players that leave
/// the artist blank (mpv, browser tabs) never resolve.
pub(crate) fn key_for(title: &str, artist: &str, album: &str, duration_secs: u64) -> TrackKey {
    let (title, artist) = clean_track(title, artist);
    TrackKey {
        artist,
        title,
        album: album.trim().to_string(),
        duration_secs,
    }
}

/// Strip MPRIS title noise so lrclib can match: bracketed tags
/// (`(Official Video)`, `[HD]`, …) whose contents look like noise, a trailing
/// `feat./ft. …`, a `" - Topic"` artist suffix, and — when the artist is empty
/// — an `"Artist - Title"` title split. Ported from the musiclyrics plugin.
fn clean_track(title: &str, artist: &str) -> (String, String) {
    const NOISE: &[&str] = &[
        "official",
        "video",
        "audio",
        "lyric",
        "lyrics",
        "visualizer",
        "mv",
        "hd",
        "4k",
        "remaster",
        "remastered",
        "live",
        "explicit",
        "hq",
        "music video",
        "color coded",
        "performance",
    ];

    // Drop bracketed groups whose contents look like noise; keep the rest.
    let mut t = String::new();
    let mut depth = 0i32;
    let mut buf = String::new();
    for ch in title.chars() {
        match ch {
            '(' | '[' => {
                if depth == 0 {
                    buf.clear();
                }
                depth += 1;
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let low = buf.to_lowercase();
                    if !NOISE.iter().any(|n| low.contains(n)) {
                        t.push('(');
                        t.push_str(&buf);
                        t.push(')');
                    }
                }
            }
            _ if depth > 0 => buf.push(ch),
            _ => t.push(ch),
        }
    }

    // "feat. …" / "ft. …" → drop to the end of that segment. Guarded on equal
    // byte length so a non-ASCII title can't desync the lowercase byte offset
    // from `t` and slice on a non-char-boundary.
    let lower = t.to_lowercase();
    if lower.len() == t.len() {
        for sep in [" feat.", " feat ", " ft.", " ft ", " featuring "] {
            if let Some(idx) = lower.find(sep) {
                t.truncate(idx);
                break;
            }
        }
    }

    let mut title = t.trim().trim_end_matches('-').trim().to_string();
    let mut artist = artist
        .trim()
        .trim_end_matches(" - Topic")
        .trim()
        .to_string();

    // "Artist - Title" with no separate artist → split it.
    if artist.is_empty() {
        let whole = title.clone();
        if let Some((left, right)) = whole.split_once(" - ") {
            artist = left.trim().to_string();
            title = right.trim().to_string();
        }
    }

    (title, artist)
}

const LRCLIB_BASE: &str = "https://lrclib.net/api";
const USER_AGENT: &str = concat!(
    "mshell-lyrics/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/kenanpelit/margo)"
);
const TIMEOUT: Duration = Duration::from_secs(12);

/// Resolve lyrics for `key`. `embedded` — the playing file's own embedded
/// lyrics text, when the caller has one (mtune only) — wins unconditionally
/// and short-circuits before touching the disk cache or the network at
/// all; it is parsed as LRC first, so embedded synced lyrics stay synced.
/// Otherwise: cache first, then the network chain. Blocking — run off the
/// main thread.
pub(crate) fn fetch(key: &TrackKey, embedded: Option<&str>, opts: &FetchOptions) -> Resolution {
    if let Some(text) = embedded.map(str::trim).filter(|s| !s.is_empty()) {
        return Resolution {
            lyrics: embedded_lyrics(text),
            source: LyricsSource::Embedded,
            transient: false,
        };
    }
    fetch_inner(key, opts, false)
}

/// Embedded tag text → [`Lyrics`]: a tag holding LRC (Vorbis `LYRICS` often
/// does, enhanced word tags included) stays synced; anything else is plain.
fn embedded_lyrics(text: &str) -> Lyrics {
    let synced = parse_lrc(text);
    if !synced.is_empty() {
        return Lyrics::Synced(synced);
    }
    Lyrics::Plain(text.lines().map(str::to_string).collect())
}

/// Like [`fetch`] but always re-hits the network, bypassing (and
/// overwriting) the disk cache. Backs the menu's manual refresh button.
pub(crate) fn refetch(key: &TrackKey, opts: &FetchOptions) -> Resolution {
    fetch_inner(key, opts, true)
}

fn fetch_inner(key: &TrackKey, opts: &FetchOptions, force: bool) -> Resolution {
    if !key.is_valid() {
        return Resolution {
            lyrics: Lyrics::None,
            source: LyricsSource::Lrclib,
            transient: false,
        };
    }
    // The user's own files always win over a cache entry or the network,
    // and are never cached (editing the file must show at once).
    if let Some(dir) = &opts.local_dir
        && let Some(lyrics) = local::lookup(dir, key)
    {
        return Resolution {
            lyrics,
            source: LyricsSource::Local,
            transient: false,
        };
    }
    if !force && let Some((lyrics, source)) = load_cache(key, opts) {
        return Resolution {
            lyrics,
            source,
            transient: false,
        };
    }
    let res = resolve_chain(fetch_lrclib(key), || netease::fetch(key), opts.netease);
    if !res.transient {
        save_cache(key, opts, &res.lyrics, res.source);
    }
    res
}

/// Combine the sources: lrclib first; NetEase (when `netease_on`) only when
/// lrclib has nothing *synced*. Each argument is `Some` for a definitive
/// answer and `None` for a transport failure. Synced beats plain beats
/// nothing; the result is transient only when no source gave a definitive
/// answer and at least one failed on the network.
fn resolve_chain(
    lrclib: Option<Lyrics>,
    netease: impl FnOnce() -> Option<Lyrics>,
    netease_on: bool,
) -> Resolution {
    let done = |lyrics, source| Resolution {
        lyrics,
        source,
        transient: false,
    };
    if let Some(l @ (Lyrics::Synced(_) | Lyrics::Instrumental)) = &lrclib {
        return done(l.clone(), LyricsSource::Lrclib);
    }
    if !netease_on {
        return match lrclib {
            Some(l) => done(l, LyricsSource::Lrclib),
            None => Resolution {
                lyrics: Lyrics::None,
                source: LyricsSource::Lrclib,
                transient: true,
            },
        };
    }
    let ne = netease();
    match (lrclib, ne) {
        (_, Some(l @ Lyrics::Synced(_))) => done(l, LyricsSource::NetEase),
        (Some(l @ Lyrics::Plain(_)), _) => done(l, LyricsSource::Lrclib),
        (_, Some(l @ (Lyrics::Plain(_) | Lyrics::Instrumental))) => done(l, LyricsSource::NetEase),
        (Some(_), Some(_)) => done(Lyrics::None, LyricsSource::Lrclib),
        _ => Resolution {
            lyrics: Lyrics::None,
            source: LyricsSource::Lrclib,
            transient: true,
        },
    }
}

/// Back-off before retry number `attempt` (0-based) after a transient
/// failure: 5 s, 15 s, 45 s, 2 min, then give up until the next track
/// change. A brief Wi-Fi drop recovers within seconds; a long outage stops
/// hammering after four tries.
pub(crate) fn retry_delay(attempt: u32) -> Option<Duration> {
    const STEPS: [u64; 4] = [5, 15, 45, 120];
    STEPS.get(attempt as usize).map(|s| Duration::from_secs(*s))
}

/// `Some` = a definitive answer from lrclib (content or "nothing exists");
/// `None` = a transport/parse error we should not cache.
fn fetch_lrclib(key: &TrackKey) -> Option<Lyrics> {
    // Exact match first — title, refined by artist / album / duration when
    // known. An empty artist must be *omitted*, not sent blank, or lrclib's
    // exact endpoint won't match.
    let mut get = ureq::get(&format!("{LRCLIB_BASE}/get")).query("track_name", &key.title);
    if !key.artist.is_empty() {
        get = get.query("artist_name", &key.artist);
    }
    if !key.album.is_empty() {
        get = get.query("album_name", &key.album);
    }
    if key.duration_secs > 0 {
        get = get.query("duration", &key.duration_secs.to_string());
    }
    match call_json(get, USER_AGENT) {
        Ok(Some(v)) => {
            // Instrumental → no lyrics exist anywhere; don't waste a search.
            if v.get("instrumental")
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
            {
                return Some(Lyrics::Instrumental);
            }
            let lyrics = lyrics_from_json(&v);
            if !lyrics.is_empty() {
                return Some(lyrics);
            }
            // 200 but no usable lyrics — fall through to the fuzzy search.
        }
        Ok(None) => {} // 404 — fall through to the fuzzy search.
        Err(()) => return None,
    }

    // Fuzzy fallback — pick the first hit that actually has synced lyrics.
    let mut search = ureq::get(&format!("{LRCLIB_BASE}/search")).query("track_name", &key.title);
    if !key.artist.is_empty() {
        search = search.query("artist_name", &key.artist);
    }
    match call_json(search, USER_AGENT) {
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

/// Run a GET with `user_agent` and parse JSON. `Ok(None)` for a 404, `Err`
/// for any transport/parse failure.
fn call_json(req: ureq::Request, user_agent: &str) -> Result<Option<serde_json::Value>, ()> {
    match req.timeout(TIMEOUT).set("User-Agent", user_agent).call() {
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
        return Lyrics::Instrumental;
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

pub(crate) use lrc::parse_lrc;

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

/// Bound for the user's timing offset (Settings → Widgets → Lyrics).
pub(crate) const MAX_OFFSET_MS: i32 = 10_000;

/// The lyric-clock position for a player position and the user's timing
/// offset. A *positive* offset shows lines earlier (the LRC `[offset:]`
/// convention), so it is added to the position the lines are looked up at.
pub(crate) fn effective_position(position_ms: u64, offset_ms: i32) -> u64 {
    let offset = offset_ms.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS);
    if offset >= 0 {
        position_ms.saturating_add(offset as u64)
    } else {
        position_ms.saturating_sub(offset.unsigned_abs() as u64)
    }
}

/// The player position to seek to so that a line stamped `line_ms` becomes
/// active under the total timing `offset_ms` — the inverse of
/// [`effective_position`].
pub(crate) fn seek_position_for(line_ms: u64, offset_ms: i32) -> u64 {
    let offset = offset_ms.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS);
    if offset >= 0 {
        line_ms.saturating_sub(offset as u64)
    } else {
        line_ms.saturating_add(offset.unsigned_abs() as u64)
    }
}

/// Global offset + this track's nudge, clamped.
pub(crate) fn total_offset(global_ms: i32, track_ms: i32) -> i32 {
    global_ms
        .saturating_add(track_ms)
        .clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS)
}

/// Whether an MPRIS player may drive the lyrics. Entries are
/// case-insensitive substrings matched against the player's identity
/// ("Spotify", "Mozilla Firefox") and bus name
/// ("org.mpris.MediaPlayer2.spotify"). A non-empty allowlist admits only
/// matching players; the blocklist then removes matches. Both empty = every
/// player (today's behaviour).
pub(crate) fn player_allowed(
    identity: &str,
    bus: &str,
    allow: &[String],
    block: &[String],
) -> bool {
    let hay = format!("{identity} {bus}").to_lowercase();
    let hit = |list: &[String]| {
        list.iter()
            .map(|e| e.trim().to_lowercase())
            .any(|e| !e.is_empty() && hay.contains(&e))
    };
    let has_allow = allow.iter().any(|e| !e.trim().is_empty());
    (!has_allow || hit(allow)) && !hit(block)
}

/// Smooth playback position between MPRIS position samples (wayle polls
/// them once a second — too coarse for word-by-word highlighting). Anchored
/// at the last sample; while playing, the position advances with wall time,
/// capped [`PlayClock::MAX_EXTRAPOLATION`] past the anchor so a stalled
/// player can't run the lyrics away.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlayClock {
    anchor_ms: u64,
    anchor_at: Instant,
    playing: bool,
}

impl PlayClock {
    pub(crate) const MAX_EXTRAPOLATION: Duration = Duration::from_millis(2_500);

    pub(crate) fn new(now: Instant) -> Self {
        Self {
            anchor_ms: 0,
            anchor_at: now,
            playing: false,
        }
    }

    /// A fresh position sample from the player.
    pub(crate) fn sample(&mut self, position_ms: u64, now: Instant) {
        self.anchor_ms = position_ms;
        self.anchor_at = now;
    }

    /// Playback started / paused. Re-anchors at the current estimate so a
    /// pause freezes where it was.
    pub(crate) fn set_playing(&mut self, playing: bool, now: Instant) {
        if playing != self.playing {
            self.anchor_ms = self.at(now);
            self.anchor_at = now;
            self.playing = playing;
        }
    }

    pub(crate) fn playing(&self) -> bool {
        self.playing
    }

    /// Estimated position at `now`, in ms.
    pub(crate) fn at(&self, now: Instant) -> u64 {
        if !self.playing {
            return self.anchor_ms;
        }
        let elapsed = now
            .saturating_duration_since(self.anchor_at)
            .min(Self::MAX_EXTRAPOLATION);
        self.anchor_ms + elapsed.as_millis() as u64
    }
}

/// Whether a lyrics value has any word-timed line (drives the fast
/// karaoke repaint tick).
pub(crate) fn has_word_timing(lyrics: &Lyrics) -> bool {
    matches!(lyrics, Lyrics::Synced(lines) if lines.iter().any(|l| !l.words.is_empty()))
}

// ── Disk cache ───────────────────────────────────────────────────────────

/// What a cache file holds. Older caches are a bare [`Lyrics`] (lrclib
/// only); [`load_cache`] still reads those.
#[derive(Serialize, Deserialize)]
struct CacheEntry {
    lyrics: Lyrics,
    source: LyricsSource,
}

fn cache_dir() -> Option<PathBuf> {
    let base = margo_fs::xdg::cache_home();
    Some(base.join("mshell").join("lyrics"))
}

/// Cache file for `key` under the current source chain. A chain that can
/// reach NetEase caches separately, so turning the fallback on is not
/// shadowed by an lrclib-only "no lyrics" cached earlier (and turning it off
/// again doesn't show NetEase results).
fn cache_path(key: &TrackKey, opts: &FetchOptions) -> Option<PathBuf> {
    let suffix = if opts.netease { "-ne" } else { "" };
    Some(cache_dir()?.join(format!("{}{suffix}.json", key.cache_id())))
}

fn parse_cache(text: &str) -> Option<(Lyrics, LyricsSource)> {
    if let Ok(entry) = serde_json::from_str::<CacheEntry>(text) {
        return Some((entry.lyrics, entry.source));
    }
    serde_json::from_str::<Lyrics>(text)
        .ok()
        .map(|l| (l, LyricsSource::Lrclib))
}

fn load_cache(key: &TrackKey, opts: &FetchOptions) -> Option<(Lyrics, LyricsSource)> {
    parse_cache(&std::fs::read_to_string(cache_path(key, opts)?).ok()?)
}

fn save_cache(key: &TrackKey, opts: &FetchOptions, lyrics: &Lyrics, source: LyricsSource) {
    let Some(path) = cache_path(key, opts) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let entry = CacheEntry {
        lyrics: lyrics.clone(),
        source,
    };
    if let Ok(text) = serde_json::to_string(&entry) {
        let _ = margo_fs::atomic_write::atomic_write(&path, text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_index_tracks_position() {
        let lines = parse_lrc("[00:01.00]a\n[00:05.00]b\n[00:10.00]c");
        assert_eq!(index_for_time(&lines, 0), None);
        assert_eq!(index_for_time(&lines, 1_000), Some(0));
        assert_eq!(index_for_time(&lines, 4_999), Some(0));
        assert_eq!(index_for_time(&lines, 5_000), Some(1));
        assert_eq!(index_for_time(&lines, 99_000), Some(2));
    }

    #[test]
    fn cleans_noise_and_splits_artist() {
        // No artist + "Artist - Title" with bracketed noise.
        let (t, a) = clean_track("Daft Punk - Get Lucky (Official Video)", "");
        assert_eq!(a, "Daft Punk");
        assert_eq!(t, "Get Lucky");

        // "feat. …" dropped, real artist kept.
        let (t, a) = clean_track("Get Lucky feat. Pharrell", "Daft Punk");
        assert_eq!(a, "Daft Punk");
        assert_eq!(t, "Get Lucky");

        // " - Topic" YouTube artist suffix stripped.
        let (_t, a) = clean_track("Some Song", "Daft Punk - Topic");
        assert_eq!(a, "Daft Punk");

        // A non-noise parenthetical is preserved.
        let (t, _a) = clean_track("Hurt (Acoustic)", "Johnny Cash");
        assert_eq!(t, "Hurt (Acoustic)");

        // A title-only key is still valid (artist optional).
        assert!(key_for("Clocks", "", "", 0).is_valid());
    }

    #[test]
    fn embedded_hint_short_circuits_to_plain_lines() {
        let key = key_for("Title", "Artist", "Album", 180);
        let res = fetch(&key, Some("line one\nline two\n"), &FetchOptions::default());
        assert_eq!(res.source, LyricsSource::Embedded);
        assert!(!res.transient);
        match res.lyrics {
            Lyrics::Plain(lines) => assert_eq!(lines, vec!["line one", "line two"]),
            other => panic!("expected Plain, got {other:?}"),
        }
    }

    #[test]
    fn embedded_lrc_stays_synced() {
        let key = key_for("Title", "Artist", "Album", 180);
        let res = fetch(
            &key,
            Some("[00:01.00]<00:01.00>one <00:01.50>two\n[00:03.00]three"),
            &FetchOptions::default(),
        );
        assert_eq!(res.source, LyricsSource::Embedded);
        let Lyrics::Synced(lines) = res.lyrics else {
            panic!("expected Synced");
        };
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words.len(), 2);
    }

    #[test]
    fn blank_or_absent_hint_falls_through_to_lrclib() {
        // An invalid key (no title) short-circuits `fetch_inner` to `None`
        // before it ever touches the network, so this stays offline and
        // deterministic while still proving the non-embedded branch ran.
        let key = key_for("", "", "", 0);
        let opts = FetchOptions::default();

        let res = fetch(&key, None, &opts);
        assert_eq!(res.source, LyricsSource::Lrclib);
        assert_eq!(res.lyrics, Lyrics::None);

        // Whitespace-only embedded text must not be mistaken for real
        // lyrics either.
        let res = fetch(&key, Some("   \n  "), &opts);
        assert_eq!(res.source, LyricsSource::Lrclib);
        assert_eq!(res.lyrics, Lyrics::None);
    }

    fn synced() -> Lyrics {
        Lyrics::Synced(parse_lrc("[00:01.00]a"))
    }
    fn plain() -> Lyrics {
        Lyrics::Plain(vec!["a".into()])
    }

    #[test]
    fn chain_stops_at_lrclib_synced_without_asking_netease() {
        let res = resolve_chain(Some(synced()), || panic!("NetEase must not be asked"), true);
        assert_eq!((res.source, res.transient), (LyricsSource::Lrclib, false));
        let res = resolve_chain(
            Some(Lyrics::Instrumental),
            || panic!("NetEase must not be asked"),
            true,
        );
        assert_eq!(res.lyrics, Lyrics::Instrumental);
    }

    #[test]
    fn chain_without_netease_is_todays_behaviour() {
        let res = resolve_chain(Some(Lyrics::None), || panic!("disabled"), false);
        assert_eq!((res.lyrics, res.transient), (Lyrics::None, false));
        let res = resolve_chain(None, || panic!("disabled"), false);
        assert!(res.transient, "a network error is retried, not cached");
    }

    #[test]
    fn chain_prefers_synced_netease_over_plain_lrclib() {
        let res = resolve_chain(Some(plain()), || Some(synced()), true);
        assert_eq!((res.lyrics, res.source), (synced(), LyricsSource::NetEase));
        let res = resolve_chain(Some(plain()), || Some(Lyrics::None), true);
        assert_eq!((res.lyrics, res.source), (plain(), LyricsSource::Lrclib));
        let res = resolve_chain(Some(Lyrics::None), || Some(plain()), true);
        assert_eq!(res.source, LyricsSource::NetEase);
    }

    #[test]
    fn chain_is_transient_only_when_nothing_definitive() {
        // One source down, the other says "none": not definitive overall.
        assert!(resolve_chain(None, || Some(Lyrics::None), true).transient);
        assert!(resolve_chain(Some(Lyrics::None), || None, true).transient);
        // One source down, the other has lyrics: use them, no retry.
        let res = resolve_chain(None, || Some(synced()), true);
        assert!(!res.transient);
        // Both definitive misses: cache the miss.
        let res = resolve_chain(Some(Lyrics::None), || Some(Lyrics::None), true);
        assert_eq!((res.lyrics, res.transient), (Lyrics::None, false));
    }

    #[test]
    fn retry_backs_off_then_gives_up() {
        let delays: Vec<_> = (0..6).map(retry_delay).collect();
        assert_eq!(delays[0], Some(Duration::from_secs(5)));
        assert!(delays.windows(2).all(|w| match (w[0], w[1]) {
            (Some(a), Some(b)) => b > a,
            (_, None) => true,
            (None, Some(_)) => false,
        }));
        assert_eq!(delays[4], None);
    }

    #[test]
    fn offset_moves_the_lookup_position() {
        assert_eq!(
            effective_position(10_000, 500),
            10_500,
            "positive = earlier lines"
        );
        assert_eq!(effective_position(10_000, -500), 9_500);
        assert_eq!(effective_position(100, -500), 0, "saturates");
        assert_eq!(
            effective_position(0, 99_999),
            MAX_OFFSET_MS as u64,
            "clamped"
        );
    }

    #[test]
    fn seeking_to_a_line_inverts_the_offset() {
        for offset in [-1_500, 0, 750] {
            let pos = seek_position_for(30_000, offset);
            let lines = parse_lrc("[00:29.00]a\n[00:30.00]b\n[00:31.00]c");
            assert_eq!(
                index_for_time(&lines, effective_position(pos, offset)),
                Some(1),
                "offset {offset}"
            );
        }
        assert_eq!(seek_position_for(100, 500), 0, "saturates");
        assert_eq!(total_offset(9_000, 5_000), MAX_OFFSET_MS);
        assert_eq!(total_offset(-250, 500), 250);
    }

    #[test]
    fn player_filter_lists() {
        let none: Vec<String> = Vec::new();
        let spotify = ("Spotify", "org.mpris.MediaPlayer2.spotify");
        let firefox = (
            "Mozilla Firefox",
            "org.mpris.MediaPlayer2.firefox.instance_1_42",
        );
        assert!(player_allowed(spotify.0, spotify.1, &none, &none));
        let allow = vec!["spotify".to_string(), " ".to_string()];
        assert!(player_allowed(spotify.0, spotify.1, &allow, &none));
        assert!(!player_allowed(firefox.0, firefox.1, &allow, &none));
        let block = vec!["FIREFOX".to_string()];
        assert!(!player_allowed(firefox.0, firefox.1, &none, &block));
        assert!(player_allowed(spotify.0, spotify.1, &none, &block));
        // Blank entries never match everything.
        let blank = vec![String::new()];
        assert!(player_allowed(firefox.0, firefox.1, &blank, &blank));
    }

    #[test]
    fn local_folder_wins_over_everything_but_embedded() {
        let d = std::env::temp_dir().join(format!("mshell-lyrics-chain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Song.lrc"), "[00:01.00]mine").unwrap();
        let opts = FetchOptions {
            netease: false,
            local_dir: Some(d.clone()),
        };
        let key = key_for("Song", "", "", 0);
        let res = fetch(&key, None, &opts);
        assert_eq!(res.source, LyricsSource::Local);
        assert!(matches!(res.lyrics, Lyrics::Synced(_)));
        let res = fetch(&key, Some("embedded words"), &opts);
        assert_eq!(res.source, LyricsSource::Embedded);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn play_clock_extrapolates_while_playing_only() {
        let t0 = Instant::now();
        let mut c = PlayClock::new(t0);
        c.sample(10_000, t0);
        assert_eq!(
            c.at(t0 + Duration::from_millis(400)),
            10_000,
            "paused holds"
        );
        c.set_playing(true, t0);
        assert_eq!(c.at(t0 + Duration::from_millis(400)), 10_400);
        assert_eq!(
            c.at(t0 + Duration::from_secs(60)),
            10_000 + PlayClock::MAX_EXTRAPOLATION.as_millis() as u64,
            "a stalled player can't run the lyrics away"
        );
        c.set_playing(false, t0 + Duration::from_millis(300));
        assert_eq!(
            c.at(t0 + Duration::from_secs(5)),
            10_300,
            "pause freezes the estimate"
        );
    }

    #[test]
    fn cache_reads_old_and_new_shapes() {
        let old = serde_json::to_string(&Lyrics::Synced(parse_lrc("[00:01.00]a"))).unwrap();
        let (l, s) = parse_cache(&old).unwrap();
        assert_eq!(s, LyricsSource::Lrclib);
        assert!(matches!(l, Lyrics::Synced(_)));
        // An old cached line (no word/translation fields) still loads.
        let legacy = r#"{"Synced":[{"time_ms":1000,"text":"a"}]}"#;
        assert!(parse_cache(legacy).is_some());
        let new = serde_json::to_string(&CacheEntry {
            lyrics: Lyrics::Instrumental,
            source: LyricsSource::NetEase,
        })
        .unwrap();
        assert_eq!(
            parse_cache(&new),
            Some((Lyrics::Instrumental, LyricsSource::NetEase))
        );
    }

    #[test]
    fn lrclib_fixture_parses_synced() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/lrclib_get_get_lucky.json")).unwrap();
        let Lyrics::Synced(lines) = lyrics_from_json(&v) else {
            panic!("expected synced");
        };
        assert_eq!(lines[0].time_ms, 30_640);
        assert_eq!(lines[0].text, "Like the legend of the phoenix, huh");
    }
}
