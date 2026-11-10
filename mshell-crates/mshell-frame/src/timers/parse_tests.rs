//! The parser table: every supported form, English and Turkish, plus the
//! edge cases (midnight wrap, `0m`, huge values, DST) and the errors.

use super::*;
use chrono::{FixedOffset, Local};
use chrono_tz::Europe::Berlin;

/// Tuesday 2026-11-10 14:00:00 in Istanbul (+03:00, no DST).
fn now() -> DateTime<FixedOffset> {
    at("2026-11-10 14:00:00")
}

fn at(s: &str) -> DateTime<FixedOffset> {
    let tz = FixedOffset::east_opt(3 * 3600).expect("valid offset");
    let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").expect("valid fixture");
    tz.from_local_datetime(&naive).single().expect("unique")
}

use TimerKind::{Reminder as R, Timer as T};

/// `(input, due, label, kind)` at [`now`].
const OK: &[(&str, &str, &str, TimerKind)] = &[
    // ── durations ──
    ("10m tea", "2026-11-10 14:10:00", "tea", T),
    ("1h 30m", "2026-11-10 15:30:00", "", T),
    ("1h30m", "2026-11-10 15:30:00", "", T),
    ("45s", "2026-11-10 14:00:45", "", T),
    ("90s", "2026-11-10 14:01:30", "", T),
    ("25m Pomodoro", "2026-11-10 14:25:00", "Pomodoro", T),
    ("in 10m tea", "2026-11-10 14:10:00", "tea", T),
    ("after 5m - stretch", "2026-11-10 14:05:00", "stretch", T),
    ("5 seconds", "2026-11-10 14:00:05", "", T),
    ("1 second", "2026-11-10 14:00:01", "", T),
    ("10 sec", "2026-11-10 14:00:10", "", T),
    ("10 min", "2026-11-10 14:10:00", "", T),
    ("10 mins", "2026-11-10 14:10:00", "", T),
    ("10 minutes", "2026-11-10 14:10:00", "", T),
    ("1 minute", "2026-11-10 14:01:00", "", T),
    ("2 hours", "2026-11-10 16:00:00", "", T),
    ("2 hrs", "2026-11-10 16:00:00", "", T),
    ("2hr", "2026-11-10 16:00:00", "", T),
    ("1 hour and 30 minutes", "2026-11-10 15:30:00", "", T),
    ("1h 30m 15s", "2026-11-10 15:30:15", "", T),
    ("2m30s", "2026-11-10 14:02:30", "", T),
    ("1h30", "2026-11-10 15:30:00", "", T),
    ("1m30", "2026-11-10 14:01:30", "", T),
    ("1.5h", "2026-11-10 15:30:00", "", T),
    ("0.5m", "2026-11-10 14:00:30", "", T),
    ("an hour", "2026-11-10 15:00:00", "", T),
    ("a minute stretch", "2026-11-10 14:01:00", "stretch", T),
    ("half an hour coffee", "2026-11-10 14:30:00", "coffee", T),
    ("25", "2026-11-10 14:25:00", "", T),
    ("25 Pomodoro", "2026-11-10 14:25:00", "Pomodoro", T),
    ("timer 5m", "2026-11-10 14:05:00", "", T),
    ("timer 5", "2026-11-10 14:05:00", "", T),
    ("set a timer for 10 minutes", "2026-11-10 14:10:00", "", T),
    (
        "remind me in 10m to call mom",
        "2026-11-10 14:10:00",
        "call mom",
        T,
    ),
    ("tea in 10m", "2026-11-10 14:10:00", "tea", T),
    ("tea 10m", "2026-11-10 14:10:00", "tea", T),
    (
        "call mom in 10m about dinner",
        "2026-11-10 14:10:00",
        "call mom about dinner",
        T,
    ),
    ("10m from now stretch", "2026-11-10 14:10:00", "stretch", T),
    ("10m later stretch", "2026-11-10 14:10:00", "stretch", T),
    ("10m for tea", "2026-11-10 14:10:00", "tea", T),
    ("2d", "2026-11-12 14:00:00", "", T),
    ("2 days backup", "2026-11-12 14:00:00", "backup", T),
    ("in 1:30", "2026-11-10 15:30:00", "", T),
    ("in 0:45 laundry", "2026-11-10 14:45:00", "laundry", T),
    ("10M TEA", "2026-11-10 14:10:00", "TEA", T),
    (
        "  10m   green   tea  ",
        "2026-11-10 14:10:00",
        "green tea",
        T,
    ),
    ("for 10m", "2026-11-10 14:10:00", "", T),
    ("10m - tea - green", "2026-11-10 14:10:00", "tea - green", T),
    ("10m — tea", "2026-11-10 14:10:00", "tea", T),
    ("10m – tea", "2026-11-10 14:10:00", "tea", T),
    ("10m, tea", "2026-11-10 14:10:00", "tea", T),
    ("buy 2 eggs in 10m", "2026-11-10 14:10:00", "buy 2 eggs", T),
    ("İN 10M", "2026-11-10 14:10:00", "", T),
    ("365d", "2027-11-10 14:00:00", "", T),
    // ── Turkish durations ──
    ("10dk", "2026-11-10 14:10:00", "", T),
    ("10 dk", "2026-11-10 14:10:00", "", T),
    ("10 dak", "2026-11-10 14:10:00", "", T),
    ("10 dakika", "2026-11-10 14:10:00", "", T),
    ("10 dk sonra çay", "2026-11-10 14:10:00", "çay", T),
    (
        "10 dakika sonra çay demle",
        "2026-11-10 14:10:00",
        "çay demle",
        T,
    ),
    ("30sn", "2026-11-10 14:00:30", "", T),
    ("30 sn", "2026-11-10 14:00:30", "", T),
    ("30 saniye", "2026-11-10 14:00:30", "", T),
    ("1sa", "2026-11-10 15:00:00", "", T),
    ("1 saat", "2026-11-10 15:00:00", "", T),
    ("1 saat 30 dakika", "2026-11-10 15:30:00", "", T),
    ("1 saat ve 30 dakika", "2026-11-10 15:30:00", "", T),
    ("1sa30dk", "2026-11-10 15:30:00", "", T),
    ("1,5 saat", "2026-11-10 15:30:00", "", T),
    ("yarım saat", "2026-11-10 14:30:00", "", T),
    ("YARIM SAAT mola", "2026-11-10 14:30:00", "mola", T),
    ("çeyrek saat", "2026-11-10 14:15:00", "", T),
    ("1 buçuk saat", "2026-11-10 15:30:00", "", T),
    (
        "bir saat sonra toplantı",
        "2026-11-10 15:00:00",
        "toplantı",
        T,
    ),
    ("çay 10 dk sonra", "2026-11-10 14:10:00", "çay", T),
    ("10 dakika içinde çık", "2026-11-10 14:10:00", "çık", T),
    ("2 gün", "2026-11-12 14:00:00", "", T),
    ("zamanlayıcı 5 dk", "2026-11-10 14:05:00", "", T),
    // ── clock times ──
    ("14:30 meeting", "2026-11-10 14:30:00", "meeting", R),
    ("14.30 meeting", "2026-11-10 14:30:00", "meeting", R),
    ("9:05", "2026-11-11 09:05:00", "", R),
    ("09:05:30", "2026-11-11 09:05:30", "", R),
    ("11am standup", "2026-11-11 11:00:00", "standup", R),
    ("11 am standup", "2026-11-11 11:00:00", "standup", R),
    ("11:30pm", "2026-11-10 23:30:00", "", R),
    ("11.30 pm", "2026-11-10 23:30:00", "", R),
    ("11pm", "2026-11-10 23:00:00", "", R),
    ("12am", "2026-11-11 00:00:00", "", R),
    ("12pm", "2026-11-11 12:00:00", "", R),
    ("12:15am", "2026-11-11 00:15:00", "", R),
    ("3 p.m.", "2026-11-10 15:00:00", "", R),
    ("3pm", "2026-11-10 15:00:00", "", R),
    ("3PM Call", "2026-11-10 15:00:00", "Call", R),
    ("tomorrow 09:00 coffee", "2026-11-11 09:00:00", "coffee", R),
    (
        "tomorrow 09:00 - coffee",
        "2026-11-11 09:00:00",
        "coffee",
        R,
    ),
    ("tomorrow at 9am", "2026-11-11 09:00:00", "", R),
    ("tomorrow 15:00", "2026-11-11 15:00:00", "", R),
    ("today 18:00", "2026-11-10 18:00:00", "", R),
    ("at 15", "2026-11-10 15:00:00", "", R),
    ("at 3", "2026-11-10 15:00:00", "", R),
    ("at 1", "2026-11-11 01:00:00", "", R),
    ("at 7 dinner", "2026-11-10 19:00:00", "dinner", R),
    ("at 12", "2026-11-11 00:00:00", "", R),
    ("tomorrow at 9", "2026-11-11 09:00:00", "", R),
    ("today at 3", "2026-11-10 15:00:00", "", R),
    ("meeting at 14:30", "2026-11-10 14:30:00", "meeting", R),
    ("14:00", "2026-11-11 14:00:00", "", R),
    ("13:59", "2026-11-11 13:59:00", "", R),
    ("14:01", "2026-11-10 14:01:00", "", R),
    ("00:00", "2026-11-11 00:00:00", "", R),
    ("24:00", "2026-11-11 00:00:00", "", R),
    ("23:59", "2026-11-10 23:59:00", "", R),
    ("12.50", "2026-11-11 12:50:00", "", R),
    ("noon lunch", "2026-11-11 12:00:00", "lunch", R),
    ("midnight", "2026-11-11 00:00:00", "", R),
    ("9am tomorrow", "2026-11-11 09:00:00", "", R),
    ("coffee tomorrow 9am", "2026-11-11 09:00:00", "coffee", R),
    (
        "table for 2 at 19:00",
        "2026-11-10 19:00:00",
        "table for 2",
        R,
    ),
    ("room 101 at 14:30", "2026-11-10 14:30:00", "room 101", R),
    ("call @ 16:00", "2026-11-10 16:00:00", "call", R),
    // ── Turkish clock times ──
    ("YARIN 09:00 kahve", "2026-11-11 09:00:00", "kahve", R),
    ("yarın 09:00 kahve", "2026-11-11 09:00:00", "kahve", R),
    ("yarin 9:00", "2026-11-11 09:00:00", "", R),
    ("09:00 yarın", "2026-11-11 09:00:00", "", R),
    ("saat 15:30 toplantı", "2026-11-10 15:30:00", "toplantı", R),
    ("toplantı saat 15:30", "2026-11-10 15:30:00", "toplantı", R),
    ("saat 3", "2026-11-10 15:00:00", "", R),
    ("15:30'da toplantı", "2026-11-10 15:30:00", "toplantı", R),
    ("15:30’da toplantı", "2026-11-10 15:30:00", "toplantı", R),
    ("15:30 da toplantı", "2026-11-10 15:30:00", "toplantı", R),
    ("bugün 18:00 spor", "2026-11-10 18:00:00", "spor", R),
    ("bugun 18:00", "2026-11-10 18:00:00", "", R),
    ("yarın saat 8 koşu", "2026-11-11 08:00:00", "koşu", R),
    ("öğlen yemek", "2026-11-11 12:00:00", "yemek", R),
    ("gece yarısı", "2026-11-11 00:00:00", "", R),
];

#[test]
fn every_supported_form_parses() {
    let now = now();
    let mut failures = Vec::new();
    for (input, due, label, kind) in OK {
        match parse(&now, input) {
            Ok(p) => {
                let got = p.due.format("%Y-%m-%d %H:%M:%S").to_string();
                if got != *due || p.label != *label || p.kind != *kind {
                    failures.push(format!(
                        "{input:?}: got ({got}, {:?}, {:?}), want ({due}, {label:?}, {kind:?})",
                        p.label, p.kind
                    ));
                }
            }
            Err(e) => failures.push(format!("{input:?}: error {e:?}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

const ERR: &[(&str, ParseError)] = &[
    ("", ParseError::Empty),
    ("   ", ParseError::Empty),
    ("tea", ParseError::NoTime),
    ("hello world", ParseError::NoTime),
    ("- tea", ParseError::NoTime),
    ("buy 2 eggs", ParseError::NoTime),
    ("pay 12.50 bill", ParseError::NoTime),
    ("v1.5 release", ParseError::NoTime),
    ("0m", ParseError::Zero),
    ("0s tea", ParseError::Zero),
    ("0", ParseError::Zero),
    ("0 dk", ParseError::Zero),
    ("0.2s", ParseError::Zero),
    ("in 0:00", ParseError::Zero),
    ("400d", ParseError::TooFar),
    ("9000h", ParseError::TooFar),
    ("99999999999999999999h", ParseError::TooFar),
    ("99999999999999999999", ParseError::TooFar),
    ("366 gün", ParseError::TooFar),
];

#[test]
fn garbage_and_out_of_range_input_is_rejected_with_a_reason() {
    let now = now();
    let mut failures = Vec::new();
    for (input, want) in ERR {
        match parse(&now, input) {
            Ok(p) => failures.push(format!("{input:?}: parsed as {}", p.due)),
            Err(e) if e != *want => failures.push(format!("{input:?}: {e:?}, want {want:?}")),
            Err(_) => {}
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn impossible_clock_times_name_the_token() {
    let now = now();
    for bad in ["25:00", "14:60", "13pm", "0am", "at 25", "9:61pm"] {
        match parse(&now, bad) {
            Err(ParseError::BadClock(t)) => {
                assert!(bad.ends_with(&t) || t.contains(':'), "{bad}: {t}")
            }
            other => panic!("{bad}: {other:?}"),
        }
    }
}

#[test]
fn today_with_a_past_time_or_a_day_without_a_time_is_an_error() {
    let now = now();
    assert_eq!(
        parse(&now, "today 09:00").map(|p| p.due),
        Err(ParseError::Passed("09:00".into()))
    );
    assert_eq!(
        parse(&now, "bugün 13:59").map(|p| p.due),
        Err(ParseError::Passed("13:59".into()))
    );
    assert_eq!(
        parse(&now, "tomorrow 10m").map(|p| p.due),
        Err(ParseError::DayNeedsClock)
    );
    assert_eq!(
        parse(&now, "tomorrow coffee").map(|p| p.due),
        Err(ParseError::DayNeedsClock)
    );
    assert_eq!(
        parse(&now, "yarın").map(|p| p.due),
        Err(ParseError::DayNeedsClock)
    );
}

#[test]
fn every_error_message_is_helpful() {
    for e in [ParseError::Empty, ParseError::NoTime] {
        assert!(e.to_string().contains("10m tea"), "{e}");
        assert!(e.to_string().contains("10 dk sonra çay"), "{e}");
    }
    assert!(
        ParseError::BadClock("25:00".into())
            .to_string()
            .contains("25:00")
    );
    assert!(
        ParseError::Passed("09:00".into())
            .to_string()
            .contains("09:00")
    );
    assert!(ParseError::TooFar.to_string().contains("365"));
}

#[test]
fn clock_times_wrap_past_midnight_and_the_year() {
    let late = at("2026-11-10 23:50:00");
    let due = |s| {
        parse(&late, s)
            .expect("parses")
            .due
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    };
    assert_eq!(due("00:30"), "2026-11-11 00:30:00");
    assert_eq!(due("10m"), "2026-11-11 00:00:00");
    assert_eq!(due("23:55"), "2026-11-10 23:55:00");
    assert_eq!(due("23:45"), "2026-11-11 23:45:00");
    assert_eq!(due("tomorrow 00:30"), "2026-11-11 00:30:00");
    assert_eq!(due("at 1"), "2026-11-11 01:00:00");

    let new_year = at("2026-12-31 23:59:30");
    let p = parse(&new_year, "1m").expect("parses");
    assert_eq!(
        p.due.format("%Y-%m-%d %H:%M:%S").to_string(),
        "2027-01-01 00:00:30"
    );
    let p = parse(&new_year, "tomorrow 8am").expect("parses");
    assert_eq!(
        p.due.format("%Y-%m-%d %H:%M:%S").to_string(),
        "2027-01-01 08:00:00"
    );
}

#[test]
fn seconds_of_now_are_kept_for_durations_and_dropped_for_clock_times() {
    let now = at("2026-11-10 14:00:42");
    let p = parse(&now, "10m").expect("parses");
    assert_eq!(p.due.format("%H:%M:%S").to_string(), "14:10:42");
    let p = parse(&now, "14:30").expect("parses");
    assert_eq!(p.due.format("%H:%M:%S").to_string(), "14:30:00");
}

#[test]
fn clock_times_resolve_in_the_zone_of_now_across_dst() {
    // Spring forward: 02:00 → 03:00 on 2026-03-29 in Berlin. 02:30 does
    // not exist; it rings at 03:30 local, the first time after the gap.
    let tz = Berlin;
    let before = tz
        .with_ymd_and_hms(2026, 3, 29, 1, 0, 0)
        .single()
        .expect("unique");
    let p = parse(&before, "02:30").expect("parses");
    assert_eq!(p.due.format("%H:%M %z").to_string(), "03:30 +0200");
    // A duration is real elapsed time: 2 h after 01:00 CET is 04:00 CEST.
    let p = parse(&before, "2h").expect("parses");
    assert_eq!(p.due.format("%H:%M %z").to_string(), "04:00 +0200");
    assert_eq!((p.due - before).num_seconds(), 7200);

    // Fall back: 02:30 happens twice on 2026-10-25; the earlier one wins.
    let before = tz
        .with_ymd_and_hms(2026, 10, 25, 1, 0, 0)
        .single()
        .expect("unique");
    let p = parse(&before, "02:30").expect("parses");
    assert_eq!(p.due.format("%H:%M %z").to_string(), "02:30 +0200");
}

#[test]
fn the_local_zone_works_end_to_end() {
    let now = Local::now();
    let p = parse(&now, "10m tea").expect("parses");
    assert_eq!((p.due - now).num_seconds(), 600);
    let p = parse(&now, "tomorrow 09:00").expect("parses");
    assert!(p.due > now);
    assert_eq!(p.due.format("%H:%M").to_string(), "09:00");
}

#[test]
fn labels_are_trimmed_capped_and_free_of_control_characters() {
    let now = now();
    let long = format!("10m {}", "x".repeat(500));
    let p = parse(&now, &long).expect("parses");
    assert_eq!(p.label.chars().count(), MAX_LABEL_CHARS);
    let p = parse(&now, "10m tea\u{7}\u{1b}[31m").expect("parses");
    assert_eq!(p.label, "tea[31m");
    let p = parse(&now, "10m - ").expect("parses");
    assert_eq!(p.label, "");
}

#[test]
fn turkish_letters_fold_for_keywords_but_labels_keep_them() {
    assert_eq!(fold("YARIN"), "yarin");
    assert_eq!(fold("yarın"), "yarin");
    assert_eq!(fold("İÇİNDE"), "icinde");
    assert_eq!(fold("Öğlen"), "oglen");
    let p = parse(&now(), "10 dk sonra Çayı demle").expect("parses");
    assert_eq!(p.label, "Çayı demle");
}

#[test]
fn glued_tokens_need_a_unit_and_reject_junk() {
    assert_eq!(glued_secs("10m"), Some(600.0));
    assert_eq!(glued_secs("1h30"), Some(5400.0));
    assert_eq!(glued_secs("1sa30dk"), Some(5400.0));
    assert_eq!(glued_secs("10"), None);
    assert_eq!(glued_secs("10x"), None);
    assert_eq!(glued_secs("m10"), None);
    assert_eq!(glued_secs("10m!"), None);
    assert_eq!(glued_secs("1.2.3m"), None);
    assert_eq!(glued_secs("30s5"), None);
}

/// The table is the spec: keep it big enough to cover the grammar.
#[test]
fn the_table_has_over_a_hundred_cases() {
    assert!(OK.len() + ERR.len() >= 140, "{}", OK.len() + ERR.len());
}
