//! Read-only calendar sources.
//!
//! A [`Provider`] lists its calendars and returns the events overlapping a
//! window, with recurrence already expanded. Providers are **blocking** (file
//! IO, `ureq`) — the shell runs [`load_all`] off the GTK thread and hands the
//! result back through relm4's command loop.

mod local;
mod remote_ics;

pub use local::LocalProvider;
pub use remote_ics::RemoteIcsProvider;

use crate::config::CalendarConfig;
use crate::error::McalError;
use crate::model::{Calendar, Event};
use chrono::{DateTime, Utc};

/// A `[start, end]` UTC time window to load events for.
pub type Window = (DateTime<Utc>, DateTime<Utc>);

/// A source of calendars and events.
pub trait Provider {
    /// The calendars this source exposes (name/colour for the UI).
    fn calendars(&self) -> Result<Vec<Calendar>, McalError>;
    /// Every event overlapping `window`, recurrence already expanded.
    fn events(&self, window: Window) -> Result<Vec<Event>, McalError>;
}

/// Load and merge events from every configured source, clamped to `window`.
///
/// Blocking — call from a background task. A source that errors is logged and
/// skipped so one bad subscription (or a missing local dir) never blanks the
/// whole calendar.
pub fn load_all(config: &CalendarConfig, window: Window) -> Vec<Event> {
    let mut events = Vec::new();

    match LocalProvider::new("local", &config.local_dir) {
        Ok(provider) => collect(&provider, window, &mut events),
        Err(err) => tracing::warn!(%err, "mcal: local provider unavailable"),
    }

    if !config.subscriptions.is_empty() {
        let provider = RemoteIcsProvider::new("remote", config.subscriptions.clone());
        collect(&provider, window, &mut events);
    }

    events
}

fn collect(provider: &impl Provider, window: Window, out: &mut Vec<Event>) {
    match provider.events(window) {
        Ok(mut events) => out.append(&mut events),
        Err(err) => tracing::warn!(%err, "mcal: provider load failed"),
    }
}
