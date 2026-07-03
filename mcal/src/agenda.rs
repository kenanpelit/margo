//! Pure view helpers over a loaded set of events: which events fall on a day,
//! which days in a month carry events (for grid marks), and agenda ordering.
//!
//! Timezone rule: **all-day** events are date-anchored — compared by their UTC
//! calendar date so a holiday lands on the same day everywhere. **Timed**
//! events are compared in the machine's local time, which is where the user
//! reads them.

use crate::model::Event;
use chrono::{Datelike, Local, NaiveDate};
use std::collections::BTreeSet;

/// The inclusive local (or UTC, for all-day) `[first, last]` calendar days an
/// event covers.
fn covered_days(event: &Event) -> (NaiveDate, NaiveDate) {
    if event.all_day {
        let start = event.start.date_naive();
        // All-day DTEND is exclusive: a Jul4→Jul5 event covers only Jul4.
        let last = event
            .end
            .date_naive()
            .pred_opt()
            .filter(|d| *d >= start)
            .unwrap_or(start);
        (start, last)
    } else {
        (
            event.start.with_timezone(&Local).date_naive(),
            event.end.with_timezone(&Local).date_naive(),
        )
    }
}

fn occurs_on(event: &Event, date: NaiveDate) -> bool {
    let (first, last) = covered_days(event);
    date >= first && date <= last
}

/// Events that occur on `date`, sorted for display (all-day first, then by
/// start time).
pub fn events_on_day(events: &[Event], date: NaiveDate) -> Vec<Event> {
    let mut out: Vec<Event> = events.iter().filter(|e| occurs_on(e, date)).cloned().collect();
    sort_agenda(&mut out);
    out
}

/// Order an agenda: all-day events pinned to the top, then chronological.
pub fn sort_agenda(events: &mut [Event]) {
    events.sort_by(|a, b| b.all_day.cmp(&a.all_day).then(a.start.cmp(&b.start)));
}

/// The day-of-month numbers (1–31) in `year`/`month` that have ≥1 event —
/// used to place marks on the month grid.
pub fn days_with_events(events: &[Event], year: i32, month: u32) -> BTreeSet<u32> {
    let mut days = BTreeSet::new();
    let (Some(first), Some(last)) = (
        NaiveDate::from_ymd_opt(year, month, 1),
        month_last_day(year, month),
    ) else {
        return days;
    };

    for event in events {
        let (start, end) = covered_days(event);
        // Clamp the event's span to this month, so the loop is ≤31 iterations.
        let mut day = start.max(first);
        let stop = end.min(last);
        while day <= stop {
            days.insert(day.day());
            match day.succ_opt() {
                Some(next) => day = next,
                None => break,
            }
        }
    }
    days
}

fn month_last_day(year: i32, month: u32) -> Option<NaiveDate> {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)?.pred_opt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Event;
    use chrono::{DateTime, TimeZone, Utc};

    fn ev(uid: &str, start: DateTime<Utc>, end: DateTime<Utc>, all_day: bool) -> Event {
        Event {
            id: uid.into(),
            calendar_id: "c".into(),
            uid: uid.into(),
            summary: uid.into(),
            description: None,
            location: None,
            url: None,
            status: None,
            start,
            end,
            all_day,
            recurrence: vec![],
            attendees: vec![],
            categories: vec![],
        }
    }

    #[test]
    fn all_day_lands_on_its_utc_date_regardless_of_local_tz() {
        // Jul4 all-day (exclusive end Jul5) → covers only Jul4, tz-independent.
        let holiday = ev(
            "h",
            Utc.with_ymd_and_hms(2026, 7, 4, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 5, 0, 0, 0).unwrap(),
            true,
        );
        let jul4 = NaiveDate::from_ymd_opt(2026, 7, 4).unwrap();
        let jul5 = NaiveDate::from_ymd_opt(2026, 7, 5).unwrap();
        assert_eq!(events_on_day(std::slice::from_ref(&holiday), jul4).len(), 1);
        assert!(events_on_day(&[holiday], jul5).is_empty());
    }

    #[test]
    fn timed_event_matches_its_local_day() {
        let timed = ev(
            "t",
            Utc.with_ymd_and_hms(2026, 7, 3, 9, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 3, 9, 30, 0).unwrap(),
            false,
        );
        // Self-consistent across machine tz: derive the expected day the same
        // way the helper does.
        let local_day = timed.start.with_timezone(&Local).date_naive();
        assert_eq!(events_on_day(&[timed], local_day).len(), 1);
    }

    #[test]
    fn agenda_puts_all_day_first_then_by_time() {
        let morning = ev(
            "m",
            Utc.with_ymd_and_hms(2026, 7, 4, 8, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 4, 9, 0, 0).unwrap(),
            false,
        );
        let allday = ev(
            "a",
            Utc.with_ymd_and_hms(2026, 7, 4, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 5, 0, 0, 0).unwrap(),
            true,
        );
        let noon = ev(
            "n",
            Utc.with_ymd_and_hms(2026, 7, 4, 12, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 4, 13, 0, 0).unwrap(),
            false,
        );
        let mut list = vec![noon, allday, morning];
        sort_agenda(&mut list);
        assert_eq!(
            list.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(),
            vec!["a", "m", "n"]
        );
    }

    #[test]
    fn days_with_events_marks_the_right_days() {
        let jul4 = ev(
            "a",
            Utc.with_ymd_and_hms(2026, 7, 4, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 5, 0, 0, 0).unwrap(),
            true,
        );
        let days = days_with_events(&[jul4], 2026, 7);
        assert!(days.contains(&4));
        assert!(!days.contains(&5));
        // Nothing in a different month.
        assert!(days_with_events(&[], 2026, 8).is_empty());
    }
}
