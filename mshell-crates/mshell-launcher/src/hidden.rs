//! Persistent hide/unhide store for launcher items.
//!
//! Twin of [`PinStore`]: a small JSON file at
//! `$XDG_CACHE_HOME/margo/launcher_hidden.json` (or `~/.cache/...`)
//! that records `usage_key`s the user wants suppressed from the
//! empty-browse list. The right-click context menu in the launcher
//! row binds **Hide** / **Unhide** to [`Self::toggle`].
//!
//! Behavioural contract:
//! - `browse(empty)`        → hidden items are filtered OUT.
//! - `search(non-empty)`    → hidden items are STILL INCLUDED.
//!   (Hide is a "don't suggest" hint, not a "delete". A user who
//!   explicitly types the app name should still find it.)
//!
//! Mirror of [`PinStore`] otherwise — atomic temp+rename writes,
//! best-effort flushes, sorted `BTreeSet` for deterministic iteration.
//!
//! [`PinStore`]: crate::pin::PinStore

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Disk-backed set of hidden `usage_key`s.
#[derive(Debug)]
pub struct HiddenStore {
    path: PathBuf,
    set: BTreeSet<String>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct Disk {
    hidden: BTreeSet<String>,
}

impl HiddenStore {
    /// Load from the canonical user cache path.
    pub fn load() -> Self {
        Self::load_from(default_path())
    }

    /// Load from a caller-provided path (tests).
    pub fn load_from(path: PathBuf) -> Self {
        let set = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Disk>(&raw).ok())
            .map(|d| d.hidden)
            .unwrap_or_default();
        Self { path, set }
    }

    /// True if the given key has been hidden.
    pub fn is_hidden(&self, key: &str) -> bool {
        self.set.contains(key)
    }

    /// Toggle hide state for a key. Returns the new state
    /// (`true` = now hidden).
    pub fn toggle(&mut self, key: &str) -> bool {
        let now_hidden = if self.set.contains(key) {
            self.set.remove(key);
            false
        } else {
            self.set.insert(key.to_string());
            true
        };
        self.flush();
        now_hidden
    }

    /// Drop a key from the hidden set (no-op when not hidden).
    /// Used by `delete_item` so a deleted entry doesn't leak a
    /// stale hidden flag.
    pub fn unhide(&mut self, key: &str) {
        if self.set.remove(key) {
            self.flush();
        }
    }

    /// Write the current set to disk atomically.
    pub fn flush(&self) {
        if let Some(parent) = self.path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            tracing::warn!(path = %parent.display(), error = %err, "launcher_hidden: mkdir failed");
            return;
        }
        let disk = Disk {
            hidden: self.set.clone(),
        };
        let json = match serde_json::to_string_pretty(&disk) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = %err, "launcher_hidden: serialize failed");
                return;
            }
        };
        let tmp = self.path.with_extension("json.tmp");
        if let Err(err) = std::fs::write(&tmp, &json) {
            tracing::warn!(path = %tmp.display(), error = %err, "launcher_hidden: tmp write failed");
            return;
        }
        if let Err(err) = std::fs::rename(&tmp, &self.path) {
            tracing::warn!(from = %tmp.display(), to = %self.path.display(), error = %err, "launcher_hidden: rename failed");
        }
    }
}

fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("margo").join("launcher_hidden.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ephemeral() -> HiddenStore {
        let path = std::env::temp_dir().join(format!(
            "mshell_launcher_hidden_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        HiddenStore::load_from(path)
    }

    #[test]
    fn hide_then_query() {
        let mut s = ephemeral();
        assert!(!s.is_hidden("apps:firefox.desktop"));
        s.toggle("apps:firefox.desktop");
        assert!(s.is_hidden("apps:firefox.desktop"));
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
            "mshell_launcher_hidden_reload_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        let mut s = HiddenStore::load_from(path.clone());
        s.toggle("a");
        s.toggle("b");
        drop(s);
        let s2 = HiddenStore::load_from(path);
        assert!(s2.is_hidden("a"));
        assert!(s2.is_hidden("b"));
        assert!(!s2.is_hidden("c"));
    }
}
