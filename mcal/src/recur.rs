//! Recurrence expansion — a recurring [`Event`] master → concrete instances
//! within a date window, via the `rrule` crate.
//!
//! Non-recurring events pass through (kept iff they overlap the window).
//! Recurring events are expanded from their reconstructed `DTSTART` + raw
//! `RRULE`/`RDATE`/`EXDATE` lines, capped at [`MAX_INSTANCES`] so an unbounded
//! rule can't run away. Every instance carries the master's `uid`.

use crate::model::Event;
use chrono::{DateTime, Utc};

/// Hard cap on expanded instances per master per window (~a year of dailies).
const MAX_INSTANCES: u16 = 732;

/// Expand `event` into the concrete occurrences that fall in `[start, end]`.
pub fn expand(event: &Event, window_start: DateTime<Utc>, window_end: DateTime<Utc>) -> Vec<Event> {
    if !event.is_recurring() {
        return if event.end >= window_start && event.start <= window_end {
            vec![event.clone()]
        } else {
            Vec::new()
        };
    }

    let duration = event.end - event.start;
    let mut blocks = vec![format!("DTSTART:{}", event.start.format("%Y%m%dT%H%M%SZ"))];
    blocks.extend(event.recurrence.iter().cloned());

    let set: rrule::RRuleSet = match blocks.join("\n").parse() {
        Ok(set) => set,
        Err(err) => {
            tracing::warn!(uid = %event.uid, %err, "mcal: unparseable recurrence, skipping");
            return Vec::new();
        }
    };

    let after = window_start.with_timezone(&rrule::Tz::UTC);
    let before = window_end.with_timezone(&rrule::Tz::UTC);
    let result = set.after(after).before(before).all(MAX_INSTANCES);

    result
        .dates
        .into_iter()
        .map(|occurrence| {
            let start = occurrence.with_timezone(&Utc);
            Event {
                id: format!("{}::{}", event.id, start.timestamp()),
                start,
                end: start + duration,
                recurrence: Vec::new(),
                ..event.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn base() -> Event {
        Event {
            id: "cal::series".into(),
            calendar_id: "cal".into(),
            uid: "series".into(),
            summary: "Weekly sync".into(),
            description: None,
            location: None,
            url: None,
            status: None,
            start: Utc.with_ymd_and_hms(2026, 7, 6, 9, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 7, 6, 9, 30, 0).unwrap(),
            all_day: false,
            recurrence: vec![],
            attendees: vec![],
            categories: vec![],
        }
    }

    #[test]
    fn non_recurring_passes_through_when_in_window() {
        let ev = base();
        let out = expand(
            &ev,
            Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 31, 0, 0, 0).unwrap(),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start, ev.start);
    }

    #[test]
    fn non_recurring_dropped_when_outside_window() {
        let ev = base();
        let out = expand(
            &ev,
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 8, 31, 0, 0, 0).unwrap(),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn weekly_rule_expands_within_window() {
        let mut ev = base();
        ev.recurrence = vec!["RRULE:FREQ=WEEKLY;COUNT=8".into()];
        // A 3-week window starting on the master start: expect 3 Mondays
        // (Jul 6, 13, 20).
        let out = expand(
            &ev,
            Utc.with_ymd_and_hms(2026, 7, 6, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 26, 0, 0, 0).unwrap(),
        );
        assert_eq!(
            out.len(),
            3,
            "got {:?}",
            out.iter().map(|e| e.start).collect::<Vec<_>>()
        );
        // Instances keep the master's uid + 30-min duration, get unique ids.
        assert!(out.iter().all(|e| e.uid == "series"));
        assert!(
            out.iter()
                .all(|e| e.end - e.start == chrono::Duration::minutes(30))
        );
        assert_eq!(
            out[0].start,
            Utc.with_ymd_and_hms(2026, 7, 6, 9, 0, 0).unwrap()
        );
        assert_eq!(
            out[2].start,
            Utc.with_ymd_and_hms(2026, 7, 20, 9, 0, 0).unwrap()
        );
    }

    #[test]
    fn exdate_is_honoured() {
        let mut ev = base();
        ev.recurrence = vec![
            "RRULE:FREQ=WEEKLY;COUNT=8".into(),
            "EXDATE:20260713T090000Z".into(),
        ];
        let out = expand(
            &ev,
            Utc.with_ymd_and_hms(2026, 7, 6, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 26, 0, 0, 0).unwrap(),
        );
        // Jul 13 excluded → 2 instances left (Jul 6, 20).
        assert_eq!(
            out.len(),
            2,
            "got {:?}",
            out.iter().map(|e| e.start).collect::<Vec<_>>()
        );
        assert!(
            !out.iter()
                .any(|e| e.start == Utc.with_ymd_and_hms(2026, 7, 13, 9, 0, 0).unwrap())
        );
    }
}
