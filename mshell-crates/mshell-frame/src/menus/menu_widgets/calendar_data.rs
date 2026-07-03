//! Shared calendar-loading glue for the event-aware calendar widgets.
//!
//! Both the full `Calendar` (clock menu) and the bare `CalendarGrid`
//! (dashboard) load the same events from `mcal` and mark the same days. This
//! module holds the parts they share: snapshotting the shell's `calendars`
//! config, the load window, the off-thread fetch, and applying day marks to a
//! `gtk::Calendar`. The agenda list is unique to the full widget and stays
//! there.

use chrono::{DateTime, Duration, Utc};
use mshell_config::schema::config::{CalendarsStoreFields, ConfigStoreFields};
use reactive_graph::traits::GetUntracked;
use relm4::gtk;
use std::path::PathBuf;

/// How far either side of "now" events are loaded, so month navigation within
/// roughly a year needs no reload.
const LOAD_WINDOW_DAYS: i64 = 400;

/// The `[now - N, now + N]` UTC window to load events for.
pub(crate) fn load_window() -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    (
        now - Duration::days(LOAD_WINDOW_DAYS),
        now + Duration::days(LOAD_WINDOW_DAYS),
    )
}

/// Snapshot the shell's `calendars` config into mcal's own config struct.
/// **Call on the GTK/main thread** (reads the reactive store), then move the
/// result into the async fetch.
pub(crate) fn shell_calendar_config() -> mcal::CalendarConfig {
    let cm = mshell_config::config_manager::config_manager();
    let local_dir = cm.config().calendars().local_dir().get_untracked();
    let subscriptions = cm.config().calendars().subscriptions().get_untracked();
    let refresh_secs = cm.config().calendars().refresh_secs().get_untracked();

    mcal::CalendarConfig {
        local_dir: if local_dir.trim().is_empty() {
            mcal::default_local_dir()
        } else {
            expand_tilde(local_dir.trim())
        },
        subscriptions: subscriptions
            .into_iter()
            .filter(|sub| !sub.url.trim().is_empty())
            .map(|sub| mcal::Subscription {
                name: sub.name,
                url: sub.url,
                color: (!sub.color.trim().is_empty()).then_some(sub.color),
            })
            .collect(),
        refresh_secs,
    }
}

/// Run the (blocking) load on a background task. Returns an empty set if the
/// blocking task is cancelled.
pub(crate) async fn fetch(
    config: mcal::CalendarConfig,
    window: (DateTime<Utc>, DateTime<Utc>),
) -> Vec<mcal::Event> {
    tokio::task::spawn_blocking(move || mcal::load_all(&config, window))
        .await
        .unwrap_or_default()
}

/// Re-mark the grid: a mark on every day of the *visible* month with ≥1 event.
pub(crate) fn refresh_marks(calendar: &gtk::Calendar, events: &[mcal::Event]) {
    calendar.clear_marks();
    let shown = calendar.date();
    for day in mcal::days_with_events(events, shown.year(), shown.month() as u32) {
        calendar.mark_day(day);
    }
}

/// Expand a leading `~/` to `$HOME`.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}
