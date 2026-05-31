//! On-disk clipboard persistence.
//!
//! Layout under `$XDG_DATA_HOME/mshell/clipboard/` (default
//! `~/.local/share/mshell/clipboard/`):
//!   - `history.json` — array of [`PersistedEntry`] (metadata +
//!     inline text). Newest first.
//!   - `blobs/<content_hash>.bin` — raw bytes for image / binary
//!     entries (content-addressed, written once).
//!
//! What gets written is governed by [`PersistMode`]: `None` wipes
//! the store, `FavoritesOnly` keeps only pinned entries, `All`
//! keeps the whole rolling history.

use std::fs;
use std::path::PathBuf;

use time::OffsetDateTime;
use tracing::{debug, warn};

use crate::entry::{ClipboardEntry, EntryPreview, PersistedEntry};
use crate::settings::PersistMode;

fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".local/share")
        });
    base.join("mshell/clipboard")
}

fn history_path() -> PathBuf {
    data_dir().join("history.json")
}

fn blobs_dir() -> PathBuf {
    data_dir().join("blobs")
}

fn blob_path(content_hash: u64) -> PathBuf {
    blobs_dir().join(format!("{content_hash:016x}.bin"))
}

/// Persist the given entries according to `mode`. `entries` is the
/// full current history (newest first); this picks what to keep.
pub fn save(entries: &[ClipboardEntry], mode: PersistMode) {
    if mode == PersistMode::None {
        wipe();
        return;
    }

    let keep: Vec<&ClipboardEntry> = match mode {
        PersistMode::FavoritesOnly => entries.iter().filter(|e| e.pinned).collect(),
        PersistMode::All => entries.iter().collect(),
        PersistMode::None => unreachable!(),
    };

    let dir = data_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        warn!("clipboard: create data dir failed: {e}");
        return;
    }

    // Write blobs for non-text payloads (content-addressed — skip
    // if already present).
    let blobs = blobs_dir();
    let _ = fs::create_dir_all(&blobs);
    for e in &keep {
        if !e.is_text() {
            let p = blob_path(e.content_hash);
            if !p.exists()
                && let Err(err) = fs::write(&p, &e.data)
            {
                warn!("clipboard: blob write failed: {err}");
            }
        }
    }

    let persisted: Vec<PersistedEntry> =
        keep.iter().map(|e| PersistedEntry::from_entry(e)).collect();

    // Garbage-collect orphan blobs no longer referenced.
    gc_blobs(&persisted);

    match serde_json::to_vec_pretty(&persisted) {
        Ok(bytes) => {
            // Write to a temp file then rename for atomicity.
            let tmp = history_path().with_extension("json.tmp");
            if fs::write(&tmp, &bytes).is_ok() {
                let _ = fs::rename(&tmp, history_path());
                debug!("clipboard: persisted {} entries", persisted.len());
            }
        }
        Err(e) => warn!("clipboard: serialize failed: {e}"),
    }
}

/// Load the persisted history, reconstructing previews. Entries
/// whose blob is missing are skipped. Returned newest-first.
pub fn load() -> Vec<ClipboardEntry> {
    let path = history_path();
    let Ok(bytes) = fs::read(&path) else {
        return Vec::new();
    };
    let persisted: Vec<PersistedEntry> = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            warn!("clipboard: history.json parse failed: {e}");
            return Vec::new();
        }
    };

    let mut out = Vec::with_capacity(persisted.len());
    for p in persisted {
        let data = if let Some(text) = p.inline_text {
            text.into_bytes()
        } else {
            match fs::read(blob_path(p.content_hash)) {
                Ok(d) => d,
                Err(_) => continue, // blob gone — drop the entry
            }
        };
        let timestamp = OffsetDateTime::from_unix_timestamp(p.timestamp)
            .unwrap_or_else(|_| OffsetDateTime::now_utc());
        let preview = EntryPreview::build(&p.mime_type, &data);
        out.push(ClipboardEntry {
            id: p.id,
            timestamp,
            mime_type: p.mime_type,
            content_hash: p.content_hash,
            preview,
            data,
            pinned: p.pinned,
        });
    }
    out
}

/// Remove every persisted file (used by `PersistMode::None`).
fn wipe() {
    let _ = fs::remove_file(history_path());
    let _ = fs::remove_dir_all(blobs_dir());
}

/// Delete blob files not referenced by any kept entry.
fn gc_blobs(kept: &[PersistedEntry]) {
    let Ok(rd) = fs::read_dir(blobs_dir()) else {
        return;
    };
    let referenced: std::collections::HashSet<String> = kept
        .iter()
        .filter(|p| p.inline_text.is_none())
        .map(|p| format!("{:016x}.bin", p.content_hash))
        .collect();
    for entry in rd.flatten() {
        if let Some(name) = entry.file_name().to_str()
            && !referenced.contains(name)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}
