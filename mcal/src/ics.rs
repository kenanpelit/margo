//! RFC 5545 ICS → [`Event`] mapping, on top of the `icalendar` crate.
//!
//! We keep everything the agenda needs (summary, time range, location, url,
//! status, categories) and preserve the raw recurrence lines for `recur`. All
//! datetimes are normalised to UTC here; `TZID` values are resolved through
//! `chrono-tz`, floating times are read as UTC (slice-1 simplification).

use crate::error::McalError;
use crate::model::Event;
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use icalendar::{
    Calendar as IcalCalendar, CalendarDateTime, Component, DatePerhapsTime, Event as IcalEvent,
};

/// Parse an `.ics` payload into events, tagging each with `calendar_id`.
/// `VTODO`s and non-event components are skipped (tasks are out of slice 1).
pub fn parse_ics(text: &str, calendar_id: &str) -> Result<Vec<Event>, McalError> {
    let cal: IcalCalendar = text.parse().map_err(McalError::Ics)?;

    let mut out = Vec::new();
    for component in &cal.components {
        if let Some(ev) = component.as_event()
            && let Some(mapped) = map_event(ev, calendar_id)
        {
            out.push(mapped);
        }
    }
    Ok(out)
}

/// Map one `icalendar` VEVENT to our [`Event`]. Returns `None` only when the
/// event has no usable start (which we can't place on a calendar).
fn map_event(ev: &IcalEvent, calendar_id: &str) -> Option<Event> {
    let (start, all_day) = dpt_to_utc(&ev.get_start()?);
    let end = match ev.get_end() {
        Some(dpt) => dpt_to_utc(&dpt).0,
        // RFC 5545: a DATE DTSTART with no DTEND is a one-day all-day event; a
        // DATE-TIME with no DTEND has zero duration.
        None if all_day => start + chrono::Duration::days(1),
        None => start,
    };

    let summary = ev.get_summary().unwrap_or("(no title)").to_string();
    let uid = ev
        .get_uid()
        .map(str::to_string)
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| format!("{}-{}", start.timestamp(), summary));

    Some(Event {
        id: format!("{calendar_id}::{uid}"),
        calendar_id: calendar_id.to_string(),
        uid,
        summary,
        description: ev.get_description().map(str::to_string),
        location: ev.property_value("LOCATION").map(str::to_string),
        url: ev.property_value("URL").map(str::to_string),
        status: ev.property_value("STATUS").map(str::to_string),
        start,
        end,
        all_day,
        recurrence: collect_recurrence(ev),
        // Attendees aren't surfaced in the slice-1 agenda; parse deferred.
        attendees: Vec::new(),
        categories: collect_categories(ev),
    })
}

/// `CATEGORIES` is a comma-separated, multi-valued property; `icalendar` may
/// hold it in either `properties` or `multi_properties`, so read both and
/// split every value on commas.
fn collect_categories(ev: &IcalEvent) -> Vec<String> {
    let mut raw: Vec<&str> = Vec::new();
    if let Some(v) = ev.property_value("CATEGORIES") {
        raw.push(v);
    }
    if let Some(multi) = ev.multi_properties().get("CATEGORIES") {
        raw.extend(multi.iter().map(|p| p.value()));
    }
    raw.iter()
        .flat_map(|v| v.split(','))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `DTSTART`/`DTEND` → (UTC instant, is-all-day).
fn dpt_to_utc(dpt: &DatePerhapsTime) -> (DateTime<Utc>, bool) {
    match dpt {
        DatePerhapsTime::Date(d) => (date_start_utc(*d), true),
        DatePerhapsTime::DateTime(cdt) => (cdt_to_utc(cdt), false),
    }
}

/// Midnight-UTC instant for an all-day date (infallible — no `unwrap`).
fn date_start_utc(d: NaiveDate) -> DateTime<Utc> {
    d.and_time(NaiveTime::MIN).and_utc()
}

/// Resolve a `CalendarDateTime` to UTC. A `TZID` we can't resolve, and floating
/// local time, both fall back to reading the wall-clock value as UTC.
fn cdt_to_utc(cdt: &CalendarDateTime) -> DateTime<Utc> {
    match cdt {
        CalendarDateTime::Utc(dt) => *dt,
        CalendarDateTime::Floating(naive) => naive.and_utc(),
        CalendarDateTime::WithTimezone { date_time, tzid } => tzid
            .parse::<chrono_tz::Tz>()
            .ok()
            .and_then(|tz| tz.from_local_datetime(date_time).single())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|| date_time.and_utc()),
    }
}

/// Reconstruct the raw `RRULE` / `RDATE` / `EXDATE` lines for `recur::expand`.
fn collect_recurrence(ev: &IcalEvent) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(rrule) = ev.property_value("RRULE") {
        lines.push(format!("RRULE:{rrule}"));
    }
    for key in ["EXDATE", "RDATE"] {
        if let Some(v) = ev.property_value(key) {
            lines.push(format!("{key}:{v}"));
        }
        if let Some(multi) = ev.multi_properties().get(key) {
            for prop in multi {
                lines.push(format!("{key}:{}", prop.value()));
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//mcal//test//EN\r
BEGIN:VEVENT\r
UID:allday@test\r
SUMMARY:Holiday\r
DTSTART;VALUE=DATE:20260704\r
DTEND;VALUE=DATE:20260705\r
CATEGORIES:Personal,Trips\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:utc@test\r
SUMMARY:Standup\r
LOCATION:Room 1\r
DTSTART:20260703T090000Z\r
DTEND:20260703T093000Z\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:tz@test\r
SUMMARY:Lunch\r
DTSTART;TZID=Europe/Istanbul:20260703T130000\r
DTEND;TZID=Europe/Istanbul:20260703T140000\r
END:VEVENT\r
END:VCALENDAR\r
";

    #[test]
    fn parses_three_events_with_correct_kinds() {
        let events = parse_ics(SAMPLE, "cal1").expect("parse");
        assert_eq!(events.len(), 3);

        let holiday = events.iter().find(|e| e.uid == "allday@test").unwrap();
        assert!(holiday.all_day);
        assert_eq!(holiday.summary, "Holiday");
        assert_eq!(holiday.categories, vec!["Personal", "Trips"]);
        assert_eq!(holiday.id, "cal1::allday@test");

        let standup = events.iter().find(|e| e.uid == "utc@test").unwrap();
        assert!(!standup.all_day);
        assert_eq!(standup.location.as_deref(), Some("Room 1"));
        assert_eq!(standup.start.to_rfc3339(), "2026-07-03T09:00:00+00:00");
        assert_eq!(standup.end.to_rfc3339(), "2026-07-03T09:30:00+00:00");
    }

    #[test]
    fn resolves_tzid_to_utc() {
        let events = parse_ics(SAMPLE, "cal1").expect("parse");
        let lunch = events.iter().find(|e| e.uid == "tz@test").unwrap();
        // Europe/Istanbul is UTC+3 → 13:00 local == 10:00 UTC.
        assert_eq!(lunch.start.to_rfc3339(), "2026-07-03T10:00:00+00:00");
        assert!(!lunch.all_day);
    }

    #[test]
    fn empty_calendar_is_ok() {
        let events = parse_ics(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n",
            "cal1",
        )
        .expect("parse");
        assert!(events.is_empty());
    }
}
