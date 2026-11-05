//! Local `.lrc` files — lyrics the user keeps on disk, for players that
//! don't embed them (mpv, browsers, Spotify…). Asked before the cache and
//! the network when `bars.widgets.lyrics.local_dir` is set.
//!
//! Lookup inside that folder, first hit wins (names compared
//! case-insensitively, `/` in a tag replaced so it can't escape the folder):
//! `Artist - Title.lrc`, `Artist/Title.lrc`, `Title.lrc`, then the same with
//! `.txt` (plain text).

use super::{Lyrics, TrackKey, parse_lrc};
use std::path::{Path, PathBuf};

/// Expand a leading `~` / `~/` against `$HOME`.
pub(crate) fn expand_dir(raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if raw == "~" {
        return margo_fs::xdg::home();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return margo_fs::xdg::home().map(|h| h.join(rest));
    }
    Some(PathBuf::from(raw))
}

/// A tag value as one safe path component.
fn component(s: &str) -> String {
    s.trim()
        .chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect()
}

/// Relative candidate names for `key`, in lookup order.
pub(crate) fn candidates(key: &TrackKey) -> Vec<PathBuf> {
    let title = component(&key.title);
    let artist = component(&key.artist);
    if title.is_empty() {
        return Vec::new();
    }
    let mut stems: Vec<PathBuf> = Vec::new();
    if !artist.is_empty() {
        stems.push(PathBuf::from(format!("{artist} - {title}")));
        stems.push(Path::new(&artist).join(&title));
    }
    stems.push(PathBuf::from(&title));
    let mut out = Vec::new();
    for ext in ["lrc", "txt"] {
        out.extend(stems.iter().map(|s| s.with_extension(ext)));
    }
    out
}

/// Resolve `rel` under `dir` case-insensitively, one component at a time.
fn find_ci(dir: &Path, rel: &Path) -> Option<PathBuf> {
    let mut cur = dir.to_path_buf();
    for part in rel.components() {
        let want = part.as_os_str().to_string_lossy().to_lowercase();
        let exact = cur.join(part);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let hit = std::fs::read_dir(&cur)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().to_lowercase() == want)?;
        cur = hit.path();
    }
    cur.is_file().then_some(cur)
}

/// The first local lyrics file for `key` under `dir`, parsed. `None` when
/// none exists or it holds nothing usable.
pub(crate) fn lookup(dir: &Path, key: &TrackKey) -> Option<Lyrics> {
    for rel in candidates(key) {
        let Some(path) = find_ci(dir, &rel) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lines = parse_lrc(&text);
        if !lines.is_empty() {
            return Some(Lyrics::Synced(lines));
        }
        let plain: Vec<String> = text.lines().map(|l| l.trim().to_string()).collect();
        if plain.iter().any(|l| !l.is_empty()) {
            return Some(Lyrics::Plain(plain));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::key_for;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mshell-lyrics-local-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn candidate_order_and_sanitising() {
        let key = key_for("AC/DC Song", "AC/DC", "", 0);
        let c = candidates(&key);
        assert_eq!(c[0], PathBuf::from("AC_DC - AC_DC Song.lrc"));
        assert_eq!(c[1], PathBuf::from("AC_DC/AC_DC Song.lrc"));
        assert_eq!(c[2], PathBuf::from("AC_DC Song.lrc"));
        assert!(c[3..].iter().all(|p| p.extension().unwrap() == "txt"));
        assert!(candidates(&key_for("", "x", "", 0)).is_empty());
    }

    #[test]
    fn finds_lrc_case_insensitively_and_prefers_artist_title() {
        let d = tmp();
        std::fs::write(d.join("get lucky.lrc"), "[00:01.00]title only").unwrap();
        std::fs::create_dir_all(d.join("daft punk")).unwrap();
        std::fs::write(d.join("daft punk/GET LUCKY.lrc"), "[00:02.00]in artist dir").unwrap();
        let key = key_for("Get Lucky", "Daft Punk", "", 0);
        let Some(Lyrics::Synced(lines)) = lookup(&d, &key) else {
            panic!("expected synced");
        };
        assert_eq!(lines[0].text, "in artist dir");
        std::fs::write(d.join("Daft Punk - Get Lucky.lrc"), "[00:03.00]flat").unwrap();
        let Some(Lyrics::Synced(lines)) = lookup(&d, &key) else {
            panic!("expected synced");
        };
        assert_eq!(lines[0].text, "flat");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn plain_text_file_is_plain_and_missing_is_none() {
        let d = tmp();
        let key = key_for("Clocks", "", "", 0);
        assert_eq!(lookup(&d, &key), None);
        std::fs::write(d.join("Clocks.txt"), "lights go out\n").unwrap();
        assert_eq!(
            lookup(&d, &key),
            Some(Lyrics::Plain(vec!["lights go out".to_string()]))
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn expands_home() {
        assert_eq!(expand_dir("  "), None);
        assert_eq!(expand_dir("/srv/lrc"), Some(PathBuf::from("/srv/lrc")));
        assert_eq!(
            expand_dir("~/Music/lyrics"),
            margo_fs::xdg::home().map(|h| h.join("Music/lyrics"))
        );
    }
}
