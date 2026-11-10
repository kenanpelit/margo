//! The timer book: the pending one-shot timers and reminders, as the
//! JSON file in the state dir holds them. Pure data + logic — the clock is
//! always passed in, so add / cancel / due / missed are tested without
//! waiting or touching the real file.

use super::parse::TimerKind;
use serde::{Deserialize, Serialize};

/// A timer that rings more than this long after its due time was missed
/// (the machine slept or mshell wasn't running): it fires once, marked
/// "missed", without the looping tone.
pub const MISSED_GRACE_SECS: i64 = 60;

/// Most pending timers kept; `add` refuses beyond it.
pub const MAX_TIMERS: usize = 100;

/// File format version.
const VERSION: u32 = 1;

/// One pending timer or reminder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timer {
    /// Small, stable while pending; what `mshellctl timer cancel` takes.
    pub id: u32,
    /// When it rings (Unix seconds).
    pub due: i64,
    /// When it was set (Unix seconds); `due - created` is its length.
    pub created: i64,
    pub kind: TimerKind,
    #[serde(default)]
    pub label: String,
}

impl Timer {
    /// Seconds left at `now` (0 once due).
    pub fn remaining(&self, now: i64) -> i64 {
        (self.due - now).max(0)
    }

    /// Fraction elapsed at `now`, 0.0 ..= 1.0.
    pub fn progress(&self, now: i64) -> f64 {
        let total = self.due - self.created;
        if total <= 0 {
            return 1.0;
        }
        ((now - self.created) as f64 / total as f64).clamp(0.0, 1.0)
    }
}

/// A timer the clock reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fired {
    pub timer: Timer,
    /// Reached more than [`MISSED_GRACE_SECS`] late.
    pub missed: bool,
}

/// Why [`TimerBook::add`] refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    Full,
}

impl std::fmt::Display for AddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddError::Full => write!(f, "Too many timers ({MAX_TIMERS}); cancel some first."),
        }
    }
}

/// The pending timers, soonest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerBook {
    #[serde(default = "version")]
    version: u32,
    #[serde(default = "first_id")]
    next_id: u32,
    #[serde(default)]
    timers: Vec<Timer>,
}

fn version() -> u32 {
    VERSION
}

fn first_id() -> u32 {
    1
}

impl Default for TimerBook {
    fn default() -> Self {
        Self {
            version: VERSION,
            next_id: 1,
            timers: Vec::new(),
        }
    }
}

impl TimerBook {
    /// Parse the saved file, repairing what a hand edit could break
    /// (order, duplicate ids, a stale `next_id`).
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let mut book: TimerBook = serde_json::from_str(text)?;
        book.normalize();
        Ok(book)
    }

    pub fn to_json(&self) -> String {
        // A struct of plain fields always serializes.
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.timers.retain(|t| t.id != 0 && seen.insert(t.id));
        self.timers.truncate(MAX_TIMERS);
        self.timers.sort_by_key(|t| (t.due, t.id));
        let max_id = self.timers.iter().map(|t| t.id).max().unwrap_or(0);
        if self.timers.is_empty() {
            self.next_id = 1;
        } else if self.next_id <= max_id {
            self.next_id = max_id.saturating_add(1);
        }
        self.version = VERSION;
    }

    /// The pending timers, soonest first.
    pub fn timers(&self) -> &[Timer] {
        &self.timers
    }

    pub fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }

    /// The soonest pending timer.
    pub fn next(&self) -> Option<&Timer> {
        self.timers.first()
    }

    /// Add a timer ringing at `due`; returns it with its id.
    pub fn add(
        &mut self,
        due: i64,
        created: i64,
        kind: TimerKind,
        label: &str,
    ) -> Result<Timer, AddError> {
        if self.timers.len() >= MAX_TIMERS {
            return Err(AddError::Full);
        }
        let id = self.next_id.max(1);
        self.next_id = id.checked_add(1).unwrap_or(1);
        let timer = Timer {
            id,
            due,
            created,
            kind,
            label: label.to_string(),
        };
        self.timers.push(timer.clone());
        self.timers.sort_by_key(|t| (t.due, t.id));
        Ok(timer)
    }

    /// Remove the timer with `id`.
    pub fn cancel(&mut self, id: u32) -> Option<Timer> {
        let i = self.timers.iter().position(|t| t.id == id)?;
        let t = self.timers.remove(i);
        self.reset_ids_when_empty();
        Some(t)
    }

    /// Remove every timer; returns how many there were.
    pub fn clear(&mut self) -> usize {
        let n = self.timers.len();
        self.timers.clear();
        self.reset_ids_when_empty();
        n
    }

    /// Take every timer due at `now`, marking the ones reached more than
    /// [`MISSED_GRACE_SECS`] late as missed.
    pub fn take_due(&mut self, now: i64) -> Vec<Fired> {
        let split = self.timers.partition_point(|t| t.due <= now);
        if split == 0 {
            return Vec::new();
        }
        let fired = self
            .timers
            .drain(..split)
            .map(|timer| Fired {
                missed: now - timer.due > MISSED_GRACE_SECS,
                timer,
            })
            .collect();
        self.reset_ids_when_empty();
        fired
    }

    /// Ids restart at 1 once nothing is pending, so they stay short.
    fn reset_ids_when_empty(&mut self) {
        if self.timers.is_empty() {
            self.next_id = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_794_312_000; // 2026-11-10 14:00 +03:00

    fn book_with(dues: &[i64]) -> TimerBook {
        let mut b = TimerBook::default();
        for d in dues {
            b.add(NOW + d, NOW, TimerKind::Timer, "x").expect("room");
        }
        b
    }

    #[test]
    fn timers_stay_sorted_soonest_first_with_increasing_ids() {
        let b = book_with(&[600, 60, 3600]);
        let dues: Vec<i64> = b.timers().iter().map(|t| t.due - NOW).collect();
        assert_eq!(dues, vec![60, 600, 3600]);
        let ids: Vec<u32> = b.timers().iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![2, 1, 3]);
        assert_eq!(b.next().map(|t| t.id), Some(2));
    }

    #[test]
    fn cancel_and_clear_remove_and_ids_restart_when_empty() {
        let mut b = book_with(&[60, 120]);
        assert_eq!(b.cancel(1).map(|t| t.id), Some(1));
        assert_eq!(b.cancel(1), None);
        let t = b.add(NOW + 5, NOW, TimerKind::Timer, "").expect("room");
        assert_eq!(t.id, 3, "ids are not reused while others are pending");
        assert_eq!(b.clear(), 2);
        assert!(b.is_empty());
        let t = b.add(NOW + 5, NOW, TimerKind::Timer, "").expect("room");
        assert_eq!(t.id, 1);
    }

    #[test]
    fn the_book_refuses_more_than_the_cap() {
        let mut b = TimerBook::default();
        for i in 0..MAX_TIMERS {
            b.add(NOW + i as i64, NOW, TimerKind::Timer, "")
                .expect("room");
        }
        assert_eq!(b.add(NOW, NOW, TimerKind::Timer, ""), Err(AddError::Full));
    }

    #[test]
    fn take_due_fires_on_time_timers_and_marks_late_ones_missed() {
        let mut b = book_with(&[-3600, -30, 0, 10]);
        let fired = b.take_due(NOW);
        let got: Vec<(i64, bool)> = fired
            .iter()
            .map(|f| (f.timer.due - NOW, f.missed))
            .collect();
        // An hour late = missed (slept through it); 30 s and 0 s = on time.
        assert_eq!(got, vec![(-3600, true), (-30, false), (0, false)]);
        assert_eq!(b.timers().len(), 1);
        assert!(b.take_due(NOW).is_empty(), "each fires exactly once");
        assert_eq!(b.take_due(NOW + 10).len(), 1);
        assert!(b.is_empty());
    }

    #[test]
    fn the_missed_boundary_is_the_grace_period() {
        let mut b = book_with(&[0, 1]);
        let fired = b.take_due(NOW + 1 + MISSED_GRACE_SECS);
        assert_eq!(
            fired.iter().map(|f| f.missed).collect::<Vec<_>>(),
            vec![true, false]
        );
    }

    #[test]
    fn json_round_trips() {
        let mut b = book_with(&[60, 120]);
        b.cancel(1);
        let text = b.to_json();
        let back = TimerBook::from_json(&text).expect("parses");
        assert_eq!(back, b);
        assert!(text.contains("\"kind\": \"timer\""), "{text}");
    }

    #[test]
    fn a_hand_edited_file_is_repaired_on_load() {
        let text = r#"{"next_id": 1, "timers": [
            {"id": 5, "due": 300, "created": 0, "kind": "reminder"},
            {"id": 5, "due": 100, "created": 0, "kind": "timer", "label": "dup"},
            {"id": 0, "due": 50, "created": 0, "kind": "timer"},
            {"id": 2, "due": 200, "created": 0, "kind": "snooze", "label": "b"}
        ]}"#;
        let mut b = TimerBook::from_json(text).expect("parses");
        let ids: Vec<u32> = b.timers().iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            vec![2, 5],
            "duplicate and zero ids dropped, sorted by due"
        );
        assert_eq!(b.timers()[1].label, "", "missing label defaults");
        let t = b.add(400, 0, TimerKind::Timer, "").expect("room");
        assert_eq!(t.id, 6, "next_id moved past the highest id");
        assert!(TimerBook::from_json("{}").expect("parses").is_empty());
        assert!(TimerBook::from_json("not json").is_err());
    }

    #[test]
    fn remaining_and_progress_are_clamped() {
        let t = Timer {
            id: 1,
            due: NOW + 100,
            created: NOW,
            kind: TimerKind::Timer,
            label: String::new(),
        };
        assert_eq!(t.remaining(NOW), 100);
        assert_eq!(t.remaining(NOW + 500), 0);
        assert_eq!(t.progress(NOW + 25), 0.25);
        assert_eq!(t.progress(NOW - 5), 0.0);
        assert_eq!(t.progress(NOW + 500), 1.0);
    }
}
