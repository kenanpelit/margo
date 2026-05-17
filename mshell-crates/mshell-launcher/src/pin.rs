//! Persistent pin/unpin store for launcher items.
//!
//! Users mark frequently-used items as "pinned" via Ctrl+P in the
//! UI. Pinned items rank at the top of empty-browse mode regardless
//! of frecency, with a ★ glyph marker. The set survives across
//! sessions via a small JSON file at
//! `$XDG_CACHE_HOME/margo/launcher_pins.json` (falling back to
//! `~/.cache/margo/launcher_pins.json` when the env var is unset).
//!
//! Mirrors `FrecencyStore`'s shape — load on startup, mutate in
//! memory, flush on launcher close (or on every change for the
//! `set`/`unset` paths since the set is tiny). Atomic write via
//! temp + rename so a half-written file can never corrupt the
//! state on next launch.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Disk-backed set of pinned `usage_key`s. The key is whatever the
/// provider stamped in [`LauncherItem::usage_key`] — typically
/// `apps:firefox.desktop`, `scripts:start-brave`, etc.
///
/// [`LauncherItem::usage_key`]: crate::item::LauncherItem::usage_key
#[derive(Debug)]
pub struct PinStore {
    path: PathBuf,
    set: BTreeSet<String>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct Disk {
    pinned: BTreeSet<String>,
}

impl PinStore {
    /// Load from the canonical user cache path. Missing / malformed
    /// files are treated as "no pins yet".
    pub fn load() -> Self {
        Self::load_from(default_path())
    }

    /// Load from a caller-provided path. Tests use this with a
    /// temp file; the default ctor calls it with [`default_path`].
    pub fn load_from(path: PathBuf) -> Self {
        let set = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Disk>(&raw).ok())
            .map(|d| d.pinned)
            .unwrap_or_default();
        Self { path, set }
    }

    /// True if the given key has been pinned.
    pub fn is_pinned(&self, key: &str) -> bool {
        self.set.contains(key)
    }

    /// Snapshot of pinned keys, sorted (BTreeSet iter is sorted).
    /// Used by the runtime to bubble pinned items to the top of
    /// browse mode in deterministic order.
    pub fn keys(&self) -> Vec<String> {
        self.set.iter().cloned().collect()
    }

    /// Pin a key. No-op if already pinned. Flushes to disk
    /// immediately — pin operations are rare (one Ctrl+P per
    /// pin/unpin action), so the extra write cost is irrelevant.
    pub fn pin(&mut self, key: &str) {
        if self.set.insert(key.to_string()) {
            self.flush();
        }
    }

    /// Unpin a key. No-op if not pinned. Flushes on change.
    pub fn unpin(&mut self, key: &str) {
        if self.set.remove(key) {
            self.flush();
        }
    }

    /// Toggle pin state for a key. Returns the new state
    /// (`true` = now pinned). Flushes on change.
    pub fn toggle(&mut self, key: &str) -> bool {
        let now_pinned = if self.set.contains(key) {
            self.set.remove(key);
            false
        } else {
            self.set.insert(key.to_string());
            true
        };
        self.flush();
        now_pinned
    }

    /// Write the current set to disk atomically. Best-effort: a
    /// failed write logs at warn level and doesn't poison the
    /// in-memory state.
    pub fn flush(&self) {
        if let Some(parent) = self.path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            tracing::warn!(path = %parent.display(), error = %err, "launcher_pins: mkdir failed");
            return;
        }
        let disk = Disk { pinned: self.set.clone() };
        let json = match serde_json::to_string_pretty(&disk) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = %err, "launcher_pins: serialize failed");
                return;
            }
        };
        let tmp = self.path.with_extension("json.tmp");
        if let Err(err) = std::fs::write(&tmp, &json) {
            tracing::warn!(path = %tmp.display(), error = %err, "launcher_pins: tmp write failed");
            return;
        }
        if let Err(err) = std::fs::rename(&tmp, &self.path) {
            tracing::warn!(from = %tmp.display(), to = %self.path.display(), error = %err, "launcher_pins: rename failed");
        }
    }
}

/// Default on-disk location for the pin set: respects
/// `$XDG_CACHE_HOME` first, then `~/.cache`, then `/tmp` as a final
/// fallback.
fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("margo").join("launcher_pins.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ephemeral() -> PinStore {
        let path = std::env::temp_dir().join(format!(
            "mshell_launcher_pins_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        PinStore::load_from(path)
    }

    #[test]
    fn pin_then_query() {
        let mut s = ephemeral();
        assert!(!s.is_pinned("apps:firefox.desktop"));
        s.pin("apps:firefox.desktop");
        assert!(s.is_pinned("apps:firefox.desktop"));
    }

    #[test]
    fn toggle_alternates() {
        let mut s = ephemeral();
        assert!(s.toggle("k"));
        assert!(!s.toggle("k"));
        assert!(s.toggle("k"));
    }

    #[test]
    fn survives_reload() {
        let path = std::env::temp_dir().join(format!(
            "mshell_launcher_pins_reload_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        let mut s = PinStore::load_from(path.clone());
        s.pin("a");
        s.pin("b");
        drop(s);
        let s2 = PinStore::load_from(path);
        assert!(s2.is_pinned("a"));
        assert!(s2.is_pinned("b"));
        assert!(!s2.is_pinned("c"));
    }
}
