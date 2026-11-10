//! Wording for timers: the live "Rings at 14:35 (in 10 min) — tea"
//! preview, the remaining-time readouts (menu rows, bar pill, CLI) and the
//! ring / missed notification text. Pure: `now` is always passed in.

use super::book::Timer;
use super::parse::TimerKind;
use chrono::{DateTime, TimeZone};
use std::time::Duration;

/// `"45 s"`, `"10 min"`, `"1 min 30 s"`, `"1 h 30 min"`, `"3 d 4 h"`.
pub fn format_in(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m, s) = (
        secs / 86_400,
        secs % 86_400 / 3600,
        secs % 3600 / 60,
        secs % 60,
    );
    match (d, h, m, s) {
        (0, 0, 0, s) => format!("{s} s"),
        (0, 0, m, 0) => format!("{m} min"),
        (0, 0, m, s) if m < 10 => format!("{m} min {s} s"),
        // Under an hour but ten minutes or more: seconds are noise.
        (0, 0, m, _) => format!("{m} min"),
        (0, h, 0, _) => format!("{h} h"),
        (0, h, m, _) => format!("{h} h {m} min"),
        (d, 0, _, _) => format!("{d} d"),
        (d, h, _, _) => format!("{d} d {h} h"),
    }
}

/// A clock time relative to today: `"14:35"`, `"tomorrow 09:00"`,
/// `"yesterday 23:10"`, `"Mon 09:00"` (this week), `"16 Nov 09:00"`.
/// `seconds` adds `:SS`.
pub fn day_clock<Tz: TimeZone>(due: &DateTime<Tz>, now: &DateTime<Tz>, seconds: bool) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let clock = due
        .format(if seconds { "%H:%M:%S" } else { "%H:%M" })
        .to_string();
    let days = (due.date_naive() - now.date_naive()).num_days();
    match days {
        0 => clock,
        1 => format!("tomorrow {clock}"),
        -1 => format!("yesterday {clock}"),
        2..=6 => format!("{} {clock}", due.format("%a")),
        _ => format!("{} {clock}", due.format("%-d %b")),
    }
}

/// Whether a readout of `due` should show seconds: short timers that do
/// not land on a whole minute.
fn wants_seconds<Tz: TimeZone>(due: &DateTime<Tz>, now: &DateTime<Tz>) -> bool {
    use chrono::Timelike;
    due.second() != 0 && (due.clone() - now.clone()).num_seconds() < 3600
}

/// The launcher / menu preview of a parsed line:
/// `"Rings at 14:35 (in 10 min) — tea"`,
/// `"Rings tomorrow 09:00 (in 19 h) — coffee"`.
pub fn describe<Tz: TimeZone>(due: &DateTime<Tz>, now: &DateTime<Tz>, label: &str) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let when = day_clock(due, now, wants_seconds(due, now));
    let at = if when.contains(' ') { "" } else { "at " };
    let left = format_in((due.clone() - now.clone()).num_seconds());
    let mut text = format!("Rings {at}{when} (in {left})");
    if !label.is_empty() {
        text.push_str(" — ");
        text.push_str(label);
    }
    text
}

/// The bar pill's remaining time: `"09:59"` under an hour, `"1h 05m"`
/// under a day, `"2d 03h"` beyond.
pub fn pill_text(remaining: i64) -> String {
    let r = remaining.max(0);
    if r < 3600 {
        format!("{:02}:{:02}", r / 60, r % 60)
    } else if r < 86_400 {
        format!("{}h {:02}m", r / 3600, r % 3600 / 60)
    } else {
        format!("{}d {:02}h", r / 86_400, r % 86_400 / 3600)
    }
}

/// How long until [`pill_text`] of `remaining` changes, so the pill wakes
/// once per displayed change: every second under an hour, once a minute
/// under a day, once an hour beyond.
pub fn pill_next_change(remaining: i64) -> Duration {
    let r = remaining.max(0);
    let secs = if r <= 3600 {
        1
    } else if r <= 86_400 {
        r % 60 + 1
    } else {
        r % 3600 + 1
    };
    Duration::from_secs(secs as u64)
}

/// The ring notification: `(summary, body)`.
pub fn ring_message(timer: &Timer, due_clock: &str) -> (String, String) {
    let length = format_in(timer.due - timer.created);
    let (title, body) = match timer.kind {
        TimerKind::Timer => ("Timer", format!("{length} timer is up.")),
        TimerKind::Reminder => ("Reminder", format!("It's {due_clock}.")),
        TimerKind::Snooze => ("Snoozed timer", format!("Snoozed for {length}.")),
    };
    let summary = if timer.label.is_empty() {
        title.to_string()
    } else {
        timer.label.clone()
    };
    (summary, body)
}

/// One notification for the timers that came due while the machine slept
/// or mshell was off. `clock` renders a timer's due time ("14:35",
/// "yesterday 23:10").
pub fn missed_message(timers: &[Timer], clock: impl Fn(&Timer) -> String) -> (String, String) {
    let name = |t: &Timer| {
        if t.label.is_empty() {
            match t.kind {
                TimerKind::Reminder => "Reminder".to_string(),
                _ => format!("{} timer", format_in(t.due - t.created)),
            }
        } else {
            t.label.clone()
        }
    };
    match timers {
        [one] => (
            format!("Missed: {}", name(one)),
            format!("Missed at {} (asleep or mshell off).", clock(one)),
        ),
        many => (
            format!("{} missed timers", many.len()),
            many.iter()
                .map(|t| format!("{} · missed at {}", name(t), clock(t)))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    }
}

/// `"rings at 14:35"` / `"rings tomorrow 09:00"` for a [`day_clock`].
pub fn rings_phrase(when: &str) -> String {
    if when.contains(' ') {
        format!("rings {when}")
    } else {
        format!("rings at {when}")
    }
}

/// A one-line row for `mshellctl timer list` / `add`:
/// `"3\t14:35\t9:59\ttea"` — id, due clock, time left, label.
pub fn list_line(timer: &Timer, due_clock: &str, now: i64) -> String {
    format!(
        "{}\t{}\t{}\t{}",
        timer.id,
        due_clock,
        pill_text(timer.remaining(now)),
        timer.label
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDateTime};

    fn at(s: &str) -> DateTime<FixedOffset> {
        let tz = FixedOffset::east_opt(3 * 3600).expect("offset");
        let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").expect("fixture");
        tz.from_local_datetime(&naive).single().expect("unique")
    }

    fn timer(kind: TimerKind, len: i64, label: &str) -> Timer {
        Timer {
            id: 3,
            due: 10_000 + len,
            created: 10_000,
            kind,
            label: label.to_string(),
        }
    }

    #[test]
    fn durations_read_naturally() {
        for (secs, want) in [
            (0, "0 s"),
            (45, "45 s"),
            (60, "1 min"),
            (90, "1 min 30 s"),
            (600, "10 min"),
            (659, "10 min"),
            (3600, "1 h"),
            (5400, "1 h 30 min"),
            (86_400, "1 d"),
            (100_000, "1 d 3 h"),
            (-5, "0 s"),
        ] {
            assert_eq!(format_in(secs), want, "{secs}");
        }
    }

    #[test]
    fn the_preview_says_when_and_how_long() {
        let now = at("2026-11-10 14:25:00");
        assert_eq!(
            describe(&at("2026-11-10 14:35:00"), &now, "tea"),
            "Rings at 14:35 (in 10 min) — tea"
        );
        assert_eq!(
            describe(&at("2026-11-10 14:25:45"), &now, ""),
            "Rings at 14:25:45 (in 45 s)"
        );
        assert_eq!(
            describe(&at("2026-11-11 09:00:00"), &now, "coffee"),
            "Rings tomorrow 09:00 (in 18 h 35 min) — coffee"
        );
        assert_eq!(
            describe(&at("2026-11-13 09:00:00"), &now, ""),
            "Rings Fri 09:00 (in 2 d 18 h)"
        );
        assert_eq!(
            describe(&at("2026-12-24 18:00:00"), &now, ""),
            "Rings 24 Dec 18:00 (in 44 d 3 h)"
        );
    }

    #[test]
    fn day_clock_names_yesterday_for_missed_timers() {
        let now = at("2026-11-11 08:00:00");
        assert_eq!(
            day_clock(&at("2026-11-10 23:10:00"), &now, false),
            "yesterday 23:10"
        );
        assert_eq!(day_clock(&at("2026-11-11 07:59:00"), &now, false), "07:59");
    }

    #[test]
    fn the_pill_shows_mm_ss_then_hours_then_days() {
        assert_eq!(pill_text(599), "09:59");
        assert_eq!(pill_text(3599), "59:59");
        assert_eq!(pill_text(3600), "1h 00m");
        assert_eq!(pill_text(3959), "1h 05m");
        assert_eq!(pill_text(90_000), "1d 01h");
        assert_eq!(pill_text(-3), "00:00");
    }

    #[test]
    fn the_pill_wakes_only_when_its_text_changes() {
        let secs = |r| pill_next_change(r).as_secs();
        assert_eq!(secs(599), 1);
        assert_eq!(secs(3600), 1, "1h 00m → 59:59 a second later");
        assert_eq!(secs(3959), 60, "1h 05m → 1h 04m at 3899");
        assert_eq!(secs(3900), 1);
        assert_eq!(secs(90_000), 1);
        assert_eq!(secs(93_599), 3600);
        // The claim behind the numbers: the text is the same until then.
        for r in [3959_i64, 3900, 7200, 93_599, 100_000] {
            let wait = pill_next_change(r).as_secs() as i64;
            assert_eq!(pill_text(r - wait + 1), pill_text(r), "{r}");
            assert_ne!(pill_text(r - wait), pill_text(r), "{r}");
        }
    }

    #[test]
    fn the_ring_notification_leads_with_the_label() {
        let (s, b) = ring_message(&timer(TimerKind::Timer, 600, "tea"), "14:35");
        assert_eq!((s.as_str(), b.as_str()), ("tea", "10 min timer is up."));
        let (s, b) = ring_message(&timer(TimerKind::Reminder, 600, ""), "14:30");
        assert_eq!((s.as_str(), b.as_str()), ("Reminder", "It's 14:30."));
        let (s, _) = ring_message(&timer(TimerKind::Snooze, 300, ""), "x");
        assert_eq!(s, "Snoozed timer");
    }

    #[test]
    fn missed_timers_say_when_they_were_due() {
        let clock = |_: &Timer| "14:35".to_string();
        let (s, b) = missed_message(&[timer(TimerKind::Timer, 600, "tea")], clock);
        assert_eq!(s, "Missed: tea");
        assert_eq!(b, "Missed at 14:35 (asleep or mshell off).");
        let (s, b) = missed_message(
            &[
                timer(TimerKind::Timer, 600, ""),
                timer(TimerKind::Reminder, 60, "standup"),
            ],
            clock,
        );
        assert_eq!(s, "2 missed timers");
        assert_eq!(
            b,
            "10 min timer · missed at 14:35\nstandup · missed at 14:35"
        );
    }

    #[test]
    fn rings_phrase_reads_with_or_without_a_day() {
        assert_eq!(rings_phrase("14:35"), "rings at 14:35");
        assert_eq!(rings_phrase("tomorrow 09:00"), "rings tomorrow 09:00");
    }

    #[test]
    fn a_list_line_is_tab_separated() {
        let t = timer(TimerKind::Timer, 600, "tea");
        assert_eq!(list_line(&t, "14:35", 10_001), "3\t14:35\t09:59\ttea");
    }
}
