//! The pure half of Screen Time: per-day, per-app focused seconds and the
//! tracker that turns focus / away / lock changes into credited time.
//!
//! Time is credited in segments `[since, now)` of local wall-clock time; a
//! segment that crosses midnight is split so each day gets its own part
//! (the rollover). A segment longer than [`MAX_SEGMENT`] is dropped: the
//! owner task wakes every minute, so a longer gap means the machine was
//! suspended or the clock jumped, and that time was not spent at the
//! screen.

use chrono::{Duration, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Longest gap credited at once (see the module docs).
pub(crate) const MAX_SEGMENT: Duration = Duration::minutes(10);

/// The history file: `{ "version": 1, "days": { "2026-11-18": { "firefox": 3600 } } }`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct History {
    #[serde(default)]
    pub(crate) version: u32,
    /// Local date (`YYYY-MM-DD`) → app id → seconds.
    #[serde(default)]
    pub(crate) days: BTreeMap<String, BTreeMap<String, u64>>,
}

pub(crate) fn day_key(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

impl History {
    pub(crate) fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let mut h: Self = serde_json::from_str(text)?;
        h.days
            .retain(|k, _| NaiveDate::parse_from_str(k, "%Y-%m-%d").is_ok());
        Ok(h)
    }

    pub(crate) fn to_json(&self) -> String {
        let mut out = self.clone();
        out.version = 1;
        serde_json::to_string_pretty(&out).unwrap_or_default()
    }

    /// Credit `app` with `[from, to)`, split at each midnight.
    pub(crate) fn credit(&mut self, app: &str, from: NaiveDateTime, to: NaiveDateTime) {
        let mut start = from;
        while start < to {
            let next_midnight = start
                .date()
                .succ_opt()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .unwrap_or(to);
            let end = next_midnight.min(to);
            let secs = (end - start).num_seconds().max(0) as u64;
            if secs > 0 {
                *self
                    .days
                    .entry(day_key(start.date()))
                    .or_default()
                    .entry(app.to_string())
                    .or_default() += secs;
            }
            start = end;
        }
    }

    /// Drop days older than `retention_days` before `today` (0 keeps today only).
    pub(crate) fn prune(&mut self, today: NaiveDate, retention_days: u32) {
        let oldest = day_key(today - Duration::days(i64::from(retention_days)));
        self.days.retain(|k, _| *k >= oldest);
    }

    pub(crate) fn day_total(&self, d: NaiveDate) -> u64 {
        self.days
            .get(&day_key(d))
            .map_or(0, |apps| apps.values().sum())
    }

    /// Apps of the `days` days ending at `today`, summed, most used first,
    /// those under `min_secs` left out.
    pub(crate) fn top_apps(
        &self,
        today: NaiveDate,
        days: u32,
        min_secs: u64,
    ) -> Vec<(String, u64)> {
        let mut sum: BTreeMap<&str, u64> = BTreeMap::new();
        for i in 0..days.max(1) {
            let d = today - Duration::days(i64::from(i));
            if let Some(apps) = self.days.get(&day_key(d)) {
                for (app, s) in apps {
                    *sum.entry(app).or_default() += s;
                }
            }
        }
        let mut v: Vec<(String, u64)> = sum
            .into_iter()
            .filter(|(_, s)| *s >= min_secs.max(1))
            .map(|(a, s)| (a.to_string(), s))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// `(date, total)` for the `days` days ending at `today`, oldest first.
    pub(crate) fn daily_totals(&self, today: NaiveDate, days: u32) -> Vec<(NaiveDate, u64)> {
        (0..days.max(1))
            .rev()
            .map(|i| {
                let d = today - Duration::days(i64::from(i));
                (d, self.day_total(d))
            })
            .collect()
    }
}

/// Turns focus / away / lock changes into credited segments.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tracker {
    pub(crate) history: History,
    app: Option<String>,
    away: bool,
    locked: bool,
    since: Option<NaiveDateTime>,
}

impl Tracker {
    pub(crate) fn new(history: History) -> Self {
        Self {
            history,
            ..Self::default()
        }
    }

    pub(crate) fn counting(&self) -> bool {
        self.app.is_some() && !self.away && !self.locked
    }

    pub(crate) fn app(&self) -> Option<&str> {
        self.app.as_deref()
    }

    /// Credit the running segment up to `now` and start a new one there.
    /// Returns whether anything was credited.
    pub(crate) fn flush(&mut self, now: NaiveDateTime) -> bool {
        let mut credited = false;
        if let Some(since) = self.since
            && self.counting()
            && now > since
            && now - since <= MAX_SEGMENT
            && let Some(app) = self.app.clone()
        {
            self.history.credit(&app, since, now);
            credited = true;
        }
        self.since = Some(now);
        credited
    }

    pub(crate) fn set_app(&mut self, app: Option<String>, now: NaiveDateTime) {
        let app = app.filter(|a| !a.trim().is_empty());
        if app != self.app {
            self.flush(now);
            self.app = app;
        }
    }

    pub(crate) fn set_away(&mut self, away: bool, now: NaiveDateTime) {
        if away != self.away {
            self.flush(now);
            self.away = away;
        }
    }

    pub(crate) fn set_locked(&mut self, locked: bool, now: NaiveDateTime) {
        if locked != self.locked {
            self.flush(now);
            self.locked = locked;
        }
    }
}

/// `3h 12m`, `45m`, `<1m`.
pub(crate) fn format_duration(secs: u64) -> String {
    let m = secs / 60;
    match (m / 60, m % 60) {
        (0, 0) => "<1m".to_string(),
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(d: u32, h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, d)
            .and_then(|d| d.and_hms_opt(h, m, s))
            .unwrap_or_default()
    }

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap_or_default()
    }

    #[test]
    fn focus_changes_credit_the_previous_app() {
        let mut tr = Tracker::default();
        tr.set_app(Some("firefox".into()), t(18, 10, 0, 0));
        tr.set_app(Some("kitty".into()), t(18, 10, 5, 0));
        tr.flush(t(18, 10, 6, 0));
        assert_eq!(tr.history.days["2026-11-18"]["firefox"], 300);
        assert_eq!(tr.history.days["2026-11-18"]["kitty"], 60);
        assert_eq!(tr.history.day_total(date(18)), 360);
    }

    #[test]
    fn away_and_lock_pause_counting() {
        let mut tr = Tracker::default();
        tr.set_app(Some("code".into()), t(18, 9, 0, 0));
        tr.set_away(true, t(18, 9, 1, 0));
        tr.flush(t(18, 9, 5, 0));
        tr.set_away(false, t(18, 9, 6, 0));
        tr.set_locked(true, t(18, 9, 7, 0));
        tr.flush(t(18, 9, 9, 0));
        tr.set_locked(false, t(18, 9, 9, 0));
        tr.flush(t(18, 9, 9, 30));
        assert_eq!(tr.history.days["2026-11-18"]["code"], 60 + 60 + 30);
    }

    #[test]
    fn no_focus_counts_nothing_and_blank_ids_are_no_focus() {
        let mut tr = Tracker::default();
        tr.set_app(Some("  ".into()), t(18, 9, 0, 0));
        assert!(!tr.counting());
        tr.flush(t(18, 9, 5, 0));
        assert!(tr.history.days.is_empty());
    }

    #[test]
    fn a_segment_over_midnight_is_split_between_days() {
        let mut tr = Tracker::default();
        tr.set_app(Some("mpv".into()), t(17, 23, 58, 0));
        tr.flush(t(18, 0, 3, 0));
        assert_eq!(tr.history.days["2026-11-17"]["mpv"], 120);
        assert_eq!(tr.history.days["2026-11-18"]["mpv"], 180);
    }

    #[test]
    fn long_gaps_and_clock_jumps_are_not_credited() {
        let mut tr = Tracker::default();
        tr.set_app(Some("a".into()), t(18, 8, 0, 0));
        tr.flush(t(18, 9, 0, 0)); // suspended for an hour
        tr.flush(t(18, 8, 30, 0)); // clock went back
        tr.flush(t(18, 8, 31, 0));
        assert_eq!(tr.history.days["2026-11-18"]["a"], 60);
    }

    #[test]
    fn top_apps_sum_days_sort_and_hide_small_ones() {
        let mut h = History::default();
        h.credit("a", t(17, 10, 0, 0), t(17, 11, 0, 0));
        h.credit("b", t(18, 10, 0, 0), t(18, 10, 50, 0));
        h.credit("a", t(18, 12, 0, 0), t(18, 12, 0, 30));
        h.credit("c", t(18, 13, 0, 0), t(18, 13, 0, 20));
        assert_eq!(h.top_apps(date(18), 1, 60), vec![("b".to_string(), 3000)]);
        assert_eq!(
            h.top_apps(date(18), 7, 60),
            vec![("a".to_string(), 3630), ("b".to_string(), 3000)]
        );
        assert_eq!(h.top_apps(date(18), 1, 0).len(), 3);
        let totals = h.daily_totals(date(18), 3);
        assert_eq!(totals.len(), 3);
        assert_eq!(totals[0], (date(16), 0));
        assert_eq!(totals[2], (date(18), 3050));
    }

    #[test]
    fn prune_keeps_the_retention_window_and_json_round_trips() {
        let mut h = History::default();
        h.credit("a", t(1, 10, 0, 0), t(1, 10, 1, 0));
        h.credit("a", t(17, 10, 0, 0), t(17, 10, 1, 0));
        h.credit("a", t(18, 10, 0, 0), t(18, 10, 1, 0));
        h.prune(date(18), 1);
        assert_eq!(
            h.days.keys().collect::<Vec<_>>(),
            ["2026-11-17", "2026-11-18"]
        );
        let back = History::from_json(&h.to_json()).unwrap_or_default();
        assert_eq!(back.days, h.days);
        let junk = History::from_json(r#"{"days":{"nope":{"a":1},"2026-11-18":{"a":5}}}"#)
            .unwrap_or_default();
        assert_eq!(junk.days.len(), 1);
        assert!(History::from_json("[").is_err());
    }

    #[test]
    fn durations_read_as_hours_and_minutes() {
        assert_eq!(format_duration(30), "<1m");
        assert_eq!(format_duration(45 * 60), "45m");
        assert_eq!(format_duration(2 * 3600), "2h");
        assert_eq!(format_duration(3 * 3600 + 12 * 60 + 59), "3h 12m");
    }
}
