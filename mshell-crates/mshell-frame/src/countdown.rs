//! Countdown — pure "time until / since a date" arithmetic shared by the
//! Countdown tab of the Alarm Clock menu and the `countdown` bar pill.
//!
//! Ports the DMS `TimeUntil` plugin's math. There is deliberately **no
//! wall-clock read** in this module: every entry point takes `now` as a
//! parameter, so the logic is deterministic and unit-testable. Callers
//! pass `chrono::Local::now().naive_local()`.

use chrono::{NaiveDate, NaiveDateTime};
use mshell_config::schema::config::Countdown;
use std::cell::Cell;

thread_local! {
    /// Set by the `countdown` bar pill's click; consumed by the Alarm
    /// Clock menu on its next reveal to jump to the Countdown tab. It
    /// lives in this crate-root module (reachable from both `bars` and
    /// `menus`) to avoid cross-module privacy gymnastics. GTK main
    /// thread only, so a plain `Cell` is enough.
    static PENDING_COUNTDOWN_TAB: Cell<bool> = const { Cell::new(false) };
}

/// Ask the Alarm Clock menu to open on its Countdown tab next reveal.
pub(crate) fn request_countdown_tab() {
    PENDING_COUNTDOWN_TAB.with(|p| p.set(true));
}

/// Consume the pending Countdown-tab request (true at most once per set).
pub(crate) fn take_countdown_tab_request() -> bool {
    PENDING_COUNTDOWN_TAB.with(|p| p.replace(false))
}

/// Time unit a countdown is expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CountdownUnit {
    Hours,
    Days,
    Weeks,
    Months,
}

impl CountdownUnit {
    /// Parse the config string; unknown/empty → `Days` (the config default).
    pub(crate) fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "hours" => Self::Hours,
            "weeks" => Self::Weeks,
            "months" => Self::Months,
            _ => Self::Days,
        }
    }

    /// Seconds in one unit. A month is the 30.44-day average (matches
    /// TimeUntil), so "months" stays a smooth, non-calendar approximation.
    fn seconds(self) -> f64 {
        match self {
            Self::Hours => 3_600.0,
            Self::Days => 86_400.0,
            Self::Weeks => 604_800.0,
            Self::Months => 86_400.0 * 30.44,
        }
    }

    /// Unit noun, singular or plural ("day" / "days").
    fn noun(self, plural: bool) -> &'static str {
        match (self, plural) {
            (Self::Hours, false) => "hour",
            (Self::Hours, true) => "hours",
            (Self::Days, false) => "day",
            (Self::Days, true) => "days",
            (Self::Weeks, false) => "week",
            (Self::Weeks, true) => "weeks",
            (Self::Months, false) => "month",
            (Self::Months, true) => "months",
        }
    }

    /// Compact suffix for the vertical/short pill ("d", "mo").
    fn short(self) -> &'static str {
        match self {
            Self::Hours => "h",
            Self::Days => "d",
            Self::Weeks => "w",
            Self::Months => "mo",
        }
    }
}

/// Parse a target timestamp: `YYYY-MM-DD HH:MM` or `YYYY-MM-DD` (→ midnight).
/// A `T` date/time separator is tolerated. `None` if unparseable.
pub(crate) fn parse_target(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim().replace('T', " ");
    if let Ok(dt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M") {
        return Some(dt);
    }
    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
}

/// Signed remaining time in `unit`, rounded to 0.1. Negative = overdue.
/// `None` if the target is unparseable.
pub(crate) fn remaining(target: &str, unit: CountdownUnit, now: NaiveDateTime) -> Option<f64> {
    let target = parse_target(target)?;
    let diff_secs = (target - now).num_milliseconds() as f64 / 1_000.0;
    let raw = diff_secs / unit.seconds();
    Some((raw * 10.0).round() / 10.0)
}

/// Long pill/menu form: "42 days remaining" / "1 day overdue". A custom
/// `label` replaces "remaining" only while upcoming; overdue always reads
/// "overdue" (matching TimeUntil).
pub(crate) fn format_long(value: f64, unit: CountdownUnit, label: &str) -> String {
    let abs = value.abs();
    let plural = (abs - 1.0).abs() > f64::EPSILON;
    let noun = unit.noun(plural);
    let suffix = if value < 0.0 {
        "overdue"
    } else {
        let l = label.trim();
        if l.is_empty() { "remaining" } else { l }
    };
    format!("{} {noun} {suffix}", fmt_num(abs))
}

/// Short pill form: "42d" / "3d!" (overdue → trailing `!`).
pub(crate) fn format_short(value: f64, unit: CountdownUnit) -> String {
    let overdue = if value < 0.0 { "!" } else { "" };
    format!("{}{}{overdue}", fmt_num(value.abs()), unit.short())
}

/// One decimal, dropping a trailing `.0` for whole numbers (42, not 42.0).
fn fmt_num(v: f64) -> String {
    if v.fract().abs() < f64::EPSILON {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Index of the countdown to surface on the pill: the soonest *upcoming*
/// target (smallest positive remaining); if none are upcoming, the least
/// overdue (closest to now). Disabled and unparseable entries are skipped.
/// `None` when nothing is displayable.
pub(crate) fn soonest(items: &[Countdown], now: NaiveDateTime) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, c) in items.iter().enumerate() {
        if !c.enabled {
            continue;
        }
        let Some(secs) = parse_target(&c.target).map(|t| (t - now).num_milliseconds() as f64)
        else {
            continue;
        };
        let better = match best {
            None => true,
            Some((_, best_secs)) => {
                let a_up = secs >= 0.0;
                let b_up = best_secs >= 0.0;
                match (a_up, b_up) {
                    (true, false) => true, // upcoming beats overdue
                    (false, true) => false,
                    (true, true) => secs < best_secs, // sooner upcoming wins
                    (false, false) => secs > best_secs, // less overdue wins
                }
            }
        };
        if better {
            best = Some((i, secs));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    fn cd(target: &str, enabled: bool) -> Countdown {
        Countdown {
            target: target.to_string(),
            unit: "days".to_string(),
            label: String::new(),
            enabled,
        }
    }

    #[test]
    fn parse_target_forms() {
        assert_eq!(
            parse_target("2027-01-01 21:37"),
            Some(at(2027, 1, 1, 21, 37))
        );
        assert_eq!(parse_target("2027-01-01"), Some(at(2027, 1, 1, 0, 0)));
        assert_eq!(parse_target("2027-01-01T09:00"), Some(at(2027, 1, 1, 9, 0)));
        assert_eq!(parse_target("  2027-01-01  "), Some(at(2027, 1, 1, 0, 0)));
        assert_eq!(parse_target("not a date"), None);
        assert_eq!(parse_target(""), None);
    }

    #[test]
    fn remaining_per_unit() {
        let now = at(2026, 1, 1, 0, 0);
        // 10 days out.
        assert_eq!(
            remaining("2026-01-11", CountdownUnit::Days, now),
            Some(10.0)
        );
        // Same span in hours = 240.
        assert_eq!(
            remaining("2026-01-11", CountdownUnit::Hours, now),
            Some(240.0)
        );
        // Same span in weeks ≈ 1.4.
        assert_eq!(
            remaining("2026-01-11", CountdownUnit::Weeks, now),
            Some(1.4)
        );
        // Unparseable → None.
        assert_eq!(remaining("nope", CountdownUnit::Days, now), None);
    }

    #[test]
    fn remaining_negative_when_past() {
        let now = at(2026, 1, 10, 0, 0);
        assert_eq!(
            remaining("2026-01-07", CountdownUnit::Days, now),
            Some(-3.0)
        );
    }

    #[test]
    fn long_form_wording() {
        assert_eq!(
            format_long(42.0, CountdownUnit::Days, ""),
            "42 days remaining"
        );
        assert_eq!(format_long(1.0, CountdownUnit::Days, ""), "1 day remaining");
        assert_eq!(format_long(-3.0, CountdownUnit::Days, ""), "3 days overdue");
        assert_eq!(format_long(-1.0, CountdownUnit::Days, ""), "1 day overdue");
        assert_eq!(
            format_long(5.0, CountdownUnit::Weeks, "until launch"),
            "5 weeks until launch"
        );
        // Custom label ignored while overdue.
        assert_eq!(
            format_long(-2.0, CountdownUnit::Weeks, "until launch"),
            "2 weeks overdue"
        );
        assert_eq!(
            format_long(2.5, CountdownUnit::Days, ""),
            "2.5 days remaining"
        );
    }

    #[test]
    fn short_form_wording() {
        assert_eq!(format_short(42.0, CountdownUnit::Days), "42d");
        assert_eq!(format_short(-3.0, CountdownUnit::Days), "3d!");
        assert_eq!(format_short(1.5, CountdownUnit::Months), "1.5mo");
        assert_eq!(format_short(6.0, CountdownUnit::Hours), "6h");
    }

    #[test]
    fn soonest_prefers_nearest_upcoming() {
        let now = at(2026, 1, 1, 0, 0);
        let items = vec![
            cd("2026-03-01", true), // far upcoming
            cd("2026-01-05", true), // nearest upcoming  ← want
            cd("2025-12-20", true), // overdue
        ];
        assert_eq!(soonest(&items, now), Some(1));
    }

    #[test]
    fn soonest_falls_back_to_least_overdue() {
        let now = at(2026, 1, 10, 0, 0);
        let items = vec![
            cd("2026-01-01", true), // 9 days overdue
            cd("2026-01-08", true), // 2 days overdue  ← want (least overdue)
        ];
        assert_eq!(soonest(&items, now), Some(1));
    }

    #[test]
    fn soonest_skips_disabled_and_invalid() {
        let now = at(2026, 1, 1, 0, 0);
        let items = vec![
            cd("2026-01-05", false), // disabled, nearest — skipped
            cd("garbage", true),     // unparseable — skipped
            cd("2026-02-01", true),  // ← only displayable
        ];
        assert_eq!(soonest(&items, now), Some(2));
        assert_eq!(soonest(&[], now), None);
    }
}
