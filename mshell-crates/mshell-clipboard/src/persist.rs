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
//!
//! Restored image / binary entries keep only their preview in memory
//! (`ClipboardEntry::data_in_blob`); the payload is read back from its
//! blob by [`read_blob`] when the entry is pasted.

use std::fs;
use std::path::{Path, PathBuf};

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

fn history_path(dir: &Path) -> PathBuf {
    dir.join("history.json")
}

fn blobs_dir(dir: &Path) -> PathBuf {
    dir.join("blobs")
}

fn blob_path(dir: &Path, content_hash: u64) -> PathBuf {
    blobs_dir(dir).join(format!("{content_hash:016x}.bin"))
}

/// The payload of a blob-backed entry (see `ClipboardEntry::data_in_blob`).
/// `None` when the blob has been removed.
pub fn read_blob(content_hash: u64) -> Option<Vec<u8>> {
    fs::read(blob_path(&data_dir(), content_hash)).ok()
}

/// Persist the given entries according to `mode`. `entries` is the
/// full current history (newest first); this picks what to keep.
pub fn save(entries: &[ClipboardEntry], mode: PersistMode) {
    save_to(&data_dir(), entries, mode);
}

fn save_to(dir: &Path, entries: &[ClipboardEntry], mode: PersistMode) {
    if mode == PersistMode::None {
        wipe(dir);
        return;
    }

    let keep: Vec<&ClipboardEntry> = match mode {
        PersistMode::FavoritesOnly => entries.iter().filter(|e| e.pinned).collect(),
        PersistMode::All => entries.iter().collect(),
        PersistMode::None => unreachable!(),
    };

    if let Err(e) = fs::create_dir_all(dir) {
        warn!("clipboard: create data dir failed: {e}");
        return;
    }

    // Write blobs for non-text payloads (content-addressed — skip
    // if already present).
    // A blob-backed entry was loaded *from* its blob, which is
    // already on disk (and its in-memory `data` is empty).
    let blobs = blobs_dir(dir);
    let _ = fs::create_dir_all(&blobs);
    for e in &keep {
        if !e.is_text() && !e.data_in_blob {
            let p = blob_path(dir, e.content_hash);
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
    gc_blobs(dir, &persisted);

    match serde_json::to_vec_pretty(&persisted) {
        Ok(bytes) => {
            // Write to a temp file then rename for atomicity.
            let tmp = history_path(dir).with_extension("json.tmp");
            if fs::write(&tmp, &bytes).is_ok() {
                let _ = fs::rename(&tmp, history_path(dir));
                debug!("clipboard: persisted {} entries", persisted.len());
            }
        }
        Err(e) => warn!("clipboard: serialize failed: {e}"),
    }
}

/// Load the persisted history, reconstructing previews. Entries
/// whose blob is missing are skipped. Returned newest-first. Blob
/// payloads are read only to build the preview, then dropped
/// (`data_in_blob`), so they don't sit in memory for the session.
pub fn load() -> Vec<ClipboardEntry> {
    load_from(&data_dir())
}

fn load_from(dir: &Path) -> Vec<ClipboardEntry> {
    let path = history_path(dir);
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
        let (data, data_in_blob) = if let Some(text) = p.inline_text {
            (text.into_bytes(), false)
        } else {
            match fs::read(blob_path(dir, p.content_hash)) {
                Ok(d) => (d, true),
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
            // The blob stays the source of truth; keep only the preview.
            data: if data_in_blob { Vec::new() } else { data },
            data_in_blob,
            pinned: p.pinned,
        });
    }
    out
}

/// Remove every persisted file (used by `PersistMode::None`).
fn wipe(dir: &Path) {
    let _ = fs::remove_file(history_path(dir));
    let _ = fs::remove_dir_all(blobs_dir(dir));
}

/// Delete blob files not referenced by any kept entry.
fn gc_blobs(dir: &Path, kept: &[PersistedEntry]) {
    let Ok(rd) = fs::read_dir(blobs_dir(dir)) else {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, mime: &str, data: Vec<u8>) -> ClipboardEntry {
        ClipboardEntry {
            id,
            timestamp: OffsetDateTime::now_utc(),
            mime_type: mime.to_string(),
            content_hash: ClipboardEntry::content_hash(&data),
            preview: EntryPreview::build(mime, &data),
            data,
            data_in_blob: false,
            pinned: false,
        }
    }

    fn png_bytes() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(8, 6, image::Rgba([10, 20, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png)
            .expect("encode png");
        out.into_inner()
    }

    #[test]
    fn restored_blob_entries_keep_only_the_preview() {
        let dir = std::env::temp_dir().join(format!("mshell-clip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let png = png_bytes();
        let image = entry(2, "image/png", png.clone());
        let text = entry(1, "text/plain", b"hello".to_vec());
        save_to(&dir, &[image.clone(), text], PersistMode::All);

        let restored = load_from(&dir);
        assert_eq!(restored.len(), 2);
        let img = &restored[0];
        assert!(img.data_in_blob, "image payload stays on disk");
        assert!(img.data.is_empty(), "no payload copy in memory");
        assert!(
            matches!(img.preview, EntryPreview::Image { .. }),
            "thumbnail still built from the blob"
        );
        assert_eq!(
            fs::read(blob_path(&dir, img.content_hash)).ok(),
            Some(png),
            "the blob still holds the payload for paste"
        );
        let txt = &restored[1];
        assert!(!txt.data_in_blob);
        assert_eq!(txt.data, b"hello");

        // Re-saving the restored history neither drops nor blanks the blob.
        save_to(&dir, &restored, PersistMode::All);
        let again = load_from(&dir);
        assert_eq!(again.len(), 2);
        assert_eq!(
            fs::read(blob_path(&dir, image.content_hash))
                .map(|b| b.len())
                .ok(),
            Some(image.data.len())
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
