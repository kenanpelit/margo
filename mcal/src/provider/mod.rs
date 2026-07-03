//! Read-only calendar sources.
//!
//! A [`Provider`] lists its calendars and returns the events overlapping a
//! window, with recurrence already expanded. Providers are **blocking** (file
//! IO, `ureq`) — the shell runs [`load_all`] off the GTK thread and hands the
//! result back through relm4's command loop.

mod google;
mod local;
mod remote_ics;

pub use google::GoogleProvider;
pub use local::LocalProvider;
pub use remote_ics::RemoteIcsProvider;

use crate::account::AccountStore;
use crate::config::CalendarConfig;
use crate::credentials::load_google;
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

    load_account_providers(window, &mut events);

    events
}

/// Build providers from the mcal account store (Google this slice) and collect
/// their events. A missing store, missing credentials, or a dead token is
/// logged and skipped — never a hard failure.
fn load_account_providers(window: Window, out: &mut Vec<Event>) {
    let store = match AccountStore::load() {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!(%err, "mcal: account store unreadable");
            return;
        }
    };
    if store.accounts.iter().all(|a| a.kind != "google") {
        return;
    }
    let credentials = match load_google() {
        Ok(Some(creds)) => creds,
        Ok(None) => {
            tracing::warn!("mcal: google accounts configured but no credentials.toml");
            return;
        }
        Err(err) => {
            tracing::warn!(%err, "mcal: credentials unreadable");
            return;
        }
    };
    for account in store.accounts.iter().filter(|a| a.kind == "google") {
        let provider = GoogleProvider::new(account.id.clone(), credentials.clone());
        collect(&provider, window, out);
    }
}

fn collect(provider: &impl Provider, window: Window, out: &mut Vec<Event>) {
    match provider.events(window) {
        Ok(mut events) => out.append(&mut events),
        Err(err) => tracing::warn!(%err, "mcal: provider load failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // Hermetic guard for the local source. `load_all`'s account-store branch
    // reads real config + hits the network, so it is verified manually (the
    // connected Google account showing up), not here — a `load_all` test would
    // otherwise fetch live events on any dev machine that has an account.
    #[test]
    fn local_provider_loads_an_ics_event() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("a.ics"),
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:u@x\r\nSUMMARY:S\r\nDTSTART:20260703T090000Z\r\nDTEND:20260703T093000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let provider = LocalProvider::new("local", tmp.path()).unwrap();
        let window = (
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap(),
        );
        assert_eq!(provider.events(window).unwrap().len(), 1);
    }
}
