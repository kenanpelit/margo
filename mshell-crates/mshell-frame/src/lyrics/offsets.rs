//! Per-track lyric timing nudges — the lyrics menu's `−` / `+` stepper.
//!
//! A published LRC is often out of sync for *one* recording (a different
//! master, a radio edit), so the fix belongs to that track, not to every
//! song: the global `bars.widgets.lyrics.offset_ms` covers a source that is
//! always off, these nudges cover one file. Stored as one small JSON map
//! (`track cache id → ms`) in `~/.cache/mshell/lyrics/offsets.json`, loaded
//! once and shared by the pill and the menu in-process, so a nudge in the
//! menu moves the bar line at once.

use super::{MAX_OFFSET_MS, TrackKey};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// One `−` / `+` press.
pub(crate) const STEP_MS: i32 = 250;

fn store() -> &'static Mutex<HashMap<String, i32>> {
    static STORE: OnceLock<Mutex<HashMap<String, i32>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(load().unwrap_or_default()))
}

fn path() -> PathBuf {
    margo_fs::xdg::cache_home()
        .join("mshell")
        .join("lyrics")
        .join("offsets.json")
}

fn load() -> Option<HashMap<String, i32>> {
    serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()
}

/// The nudge saved for `key` (0 when none).
pub(crate) fn get(key: &TrackKey) -> i32 {
    store()
        .lock()
        .map(|m| m.get(&key.cache_id()).copied().unwrap_or(0))
        .unwrap_or(0)
}

/// Apply `delta` to `key`'s nudge (`delta == 0` resets it), persist, and
/// return the new value.
pub(crate) fn nudge(key: &TrackKey, delta: i32) -> i32 {
    let Ok(mut map) = store().lock() else {
        return 0;
    };
    let id = key.cache_id();
    let next = step(map.get(&id).copied().unwrap_or(0), delta);
    if next == 0 {
        map.remove(&id);
    } else {
        map.insert(id, next);
    }
    drop(map);
    persist();
    next
}

/// Write the store off the GTK thread. Each writer serialises the *current*
/// map under a write lock, so overlapping presses can't land an older
/// snapshot last.
fn persist() {
    static WRITE: Mutex<()> = Mutex::new(());
    std::thread::spawn(|| {
        let Ok(_guard) = WRITE.lock() else { return };
        let Some(text) = store()
            .lock()
            .ok()
            .and_then(|m| serde_json::to_string(&*m).ok())
        else {
            return;
        };
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = margo_fs::atomic_write::atomic_write(&p, text.as_bytes());
    });
}

/// The next nudge value: `delta == 0` resets, otherwise add and clamp.
pub(crate) fn step(current: i32, delta: i32) -> i32 {
    if delta == 0 {
        0
    } else {
        current
            .saturating_add(delta)
            .clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS)
    }
}

/// `+0.25 s` / `−1.50 s` / `±0 s` for the stepper label.
pub(crate) fn format(ms: i32) -> String {
    if ms == 0 {
        return "±0 s".to_string();
    }
    let sign = if ms > 0 { '+' } else { '−' };
    format!("{sign}{:.2} s", f64::from(ms.unsigned_abs()) / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_adds_clamps_and_resets() {
        assert_eq!(step(0, STEP_MS), 250);
        assert_eq!(step(250, -STEP_MS), 0);
        assert_eq!(step(500, 0), 0, "0 resets");
        assert_eq!(step(MAX_OFFSET_MS, STEP_MS), MAX_OFFSET_MS);
        assert_eq!(step(-MAX_OFFSET_MS, -STEP_MS), -MAX_OFFSET_MS);
    }

    #[test]
    fn formats_with_sign() {
        assert_eq!(format(0), "±0 s");
        assert_eq!(format(250), "+0.25 s");
        assert_eq!(format(-1500), "−1.50 s");
    }
}
