//! Google Calendar API v3 provider (read-only).
//!
//! `singleEvents=true` makes Google expand recurrence server-side, so mapped
//! events carry no RRULE and skip [`crate::recur`]. Token handling is per-fetch:
//! read the refresh token from the keyring, mint an access token, then page
//! through each of the account's calendars.

use super::{Provider, Window};
use crate::credentials::GoogleCredentials;
use crate::error::McalError;
use crate::model::{Calendar, Event};
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::Deserialize;

const API: &str = "https://www.googleapis.com/calendar/v3";

/// A Google account as an mcal calendar source.
pub struct GoogleProvider {
    account_id: String,
    credentials: GoogleCredentials,
}

#[derive(Debug, Deserialize)]
struct CalendarListResponse {
    #[serde(default)]
    items: Vec<GoogleCalendar>,
}

#[derive(Debug, Deserialize)]
struct GoogleCalendar {
    id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default, rename = "backgroundColor")]
    background_color: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventsResponse {
    #[serde(default)]
    items: Vec<GoogleEvent>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleEvent {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default, rename = "htmlLink")]
    html_link: Option<String>,
    #[serde(default)]
    start: Option<GoogleDate>,
    #[serde(default)]
    end: Option<GoogleDate>,
}

#[derive(Debug, Deserialize)]
struct GoogleDate {
    #[serde(default)]
    date: Option<String>,
    #[serde(default, rename = "dateTime")]
    date_time: Option<String>,
}

/// Resolve a Google date/time to UTC; `true` if it was an all-day `date`.
fn resolve(d: &GoogleDate) -> Option<(DateTime<Utc>, bool)> {
    if let Some(dt) = &d.date_time {
        let parsed = DateTime::parse_from_rfc3339(dt).ok()?;
        Some((parsed.with_timezone(&Utc), false))
    } else if let Some(date) = &d.date {
        let day = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
        Some((Utc.from_utc_datetime(&day.and_time(NaiveTime::MIN)), true))
    } else {
        None
    }
}

/// Map one Google event to an mcal [`Event`], or `None` to skip it.
fn map_event(raw: &GoogleEvent, calendar_id: &str) -> Option<Event> {
    if raw.status.as_deref() == Some("cancelled") {
        return None;
    }
    let (start, all_day) = resolve(raw.start.as_ref()?)?;
    let end = raw
        .end
        .as_ref()
        .and_then(resolve)
        .map(|(dt, _)| dt)
        .unwrap_or(start);
    let id = raw.id.clone().unwrap_or_default();
    Some(Event {
        id: format!("{calendar_id}:{id}"),
        calendar_id: calendar_id.to_string(),
        uid: id,
        summary: raw.summary.clone().unwrap_or_else(|| "(no title)".into()),
        description: raw.description.clone(),
        location: raw.location.clone(),
        url: raw.html_link.clone(),
        status: raw.status.clone(),
        start,
        end,
        all_day,
        recurrence: Vec::new(),
        attendees: Vec::new(),
        categories: Vec::new(),
    })
}

/// Percent-encode a calendar id for use in a path segment.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl GoogleProvider {
    pub fn new(account_id: impl Into<String>, credentials: GoogleCredentials) -> Self {
        Self {
            account_id: account_id.into(),
            credentials,
        }
    }

    /// A fresh access token from the stored refresh token.
    fn access_token(&self) -> Result<String, McalError> {
        let refresh = crate::secret::get_refresh_token(&self.account_id)?;
        let tokens = crate::oauth::refresh_access_token(&self.credentials, &refresh)?;
        Ok(tokens.access_token)
    }

    fn calendar_ids(&self, token: &str) -> Result<Vec<GoogleCalendar>, McalError> {
        let url = format!("{API}/users/me/calendarList");
        let resp: CalendarListResponse = ureq::get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(|e| McalError::Fetch {
                url: url.clone(),
                source: Box::new(e),
            })?
            .into_json()
            .map_err(|e| McalError::Json(e.to_string()))?;
        Ok(resp.items)
    }

    fn events_for(
        &self,
        token: &str,
        calendar_id: &str,
        window: Window,
    ) -> Result<Vec<Event>, McalError> {
        let mapped_id = format!("google:{calendar_id}");
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut req = ureq::get(&format!(
                "{API}/calendars/{}/events",
                urlencode(calendar_id)
            ))
            .set("Authorization", &format!("Bearer {token}"))
            .query("singleEvents", "true")
            .query("orderBy", "startTime")
            .query("maxResults", "2500")
            .query("timeMin", &window.0.to_rfc3339())
            .query("timeMax", &window.1.to_rfc3339());
            if let Some(tok) = &page {
                req = req.query("pageToken", tok);
            }
            let resp: EventsResponse = req
                .call()
                .map_err(|e| McalError::Fetch {
                    url: format!("{API}/calendars/{calendar_id}/events"),
                    source: Box::new(e),
                })?
                .into_json()
                .map_err(|e| McalError::Json(e.to_string()))?;
            for raw in &resp.items {
                if let Some(ev) = map_event(raw, &mapped_id) {
                    out.push(ev);
                }
            }
            match resp.next_page_token {
                Some(tok) => page = Some(tok),
                None => break,
            }
        }
        Ok(out)
    }
}

impl Provider for GoogleProvider {
    fn calendars(&self) -> Result<Vec<Calendar>, McalError> {
        let token = self.access_token()?;
        Ok(self
            .calendar_ids(&token)?
            .into_iter()
            .map(|c| Calendar {
                account_id: self.account_id.clone(),
                remote_id: format!("google:{}", c.id),
                name: c.summary.unwrap_or_else(|| c.id.clone()),
                color: c.background_color,
            })
            .collect())
    }

    fn events(&self, window: Window) -> Result<Vec<Event>, McalError> {
        let token = self.access_token()?;
        let mut out = Vec::new();
        for cal in self.calendar_ids(&token)? {
            out.extend(self.events_for(&token, &cal.id, window)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> GoogleEvent {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn maps_a_timed_event() {
        let raw = parse(
            r#"{"id":"e1","summary":"Standup","location":"Meet",
                "start":{"dateTime":"2026-07-03T09:00:00+03:00"},
                "end":{"dateTime":"2026-07-03T09:30:00+03:00"}}"#,
        );
        let ev = map_event(&raw, "google:primary").unwrap();
        assert_eq!(ev.summary, "Standup");
        assert_eq!(ev.location.as_deref(), Some("Meet"));
        assert!(!ev.all_day);
        assert_eq!(ev.start.to_rfc3339(), "2026-07-03T06:00:00+00:00");
        assert_eq!(ev.calendar_id, "google:primary");
    }

    #[test]
    fn maps_an_all_day_event() {
        let raw = parse(
            r#"{"id":"e2","summary":"Holiday",
                "start":{"date":"2026-07-04"},"end":{"date":"2026-07-05"}}"#,
        );
        let ev = map_event(&raw, "google:primary").unwrap();
        assert!(ev.all_day);
        assert_eq!(ev.start.to_rfc3339(), "2026-07-04T00:00:00+00:00");
    }

    #[test]
    fn skips_cancelled_events() {
        let raw = parse(r#"{"id":"e3","status":"cancelled","start":{"date":"2026-07-04"}}"#);
        assert!(map_event(&raw, "google:primary").is_none());
    }

    #[test]
    fn skips_events_without_a_start() {
        let raw = parse(r#"{"id":"e4","summary":"x"}"#);
        assert!(map_event(&raw, "google:primary").is_none());
    }
}
