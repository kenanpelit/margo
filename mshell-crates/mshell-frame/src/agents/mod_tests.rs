//! Tests for the limits fetch schedule (`refresh_provider`) and the pill's
//! headline window.

use super::*;
use limits::Severity;
use std::cell::Cell;

const T0: i64 = 1_790_000_000_000;
const MIN: i64 = 60_000;

fn window(kind: WindowKind, percent: f64) -> LimitWindow {
    LimitWindow {
        kind,
        label: format!("{kind:?}"),
        percent,
        resets_at: None,
        period_ms: 0,
        severity: Severity::default(),
    }
}

fn usage(percent: f64) -> PlanUsage {
    PlanUsage {
        provider: Provider::Claude,
        plan: "Claude Max 5x".into(),
        windows: vec![window(WindowKind::Session, percent)],
    }
}

/// Runs `refresh_provider` and says whether it called the fetcher.
fn tick(
    st: &mut FetchState,
    now: i64,
    force: bool,
    result: Result<PlanUsage, LimitsError>,
) -> bool {
    let called = Cell::new(false);
    refresh_provider(st, Provider::Claude, now, force, || {
        called.set(true);
        result
    });
    called.get()
}

#[test]
fn fetches_first_time_then_every_five_minutes() {
    let mut st = FetchState::default();
    assert!(tick(&mut st, T0, false, Ok(usage(10.0))));
    let l = st.limits.clone().unwrap();
    assert_eq!(l.fetched_at, Some(T0));
    assert_eq!(l.error, None);
    assert!(!tick(&mut st, T0 + MIN, false, Ok(usage(11.0))));
    assert!(!tick(&mut st, T0 + 5 * MIN - 1, false, Ok(usage(11.0))));
    assert!(tick(&mut st, T0 + 5 * MIN, false, Ok(usage(12.0))));
    assert_eq!(st.limits.unwrap().usage.unwrap().windows[0].percent, 12.0);
}

#[test]
fn manual_refresh_honours_the_minimum_gap() {
    let mut st = FetchState::default();
    tick(&mut st, T0, false, Ok(usage(10.0)));
    assert!(!tick(&mut st, T0 + MIN - 1, true, Ok(usage(11.0))));
    assert!(tick(&mut st, T0 + MIN, true, Ok(usage(11.0))));
}

#[test]
fn failure_keeps_last_good_reading() {
    let mut st = FetchState::default();
    tick(&mut st, T0, false, Ok(usage(42.0)));
    tick(
        &mut st,
        T0 + 5 * MIN,
        false,
        Err(LimitsError::Network("timeout".into())),
    );
    let l = st.limits.clone().unwrap();
    assert_eq!(l.usage.unwrap().windows[0].percent, 42.0, "shown dimmed");
    assert_eq!(l.fetched_at, Some(T0), "age of the kept reading");
    assert_eq!(l.error, Some(LimitsError::Network("timeout".into())));
    assert_eq!(st.hold_until, 0, "only a 429 holds");
    // The next success clears the error.
    tick(&mut st, T0 + 10 * MIN, false, Ok(usage(43.0)));
    assert_eq!(st.limits.unwrap().error, None);
}

#[test]
fn failure_with_no_previous_reading_has_no_usage() {
    let mut st = FetchState::default();
    tick(&mut st, T0, false, Err(LimitsError::NotSignedIn));
    let l = st.limits.unwrap();
    assert_eq!(l.usage, None);
    assert_eq!(l.fetched_at, None);
    assert_eq!(l.error, Some(LimitsError::NotSignedIn));
}

#[test]
fn rate_limit_backs_off_doubling_to_an_hour_and_resets_on_success() {
    let mut st = FetchState::default();
    let mut now = T0;
    let mut holds = Vec::new();
    for _ in 0..5 {
        assert!(tick(
            &mut st,
            now,
            false,
            Err(LimitsError::RateLimited(None))
        ));
        holds.push((st.hold_until - now) / MIN);
        // Held: neither the schedule nor a manual refresh calls it.
        let held = st.hold_until - 1;
        assert!(!tick(&mut st, held, true, Ok(usage(1.0))));
        now = st.hold_until;
    }
    assert_eq!(holds, [10, 20, 40, 60, 60]);
    assert!(tick(&mut st, now, false, Ok(usage(5.0))));
    assert_eq!(st.backoff, 0);
    assert!(tick(
        &mut st,
        now + 5 * MIN,
        false,
        Err(LimitsError::RateLimited(None))
    ));
    assert_eq!(
        st.hold_until - (now + 5 * MIN),
        10 * MIN,
        "backoff starts over"
    );
}

#[test]
fn retry_after_longer_than_backoff_wins() {
    let mut st = FetchState::default();
    tick(
        &mut st,
        T0,
        false,
        Err(LimitsError::RateLimited(Some(7200))),
    );
    assert_eq!(st.hold_until, T0 + 7_200_000);
    let mut st = FetchState::default();
    tick(&mut st, T0, false, Err(LimitsError::RateLimited(Some(30))));
    assert_eq!(
        st.hold_until,
        T0 + 10 * MIN,
        "short Retry-After uses the backoff"
    );
}

fn limits_with(provider: Provider, windows: Vec<LimitWindow>) -> ProviderLimits {
    ProviderLimits {
        provider,
        usage: Some(PlanUsage {
            provider,
            plan: String::new(),
            windows,
        }),
        error: None,
        fetched_at: Some(T0),
    }
}

#[test]
fn headline_is_the_fullest_session_or_weekly_window() {
    let mut snap = empty_snapshot(T0);
    assert!(snap.headline().is_none());
    snap.claude = limits_with(
        Provider::Claude,
        vec![
            window(WindowKind::Session, 30.0),
            window(WindowKind::Weekly, 55.0),
            window(WindowKind::Other, 99.0),
        ],
    );
    let (p, w) = snap.headline().unwrap();
    assert_eq!((p.provider, w.kind), (Provider::Claude, WindowKind::Weekly));
    snap.codex = Some(limits_with(
        Provider::Codex,
        vec![window(WindowKind::Session, 80.0)],
    ));
    let (p, w) = snap.headline().unwrap();
    assert_eq!((p.provider, w.percent), (Provider::Codex, 80.0));
    // A provider with only an error (no reading) is skipped.
    snap.codex.as_mut().unwrap().usage = None;
    assert_eq!(snap.headline().unwrap().0.provider, Provider::Claude);
}

#[test]
fn an_account_without_a_session_window_heads_with_its_weekly_one() {
    // Ultra-style reading: no 5-hour window at all.
    let mut snap = empty_snapshot(T0);
    snap.claude = limits_with(
        Provider::Claude,
        vec![
            window(WindowKind::Weekly, 6.0),
            window(WindowKind::Other, 90.0),
        ],
    );
    let (p, w) = snap.headline().unwrap();
    assert_eq!(
        (p.provider, w.kind, w.percent),
        (Provider::Claude, WindowKind::Weekly, 6.0)
    );
    // No session window, so no burn-rate sample is taken from it.
    let mut rate = RateTracker::default();
    let sampled = snap
        .claude
        .usage
        .as_ref()
        .and_then(|u| u.windows.iter().find(|w| w.kind == WindowKind::Session))
        .and_then(|w| rate.sample(w, T0));
    assert_eq!(sampled, None);
}

#[test]
fn history_views_cover_the_last_thirty_days() {
    use history::tests::{at, utc};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("projects/-w");
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(
        p.join("s.jsonl"),
        format!(
            "{}\n{}\n",
            at("2026-09-01T10:00:00.000Z", "old"),
            at("2026-10-30T21:00:00.000Z", "new")
        ),
    )
    .unwrap();
    let mut h = history::UsageHistory::default();
    h.refresh(&[dir.path().join("projects")], &utc);
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
    let snap = summarize(&h, today);
    assert_eq!(snap.projects.len(), 1);
    assert_eq!(
        snap.projects[0].1.messages, 1,
        "September is outside the window"
    );
    assert_eq!(snap.hours[21], snap.projects[0].1.tokens());
    assert_eq!(snap.hours.iter().sum::<u64>(), snap.projects[0].1.tokens());
}

#[test]
fn history_periods_week_month_all_time_and_weekdays() {
    use history::tests::{at, utc};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("projects/-w");
    std::fs::create_dir_all(&p).unwrap();
    // 2026-11-05 is a Thursday: Monday is 11-02, the month starts 11-01.
    let lines = [
        at("2026-10-31T10:00:00.000Z", "oct"),
        at("2026-11-01T10:00:00.000Z", "sun"),
        at("2026-11-02T10:00:00.000Z", "mon"),
        at("2026-11-05T10:00:00.000Z", "thu"),
        at("2026-11-05T11:00:00.000Z", "thu2"),
    ];
    std::fs::write(p.join("s.jsonl"), lines.join("\n") + "\n").unwrap();
    let mut h = history::UsageHistory::default();
    h.refresh(&[dir.path().join("projects")], &utc);
    let snap = summarize(&h, chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap());
    let per = snap.today.tokens() / 2;
    assert_eq!(
        (
            snap.today.messages,
            snap.week.messages,
            snap.month.messages,
            snap.all.messages
        ),
        (2, 3, 4, 5)
    );
    assert_eq!(snap.weekdays, vec![per, 0, 0, 2 * per, 0, 0, 0]);
    assert_eq!(snap.all_sessions, 1);
    let cost = snap.today.cost(&pricing::test_table()).unwrap();
    assert!(cost > 0.0);
}
