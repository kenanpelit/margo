use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::entry::ClipboardEntry;

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
        // Pinned first (recency order within each group), then the
        // rolling history. The UI reads this order top-to-bottom.
        let mut pinned: Vec<ClipboardEntry> = Vec::new();
        let mut rest: Vec<ClipboardEntry> = Vec::new();
        for e in inner.entries.iter() {
            if e.pinned {
                pinned.push(e.clone());
            } else {
                rest.push(e.clone());
            }
        }
        pinned.extend(rest);
        pinned
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
