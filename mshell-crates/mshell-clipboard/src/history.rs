use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::entry::{ClipboardEntry, EntryPreview, EntryView};

#[derive(Clone)]
pub struct ClipboardHistory {
    inner: Arc<Mutex<HistoryInner>>,
}

struct HistoryInner {
    entries: VecDeque<ClipboardEntry>,
    max_entries: usize,
    next_id: u64,
}

impl ClipboardHistory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HistoryInner {
                entries: VecDeque::with_capacity(max_entries.max(1)),
                max_entries,
                next_id: 1,
            })),
        }
    }

    /// Push a new entry, deduplicating against existing entries.
    ///
    /// If an entry with the same content hash already exists, it is moved
    /// to the front (most recent), its timestamp updated, and its pinned
    /// flag preserved — rather than inserting a duplicate.
    ///
    /// Returns the id of the (new or promoted) entry.
    pub fn push(&self, mut entry: ClipboardEntry) -> u64 {
        let mut inner = self.inner.lock().unwrap();

        if let Some(pos) = inner
            .entries
            .iter()
            .position(|e| e.content_hash == entry.content_hash)
        {
            let existing = inner.entries.remove(pos).unwrap();
            entry.id = existing.id;
            // Preserve a previously-set pin across re-copies.
            entry.pinned = existing.pinned || entry.pinned;
            inner.entries.push_front(entry);
            return inner.entries.front().unwrap().id;
        }

        entry.id = inner.next_id;
        inner.next_id += 1;

        let id = entry.id;
        inner.entries.push_front(entry);
        inner.evict_overflow();
        id
    }

    /// Seed history from a persisted snapshot (oldest-last). Sets
    /// `next_id` past the highest restored id so new copies don't
    /// collide. Used once at startup.
    pub fn load_snapshot(&self, snapshot: Vec<ClipboardEntry>) {
        let mut inner = self.inner.lock().unwrap();
        inner.entries.clear();
        let mut max_id = 0;
        for e in snapshot {
            max_id = max_id.max(e.id);
            inner.entries.push_back(e);
        }
        inner.next_id = max_id + 1;
        inner.evict_overflow();
    }

    pub fn entries(&self) -> Vec<ClipboardEntry> {
        let inner = self.inner.lock().unwrap();
        // Plain recency order (newest first). Pinned entries keep
        // their natural position — they're marked with a star, not
        // hoisted to the top; the Favorites tab is where they're
        // shown in isolation.
        inner.entries.iter().cloned().collect()
    }

    /// Lightweight projection of the whole history for the UI list
    /// model: each entry mapped to an [`EntryView`] (preview +
    /// metadata + search haystack) under a single lock, **without**
    /// cloning the entries' raw `data` payloads. This is what the
    /// clipboard menu populates from; [`Self::entries`] (full clones,
    /// incl. `data`) is reserved for persistence, which needs the bytes.
    pub fn views(&self) -> Vec<EntryView> {
        // Degrade to an empty list rather than panic if the lock is
        // poisoned — a stale/blank clipboard menu beats taking the
        // shell down (and keeps this off the panic-ratchet).
        let Ok(inner) = self.inner.lock() else {
            return Vec::new();
        };
        inner.entries.iter().map(ClipboardEntry::to_view).collect()
    }

    /// The display preview of entry `id`, decoding a pending image
    /// thumbnail ([`EntryPreview::ImagePending`]) from its blob on first
    /// call and caching it on the entry. Blocking (file read + image
    /// decode, done outside the history lock): call it off the GTK main
    /// thread. `None` when the entry is gone.
    pub fn resolve_preview(&self, id: u64) -> Option<EntryPreview> {
        self.resolve_preview_with(id, crate::persist::read_blob)
    }

    fn resolve_preview_with(
        &self,
        id: u64,
        read_blob: impl FnOnce(u64) -> Option<Vec<u8>>,
    ) -> Option<EntryPreview> {
        let (content_hash, mime_type) = {
            let inner = self.inner.lock().ok()?;
            let entry = inner.entries.iter().find(|e| e.id == id)?;
            match &entry.preview {
                EntryPreview::ImagePending { mime_type } => (entry.content_hash, mime_type.clone()),
                ready => return Some(ready.clone()),
            }
        };
        let preview = match read_blob(content_hash) {
            Some(data) => EntryPreview::build(&mime_type, &data),
            // The blob vanished under us: show it as an (empty) binary
            // rather than retrying the read on every bind.
            None => EntryPreview::Binary { mime_type, size: 0 },
        };
        let mut inner = self.inner.lock().ok()?;
        if let Some(entry) = inner
            .entries
            .iter_mut()
            .find(|e| e.id == id && e.content_hash == content_hash)
            && matches!(entry.preview, EntryPreview::ImagePending { .. })
        {
            entry.preview = preview.clone();
        }
        Some(preview)
    }

    pub fn get(&self, id: u64) -> Option<ClipboardEntry> {
        let inner = self.inner.lock().unwrap();
        inner.entries.iter().find(|e| e.id == id).cloned()
    }

    pub fn remove(&self, id: u64) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if let Some(pos) = inner.entries.iter().position(|e| e.id == id) {
            inner.entries.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.entries.clear();
    }

    /// Clear everything except pinned (favourite) entries.
    pub fn clear_unpinned(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.entries.retain(|e| e.pinned);
    }

    /// Drop non-pinned entries whose timestamp is older than
    /// `max_age_secs` from now. Returns true if anything changed.
    pub fn prune_older_than(&self, max_age_secs: i64) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let cutoff = time::OffsetDateTime::now_utc().unix_timestamp() - max_age_secs;
        let before = inner.entries.len();
        inner
            .entries
            .retain(|e| e.pinned || e.timestamp.unix_timestamp() >= cutoff);
        inner.entries.len() != before
    }

    pub fn set_max_entries(&self, max: usize) {
        let mut inner = self.inner.lock().unwrap();
        inner.max_entries = max.max(1);
        inner.evict_overflow();
    }

    /// Toggle the pinned flag on an entry. Returns the new state,
    /// or `None` if the id wasn't found.
    pub fn toggle_pin(&self, id: u64) -> Option<bool> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.entries.iter_mut().find(|e| e.id == id) {
            e.pinned = !e.pinned;
            Some(e.pinned)
        } else {
            None
        }
    }

    /// Move an entry to the front of the history without changing its id.
    pub fn promote(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(pos) = inner.entries.iter().position(|e| e.id == id) {
            let mut entry = inner.entries.remove(pos).unwrap();
            entry.timestamp = time::OffsetDateTime::now_utc();
            inner.entries.push_front(entry);
        }
    }
}

impl HistoryInner {
    /// Evict the oldest NON-pinned entries until within capacity.
    /// Pinned entries never count against the cap and are never
    /// evicted — if the history is all-pinned it's allowed to grow.
    fn evict_overflow(&mut self) {
        while self.entries.iter().filter(|e| !e.pinned).count() > self.max_entries {
            // Find the oldest (back-most) non-pinned entry.
            if let Some(pos) = self.entries.iter().rposition(|e| !e.pinned) {
                self.entries.remove(pos);
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn png_bytes() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(8, 6, image::Rgba([10, 20, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png)
            .expect("encode png");
        out.into_inner()
    }

    fn pending_image(id: u64) -> ClipboardEntry {
        ClipboardEntry {
            id,
            timestamp: time::OffsetDateTime::now_utc(),
            mime_type: "image/png".into(),
            data: Vec::new(),
            data_in_blob: true,
            preview: EntryPreview::ImagePending {
                mime_type: "image/png".into(),
            },
            content_hash: 42,
            pinned: false,
        }
    }

    #[test]
    fn pending_thumbnail_is_decoded_once_on_demand() {
        let history = ClipboardHistory::new(10);
        history.load_snapshot(vec![pending_image(1)]);
        let png = png_bytes();
        let reads = Cell::new(0);
        let read = |hash: u64| {
            assert_eq!(hash, 42);
            reads.set(reads.get() + 1);
            Some(png.clone())
        };

        let first = history.resolve_preview_with(1, read).unwrap();
        assert!(matches!(first, EntryPreview::Image { .. }));
        // Cached on the entry: the views (menu model) see the decoded
        // thumbnail and a second resolve reads nothing.
        assert!(matches!(
            history.views()[0].preview,
            EntryPreview::Image { .. }
        ));
        let again = history
            .resolve_preview_with(1, |_| panic!("decoded twice"))
            .unwrap();
        assert!(matches!(again, EntryPreview::Image { .. }));
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn missing_blob_degrades_to_binary_and_unknown_id_is_none() {
        let history = ClipboardHistory::new(10);
        history.load_snapshot(vec![pending_image(1)]);
        assert!(matches!(
            history.resolve_preview_with(1, |_| None),
            Some(EntryPreview::Binary { size: 0, .. })
        ));
        assert!(history.resolve_preview_with(7, |_| None).is_none());
    }
}
