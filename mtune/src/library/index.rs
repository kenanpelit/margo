// SPDX-License-Identifier: GPL-3.0-or-later
//! On-disk tag cache (`~/.cache/margo/mtune/index.json`). Lets a large
//! library skip a full metadata re-read on every launch; entries are
//! invalidated per-file by mtime.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub path: PathBuf,
    pub mtime: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LibraryIndex {
    pub entries: Vec<IndexEntry>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reconcile {
    /// In `found`, not in the index.
    pub added: Vec<PathBuf>,
    /// In the index, no longer in `found`.
    pub removed: Vec<PathBuf>,
    /// In both, but the file's mtime changed since it was indexed.
    pub stale: Vec<PathBuf>,
}

/// Seconds since the epoch of `path`'s last modification, if it exists.
pub fn mtime_of(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

impl LibraryIndex {
    /// `$XDG_CACHE_HOME/margo/mtune/index.json`, falling back to `~/.cache/…`.
    pub fn path() -> PathBuf {
        let base = std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache")
            });
        base.join("margo").join("mtune").join("index.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(p: &Path) -> Self {
        std::fs::read(p)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, p: &Path) -> anyhow::Result<()> {
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = serde_json::to_vec_pretty(self)?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, p)?;
        Ok(())
    }

    /// Indexed paths whose file still exists with an unchanged mtime.
    pub fn fresh_paths(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .filter(|e| mtime_of(&e.path) == Some(e.mtime))
            .map(|e| e.path.clone())
            .collect()
    }

    /// Classify a fresh scan result against the index.
    pub fn reconcile(&self, found: &[PathBuf]) -> Reconcile {
        use std::collections::{HashMap, HashSet};
        let indexed: HashMap<&PathBuf, u64> =
            self.entries.iter().map(|e| (&e.path, e.mtime)).collect();
        let found_set: HashSet<&PathBuf> = found.iter().collect();
        let mut r = Reconcile::default();
        for f in found {
            match indexed.get(f) {
                None => r.added.push(f.clone()),
                Some(&m) if mtime_of(f) != Some(m) => r.stale.push(f.clone()),
                Some(_) => {}
            }
        }
        for e in &self.entries {
            if !found_set.contains(&e.path) {
                r.removed.push(e.path.clone());
            }
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn entry(path: PathBuf, mtime: u64) -> IndexEntry {
        IndexEntry {
            path,
            mtime,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration_secs: 0,
        }
    }

    #[test]
    fn save_load_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("index.json");
        let mut idx = LibraryIndex::default();
        idx.entries.push(IndexEntry {
            path: PathBuf::from("/m/a.mp3"),
            mtime: 111,
            title: "A".into(),
            artist: "B".into(),
            album: "C".into(),
            duration_secs: 200,
        });
        idx.save_to(&p).unwrap();
        let back = LibraryIndex::load_from(&p);
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].album, "C");
    }

    #[test]
    fn load_corrupt_is_empty() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("index.json");
        fs::write(&p, b"{ not json").unwrap();
        assert!(LibraryIndex::load_from(&p).entries.is_empty());
    }

    #[test]
    fn reconcile_classifies_added_removed_stale() {
        let d = tempfile::tempdir().unwrap();
        let keep = d.path().join("keep.mp3");
        let stale = d.path().join("stale.mp3");
        fs::write(&keep, b"x").unwrap();
        fs::write(&stale, b"x").unwrap();
        let mut idx = LibraryIndex::default();
        idx.entries
            .push(entry(keep.clone(), mtime_of(&keep).unwrap()));
        idx.entries.push(entry(stale.clone(), 1)); // wrong mtime on purpose
        idx.entries.push(entry(d.path().join("gone.mp3"), 1));

        let new_file = d.path().join("new.mp3");
        fs::write(&new_file, b"x").unwrap();
        let found = vec![keep.clone(), stale.clone(), new_file.clone()];
        let r = idx.reconcile(&found);
        assert_eq!(r.added, vec![new_file]);
        assert_eq!(r.removed, vec![d.path().join("gone.mp3")]);
        assert_eq!(r.stale, vec![stale]);
    }

    #[test]
    fn fresh_paths_excludes_changed_and_missing() {
        let d = tempfile::tempdir().unwrap();
        let ok = d.path().join("ok.mp3");
        fs::write(&ok, b"x").unwrap();
        let mut idx = LibraryIndex::default();
        idx.entries.push(entry(ok.clone(), mtime_of(&ok).unwrap()));
        idx.entries.push(entry(d.path().join("missing.mp3"), 1));
        assert_eq!(idx.fresh_paths(), vec![ok]);
    }
}
