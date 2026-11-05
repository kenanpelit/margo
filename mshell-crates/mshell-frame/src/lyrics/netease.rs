//! NetEase Cloud Music (music.163.com) — the optional second lyric source.
//!
//! Off by default (`bars.widgets.lyrics.netease_fallback`); when on it is
//! asked only after lrclib has no *synced* lyrics. Its catalogue is strong
//! for Asian releases and it often carries **word-timed** lyrics (YRC) plus
//! translation / romanization layers, which lrclib never does.
//!
//! Two public, unauthenticated endpoints: `/api/cloudsearch/pc` (song
//! search) and `/api/song/lyric/v1` (lyrics by song id). Both want a browser
//! User-Agent and the music.163.com Referer — a bare request gets an
//! encrypted blob instead of JSON. Blocking (`ureq`): run off the GTK thread.

use super::lrc::{self, Layer};
use super::{Lyrics, TrackKey, call_json};

const BASE: &str = "https://music.163.com";
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const REFERER: &str = "https://music.163.com/";
/// Candidates whose lyrics are tried before giving up on a search.
const MAX_CANDIDATES: usize = 3;

/// One song from a NetEase search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Song {
    pub id: u64,
    pub name: String,
    pub artists: Vec<String>,
    pub duration_ms: u64,
}

/// `Some` = a definitive answer (lyrics, instrumental, or "nothing there");
/// `None` = a transport / API error the caller must not cache.
pub(super) fn fetch(key: &TrackKey) -> Option<Lyrics> {
    let mut queries = Vec::new();
    if !key.artist.is_empty() {
        queries.push(format!("{} {}", key.artist, key.title));
    }
    queries.push(key.title.clone());

    let mut ranked = Vec::new();
    for q in &queries {
        let req = ureq::get(&format!("{BASE}/api/cloudsearch/pc"))
            .query("s", q)
            .query("type", "1")
            .query("limit", "10")
            .query("offset", "0");
        let v = call_json(with_headers(req), BROWSER_UA).ok()??;
        // A non-200 `code` is NetEase's rate limiter / anti-abuse answer, not
        // "no such song" — treat it as transient.
        if v.get("code").and_then(|c| c.as_i64()) != Some(200) {
            return None;
        }
        ranked = rank(parse_search(&v), key);
        if !ranked.is_empty() {
            break;
        }
    }
    if ranked.is_empty() {
        return Some(Lyrics::None);
    }

    let mut first_plain = None;
    let mut instrumental = false;
    let mut transport_error = false;
    for song in ranked.iter().take(MAX_CANDIDATES) {
        let req = ureq::get(&format!("{BASE}/api/song/lyric/v1"))
            .query("id", &song.id.to_string())
            .query("cp", "false");
        let req = ["lv", "kv", "tv", "rv", "yv", "ytv", "yrv"]
            .into_iter()
            .fold(req, |r, k| r.query(k, "0"));
        let payload = match call_json(with_headers(req), BROWSER_UA) {
            Ok(Some(v)) if v.get("code").and_then(|c| c.as_i64()) == Some(200) => v,
            Ok(_) => continue,
            Err(()) => {
                transport_error = true;
                continue;
            }
        };
        match lyrics_from_payload(&payload) {
            Lyrics::Synced(lines) => return Some(Lyrics::Synced(lines)),
            Lyrics::Plain(lines) => {
                first_plain.get_or_insert(lines);
            }
            Lyrics::Instrumental => instrumental = true,
            Lyrics::None => {}
        }
    }
    if let Some(lines) = first_plain {
        return Some(Lyrics::Plain(lines));
    }
    if instrumental {
        return Some(Lyrics::Instrumental);
    }
    if transport_error {
        None
    } else {
        Some(Lyrics::None)
    }
}

fn with_headers(req: ureq::Request) -> ureq::Request {
    req.set("Referer", REFERER)
}

/// Songs from a `/api/cloudsearch/pc` response (missing fields → skipped).
pub(super) fn parse_search(v: &serde_json::Value) -> Vec<Song> {
    v.pointer("/result/songs")
        .and_then(|s| s.as_array())
        .map(|songs| {
            songs
                .iter()
                .filter_map(|s| {
                    Some(Song {
                        id: s.get("id")?.as_u64()?,
                        name: s.get("name")?.as_str()?.to_string(),
                        artists: s
                            .get("ar")
                            .and_then(|a| a.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.get("name")?.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        duration_ms: s.get("dt").and_then(|d| d.as_u64()).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Lower-cased letters and digits only, so "Get Lucky!" == "get lucky".
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Score one search hit against the track; `<= 0` = not this song.
pub(super) fn score(song: &Song, key: &TrackKey) -> i32 {
    const VERSION_WORDS: &[&str] = &[
        "remix",
        "edit",
        "live",
        "instrumental",
        "karaoke",
        "cover",
        "acoustic",
        "mix",
        "伴奏",
    ];
    let want = norm(&key.title);
    let got = norm(&song.name);
    if want.is_empty() || got.is_empty() {
        return 0;
    }
    let mut s = if want == got {
        40
    } else if got.contains(&want) || want.contains(&got) {
        20
    } else {
        return 0;
    };
    if !key.artist.is_empty() {
        let artist = norm(&key.artist);
        let hit =
            song.artists.iter().map(|a| norm(a)).any(|a| {
                !a.is_empty() && (a == artist || artist.contains(&a) || a.contains(&artist))
            });
        s += if hit { 30 } else { -20 };
    }
    if key.duration_secs > 0 && song.duration_ms > 0 {
        let diff = (key.duration_secs * 1000).abs_diff(song.duration_ms);
        s += match diff {
            0..=3_000 => 20,
            3_001..=10_000 => 5,
            10_001..=30_000 => 0,
            _ => -15,
        };
    }
    let name = song.name.to_lowercase();
    let title = key.title.to_lowercase();
    for w in VERSION_WORDS {
        if name.contains(w) && !title.contains(w) {
            s -= 15;
        }
    }
    s
}

/// Hits worth trying, best first.
pub(super) fn rank(songs: Vec<Song>, key: &TrackKey) -> Vec<Song> {
    let mut scored: Vec<(i32, Song)> = songs
        .into_iter()
        .map(|s| (score(&s, key), s))
        .filter(|(sc, _)| *sc > 0)
        .collect();
    scored.sort_by_key(|(sc, _)| std::cmp::Reverse(*sc));
    scored.into_iter().map(|(_, s)| s).collect()
}

/// Turn a `/api/song/lyric/v1` payload into [`Lyrics`], format priority
/// word (YRC) → line (LRC) → plain, with the matching translation /
/// romanization layer merged in and NetEase's credit rows dropped.
pub(super) fn lyrics_from_payload(v: &serde_json::Value) -> Lyrics {
    let field = |name: &str| {
        v.pointer(&format!("/{name}/lyric"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
    };
    let strip = |mut lines: Vec<super::LyricLine>| {
        lines.retain(|l| !lrc::is_credit_line(&l.text) && !l.text.trim().is_empty());
        lines
    };

    // (lines, translation field, romanization field) — each layer is timed
    // against its own base format.
    let (mut lines, tr, ro) = {
        let yrc = strip(lrc::parse_yrc(field("yrc")));
        if yrc.iter().any(|l| !l.words.is_empty()) {
            (yrc, "ytlrc", "yromalrc")
        } else {
            (strip(lrc::parse_lrc(field("lrc"))), "tlyric", "romalrc")
        }
    };

    if lines.is_empty() {
        // Untimed text (no stamps at all) is plain lyrics.
        let plain: Vec<String> = field("lrc")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('{') && !l.starts_with('['))
            .filter(|l| !lrc::is_credit_line(l))
            .map(str::to_string)
            .collect();
        if plain
            .iter()
            .any(|l| l.contains("纯音乐") || l == "暂无歌词")
            && plain.len() <= 2
        {
            return Lyrics::Instrumental;
        }
        return if plain.is_empty() {
            Lyrics::None
        } else {
            Lyrics::Plain(plain)
        };
    }
    if lrc::is_placeholder(&lines) {
        return Lyrics::Instrumental;
    }
    for (name, kind) in [(tr, Layer::Translation), (ro, Layer::Romanization)] {
        let mut layer = lrc::parse_lrc(field(name));
        if layer.is_empty() {
            // `ytlrc` / `yromalrc` are optional; fall back to the LRC-timed
            // layer, which still lands within tolerance for most lines.
            let alt = if kind == Layer::Translation {
                "tlyric"
            } else {
                "romalrc"
            };
            layer = lrc::parse_lrc(field(alt));
        }
        let layer: Vec<_> = layer
            .into_iter()
            .filter(|l| !lrc::is_credit_line(&l.text))
            .collect();
        lrc::merge_layer(&mut lines, &layer, kind);
    }
    Lyrics::Synced(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::key_for;

    fn search_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("fixtures/netease_search_get_lucky.json")).unwrap()
    }

    #[test]
    fn parses_real_search_results() {
        let songs = parse_search(&search_fixture());
        assert_eq!(songs.len(), 5);
        assert_eq!(songs[0].id, 26349642);
        assert_eq!(songs[0].name, "Get Lucky");
        assert_eq!(songs[0].artists[0], "Daft Punk");
        assert_eq!(songs[0].duration_ms, 369_684);
    }

    #[test]
    fn ranks_the_plain_version_over_remixes() {
        // The album version playing: exact title + artist + duration wins;
        // remixes / edits the title doesn't ask for score below it.
        let key = key_for("Get Lucky", "Daft Punk", "", 369);
        let ranked = rank(parse_search(&search_fixture()), &key);
        assert_eq!(ranked[0].id, 26349642, "album version, exact duration");
        assert!(
            ranked
                .iter()
                .position(|s| s.name.contains("Remix"))
                .unwrap_or(usize::MAX)
                > 0,
            "remix never first"
        );
    }

    #[test]
    fn rejects_unrelated_titles() {
        let key = key_for("Harder Better Faster Stronger", "Daft Punk", "", 0);
        assert!(rank(parse_search(&search_fixture()), &key).is_empty());
    }

    #[test]
    fn payload_prefers_word_timed_yrc_with_translation() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/netease_lyric_get_lucky.json")).unwrap();
        let Lyrics::Synced(lines) = lyrics_from_payload(&v) else {
            panic!("expected synced");
        };
        assert!(
            !lines
                .iter()
                .any(|l| l.text.contains("Nile Rodgers") && l.text.contains('：')),
            "credit rows dropped"
        );
        let far = lines
            .iter()
            .find(|l| l.text == "We've come too far")
            .unwrap();
        assert_eq!(far.words.len(), 4);
        assert_eq!(far.translation.as_deref(), Some("行迹致远"));
    }

    #[test]
    fn payload_without_yrc_uses_lrc() {
        let v = serde_json::json!({
            "code": 200,
            "lrc": {"lyric": "[00:01.00]one\n[00:02.00]two"},
            "tlyric": {"lyric": "[00:01.00]bir"},
        });
        let Lyrics::Synced(lines) = lyrics_from_payload(&v) else {
            panic!("expected synced");
        };
        assert_eq!(lines.len(), 2);
        assert!(lines[0].words.is_empty());
        assert_eq!(lines[0].translation.as_deref(), Some("bir"));
    }

    #[test]
    fn payload_placeholder_is_instrumental() {
        let v = serde_json::json!({"code": 200, "lrc": {"lyric": "[00:00.00]纯音乐，请欣赏"}});
        assert_eq!(lyrics_from_payload(&v), Lyrics::Instrumental);
        let v = serde_json::json!({"code": 200, "lrc": {"lyric": ""}});
        assert_eq!(lyrics_from_payload(&v), Lyrics::None);
    }
}
