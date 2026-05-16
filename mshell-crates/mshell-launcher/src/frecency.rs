//! Disk-persisted usage counters for "most used" sorting.
//!
//! Each call to [`FrecencyStore::bump`] increments a per-key
//! counter and (lazily) writes the whole map back to
//! `$XDG_CACHE_HOME/margo/launcher_usage.json`. Reads return 0 for
//! unknown keys so providers can call [`FrecencyStore::count`] on
//! every item without branching.
//!
//! The format is intentionally trivial JSON (`{ "key": count }`)
//! so an external tool — or the user with `jq` — can edit or reset
//! counters without launching mshell. We *do not* track timestamps
//! yet: a true frecency (frequency × recency) algorithm would be a
//! follow-up.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Backing JSON layout. Wrapping the map in a struct gives us room
/// to add fields later (last-seen timestamps, schema version) with
/// a forward-compatible deserialiser.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    counts: HashMap<String, u64>,
}

/// In-memory cache + disk-write coordinator for launcher usage
/// counts.
#[derive(Debug)]
pub struct FrecencyStore {
    path: PathBuf,
    inner: StoreFile,
    /// Set by `bump`, cleared by `flush`. Lets the runtime call
    /// `flush` opportunistically (e.g. on launcher close) without
    /// touching disk when nothing changed.
    dirty: bool,
}

impl FrecencyStore {
    /// Load the on-disk store. A missing or unreadable file yields
    /// an empty store rather than an error — first-run users
    /// shouldn't see a crash because they never bumped anything.
    pub fn load() -> Self {
        let path = default_path();
        Self::load_from(path)
    }

    /// Test-friendly constructor that lets callers pin the backing
    /// file path. Production code should use [`FrecencyStore::load`].
    pub fn load_from(path: PathBuf) -> Self {
        let inner = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<StoreFile>(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            inner,
            dirty: false,
        }
    }

    /// Current usage count for `key`. Returns 0 for unknown keys.
    pub fn count(&self, key: &str) -> u64 {
        self.inner.counts.get(key).copied().unwrap_or(0)
    }

    /// Increment the counter for `key`. Defers disk write until
    /// [`FrecencyStore::flush`].
    pub fn bump(&mut self, key: &str) {
        *self.inner.counts.entry(key.to_string()).or_insert(0) += 1;
        self.dirty = true;
    }

    /// Write the store to disk if anything changed since the last
    /// flush. Errors are logged at warn level but not returned —
    /// failing to record a usage count must never break launching
    /// the app the user just clicked.
    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self.inner) {
            Ok(text) => {
                if let Err(err) = atomic_write(&self.path, text.as_bytes()) {
                    tracing::warn!(?err, path = ?self.path, "frecency store write failed");
                } else {
                    self.dirty = false;
                }
            }
            Err(err) => {
                tracing::warn!(?err, "frecency store serialise failed");
            }
        }
    }
}

impl Drop for FrecencyStore {
    fn drop(&mut self) {
        self.flush();
    }
}

fn default_path() -> PathBuf {
    dirs::cache_dir()
        .map(|d| d.join("margo").join("launcher_usage.json"))
        .unwrap_or_else(|| PathBuf::from("/tmp/margo_launcher_usage.json"))
}

/// Public accessor for the conventional frecency-store path —
/// used by the Settings UI to show the path and offer a "Clear"
/// button. Same logic as the in-process loader.
pub fn store_path() -> PathBuf {
    default_path()
}

/// Remove the on-disk frecency file. Used by the Settings
/// "Clear cache" button. Best-effort: missing file is treated as
/// already-clear and returns `Ok(())`.
pub fn clear_disk() -> std::io::Result<()> {
    match std::fs::remove_file(default_path()) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Write `bytes` to `path` via a tmp-file + rename so a crash mid-
/// write never leaves a half-baked JSON file that fails to parse on
/// the next launch.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mshell_launcher_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{name}.json"))
    }

    #[test]
    fn unknown_key_reads_zero() {
        let path = tmp_path("unknown");
        let _ = std::fs::remove_file(&path);
        let store = FrecencyStore::load_from(path);
        assert_eq!(store.count("never-seen"), 0);
    }

    #[test]
    fn bump_persists_across_reload() {
        let path = tmp_path("persist");
        let _ = std::fs::remove_file(&path);

        let mut store = FrecencyStore::load_from(path.clone());
        store.bump("firefox");
        store.bump("firefox");
        store.bump("kitty");
        store.flush();
        drop(store);

        let reloaded = FrecencyStore::load_from(path);
        assert_eq!(reloaded.count("firefox"), 2);
        assert_eq!(reloaded.count("kitty"), 1);
    }

    #[test]
    fn flush_without_bump_is_noop() {
        let path = tmp_path("noop");
        let _ = std::fs::remove_file(&path);
        let mut store = FrecencyStore::load_from(path.clone());
        store.flush();
        // No bump happened, so the file should not have been
        // created.
        assert!(!path.exists());
    }
}
