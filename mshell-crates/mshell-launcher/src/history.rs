//! Persistent MRU (most-recently-used) history of shell commands
//! run via the `>cmd` Command provider.
//!
//! Stored as a JSON array on disk so the user can edit / clear it
//! with `jq` or by deleting the file. Capped at
//! [`CommandHistory::MAX_ENTRIES`] entries — bumping an existing
//! entry moves it to the front (MRU) rather than appending a
//! duplicate.
//!
//! The store is read once at launcher construction and written
//! after every bump (atomic tmp-file + rename so a crash can't
//! corrupt the file).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    /// Most-recently-used first. Each entry is the raw shell
    /// expression as the user typed it (without the `>cmd `
    /// prefix).
    #[serde(default)]
    entries: Vec<String>,
}

#[derive(Debug)]
pub struct CommandHistory {
    path: PathBuf,
    inner: StoreFile,
    dirty: bool,
}

impl CommandHistory {
    /// Most-recently-used commands kept around. Older entries
    /// drop off the back. 100 is plenty without making the
    /// history search feel sluggish.
    pub const MAX_ENTRIES: usize = 100;

    /// Load the store from the default on-disk location. Missing
    /// or corrupt files yield an empty history rather than an
    /// error.
    pub fn load() -> Self {
        Self::load_from(default_path())
    }

    /// Test-friendly constructor that lets callers pin the
    /// backing file path.
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

    /// Snapshot of every stored expression, MRU first. Safe to
    /// hold across calls — the returned slice never invalidates
    /// because `&self` borrows the store.
    pub fn entries(&self) -> &[String] {
        &self.inner.entries
    }

    /// Promote (or insert) `expression` to the front of the MRU
    /// list. Trims to the cap and marks the store dirty.
    pub fn bump(&mut self, expression: &str) {
        let trimmed = expression.trim();
        if trimmed.is_empty() {
            return;
        }
        // Remove any existing copy so the same command can't
        // occupy two slots; then push to front.
        self.inner.entries.retain(|e| e != trimmed);
        self.inner.entries.insert(0, trimmed.to_string());
        if self.inner.entries.len() > Self::MAX_ENTRIES {
            self.inner.entries.truncate(Self::MAX_ENTRIES);
        }
        self.dirty = true;
    }

    /// Persist the store if anything changed. Idempotent.
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
                    tracing::warn!(?err, path = ?self.path, "command history write failed");
                } else {
                    self.dirty = false;
                }
            }
            Err(err) => {
                tracing::warn!(?err, "command history serialise failed");
            }
        }
    }
}

impl Drop for CommandHistory {
    fn drop(&mut self) {
        self.flush();
    }
}

fn default_path() -> PathBuf {
    dirs::cache_dir()
        .map(|d| d.join("margo").join("launcher_command_history.json"))
        .unwrap_or_else(|| PathBuf::from("/tmp/margo_launcher_command_history.json"))
}

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
        let dir = std::env::temp_dir().join(format!(
            "mshell_launcher_history_test_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{name}.json"))
    }

    #[test]
    fn empty_history_returns_empty_slice() {
        let path = tmp_path("empty");
        let _ = std::fs::remove_file(&path);
        let h = CommandHistory::load_from(path);
        assert!(h.entries().is_empty());
    }

    #[test]
    fn bump_pushes_to_front() {
        let path = tmp_path("bump-front");
        let _ = std::fs::remove_file(&path);
        let mut h = CommandHistory::load_from(path);
        h.bump("ls");
        h.bump("vim");
        h.bump("git status");
        assert_eq!(h.entries(), &["git status", "vim", "ls"]);
    }

    #[test]
    fn bump_dedups_existing_entry() {
        let path = tmp_path("dedup");
        let _ = std::fs::remove_file(&path);
        let mut h = CommandHistory::load_from(path);
        h.bump("ls");
        h.bump("vim");
        h.bump("ls"); // re-use ls → should move to front, not duplicate
        assert_eq!(h.entries(), &["ls", "vim"]);
    }

    #[test]
    fn bump_persists_across_reload() {
        let path = tmp_path("persist");
        let _ = std::fs::remove_file(&path);

        let mut h = CommandHistory::load_from(path.clone());
        h.bump("echo hello");
        h.bump("ls -la");
        h.flush();
        drop(h);

        let reloaded = CommandHistory::load_from(path);
        assert_eq!(reloaded.entries(), &["ls -la", "echo hello"]);
    }

    #[test]
    fn cap_drops_oldest_entries() {
        let path = tmp_path("cap");
        let _ = std::fs::remove_file(&path);
        let mut h = CommandHistory::load_from(path);
        for i in 0..(CommandHistory::MAX_ENTRIES + 10) {
            h.bump(&format!("cmd-{i}"));
        }
        assert_eq!(h.entries().len(), CommandHistory::MAX_ENTRIES);
        // Newest entry is at the front; oldest 10 were dropped.
        assert_eq!(h.entries()[0], format!("cmd-{}", CommandHistory::MAX_ENTRIES + 9));
    }

    #[test]
    fn empty_bump_is_noop() {
        let path = tmp_path("emptybump");
        let _ = std::fs::remove_file(&path);
        let mut h = CommandHistory::load_from(path);
        h.bump("");
        h.bump("   ");
        assert!(h.entries().is_empty());
    }
}
