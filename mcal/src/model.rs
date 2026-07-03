//! Calendar domain model — ported from dankcalendar `models/*.go`.
//!
//! Everything is stored in UTC (`DateTime<Utc>`); the UI renders in local time.
//! Slice 1 is read-only, so the `EventCreate`/`EventUpdate` structs from the Go
//! source are intentionally not ported yet.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single calendar occurrence. For a recurring master this is the series
/// definition (with `recurrence` populated); `recur::expand` turns it into
/// concrete dated instances that share the master's `uid`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub calendar_id: String,
    pub uid: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    /// Raw RRULE / RDATE / EXDATE lines, kept verbatim for the recurrence pass.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recurrence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<Attendee>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
}

impl Event {
    /// True if this event has any recurrence rule to expand.
    pub fn is_recurring(&self) -> bool {
        self.recurrence.iter().any(|line| {
            let up = line.trim_start().to_ascii_uppercase();
            up.starts_with("RRULE") || up.starts_with("RDATE")
        })
    }
}

/// A meeting participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attendee {
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// A calendar within an account (a `.ics` file, a directory, or a subscription).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calendar {
    pub account_id: String,
    /// Stable id within the account: `file:foo.ics`, `dir:foo`, or the URL.
    pub remote_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// A source of calendars. Slice 1 has only local files and remote ICS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub kind: AccountKind,
    pub name: String,
}

/// The provider kind backing an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    /// `.ics` files and directories under a local root.
    Local,
    /// A remote `.ics` subscription URL (read-only).
    RemoteIcs,
}
