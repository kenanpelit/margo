//! Drive issues and how they change: which issues a reading shows, and
//! which of them are new, worse or cleared compared to the persisted
//! state — the decisions behind the drive-health notifications. Pure.
//!
//! Two sources own different issue kinds: the minute sysfs scan
//! (temperature, a drive gone missing) and a full SMART read (everything
//! else). A reading only ever settles its own source's kinds, so a SMART
//! issue stays active between reads instead of "clearing" at every scan.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::menus::menu_widgets::cpu_dashboard::drives::{Limits, SmartReport};

/// One kind of drive issue. The order is the order notifications list them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Missing,
    SmartFailed,
    CriticalWarning,
    MediaErrors,
    SpareLow,
    BadSectors,
    Wear,
    Temperature,
}

impl Kind {
    /// Kinds a full SMART read decides.
    pub(crate) const SMART: &[Kind] = &[
        Kind::SmartFailed,
        Kind::CriticalWarning,
        Kind::MediaErrors,
        Kind::SpareLow,
        Kind::BadSectors,
        Kind::Wear,
    ];
    /// Kinds the sysfs temperature reading decides.
    pub(crate) const TEMPERATURE: &[Kind] = &[Kind::Temperature];

    /// Short name for "… is back to normal".
    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Missing => "Drive detection",
            Kind::SmartFailed => "SMART self-assessment",
            Kind::CriticalWarning => "Critical-warning flags",
            Kind::MediaErrors => "Media errors",
            Kind::SpareLow => "Spare capacity",
            Kind::BadSectors => "Bad sectors",
            Kind::Wear => "Wear",
            Kind::Temperature => "Temperature",
        }
    }

    /// Whether `new` is worse than `old` for this kind (`level`s only grow
    /// with severity; critical-warning levels are bit sets).
    fn worse(self, old: u64, new: u64) -> bool {
        match self {
            Kind::CriticalWarning => new & !old != 0,
            _ => new > old,
        }
    }
}

/// One issue in a reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Issue {
    pub kind: Kind,
    /// Severity measure; larger = worse (a count, a bucket, a level).
    pub level: u64,
    pub text: String,
    /// Raise the notification as critical.
    pub critical: bool,
}

/// How far below a threshold the temperature must fall before its issue
/// clears or steps down (no flapping around the line).
pub(crate) const TEMP_HYSTERESIS_C: i64 = 3;

/// The temperature issue for `temp_c`, given the level active before
/// (1 = hot, 2 = critical) — with hysteresis on the way down.
pub(crate) fn temperature_issue(temp_c: i64, active: Option<u64>, l: &Limits) -> Option<Issue> {
    let raw = if temp_c >= l.temp_crit_c {
        2
    } else if temp_c >= l.temp_warn_c {
        1
    } else {
        0
    };
    let threshold = |lvl: u64| {
        if lvl >= 2 {
            l.temp_crit_c
        } else {
            l.temp_warn_c
        }
    };
    let level = match active {
        Some(prev) if prev > raw && temp_c > threshold(prev) - TEMP_HYSTERESIS_C => prev,
        Some(prev) if prev > raw && raw == 0 && temp_c > l.temp_warn_c - TEMP_HYSTERESIS_C => 1,
        _ => raw,
    };
    (level > 0).then(|| Issue {
        kind: Kind::Temperature,
        level,
        text: if level >= 2 {
            format!("critically hot: {temp_c}°C (limit {}°C)", l.temp_crit_c)
        } else {
            format!("running hot: {temp_c}°C (limit {}°C)", l.temp_warn_c)
        },
        critical: level >= 2,
    })
}

/// The issues a full SMART read shows.
pub(crate) fn smart_issues(r: &SmartReport, l: &Limits) -> Vec<Issue> {
    let mut out = Vec::new();
    let mut push = |kind, level, text: String, critical| {
        out.push(Issue {
            kind,
            level,
            text,
            critical,
        })
    };
    if r.passed == Some(false) {
        push(
            Kind::SmartFailed,
            1,
            "SMART self-assessment FAILED".into(),
            true,
        );
    }
    if let Some(c) = r.critical_warning.filter(|c| *c != 0) {
        push(
            Kind::CriticalWarning,
            c,
            format!("critical warning flags 0x{c:02x}"),
            true,
        );
    }
    if let Some(m) = r.media_errors.filter(|m| *m > 0) {
        push(Kind::MediaErrors, m, format!("{m} media errors"), true);
    }
    if let (Some(s), Some(t)) = (r.spare_pct, r.spare_threshold_pct)
        && s < t
    {
        push(
            Kind::SpareLow,
            100u64.saturating_sub(s),
            format!("spare {s}% below threshold {t}%"),
            true,
        );
    }
    if let Some(b) = r.bad_sectors.filter(|b| *b > 0) {
        push(Kind::BadSectors, b, format!("{b} bad sectors"), false);
    }
    if let Some(w) = r.wear_pct.filter(|w| *w >= l.wear_warn_pct) {
        // Every further 5 points counts as worse, not every point.
        push(
            Kind::Wear,
            w / 5 * 5,
            format!("{w}% of rated endurance used"),
            false,
        );
    }
    out
}

/// What changed for one drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    New(Issue),
    Worse(Issue),
    Cleared(Kind),
}

/// Settle `scope`'s kinds in `active` (kind → level) against `issues` and
/// return what changed. Kinds outside `scope` are left alone.
pub(crate) fn apply(
    active: &mut BTreeMap<Kind, u64>,
    scope: &[Kind],
    issues: Vec<Issue>,
) -> Vec<Change> {
    let mut changes = Vec::new();
    for kind in scope {
        if active.contains_key(kind) && !issues.iter().any(|i| i.kind == *kind) {
            active.remove(kind);
            changes.push(Change::Cleared(*kind));
        }
    }
    for issue in issues.into_iter().filter(|i| scope.contains(&i.kind)) {
        match active.insert(issue.kind, issue.level) {
            None => changes.push(Change::New(issue)),
            Some(old) if issue.kind.worse(old, issue.level) => changes.push(Change::Worse(issue)),
            Some(_) => {}
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l() -> Limits {
        Limits {
            temp_warn_c: 70,
            temp_crit_c: 85,
            wear_warn_pct: 80,
        }
    }

    #[test]
    fn temperature_steps_up_and_down_with_hysteresis() {
        let l = l();
        assert_eq!(temperature_issue(60, None, &l), None);
        assert_eq!(temperature_issue(70, None, &l).unwrap().level, 1);
        let hot = temperature_issue(86, Some(1), &l).unwrap();
        assert_eq!((hot.level, hot.critical), (2, true));
        // 83 °C after critical: within 3 °C of 85, stays critical.
        assert_eq!(temperature_issue(83, Some(2), &l).unwrap().level, 2);
        // 82 °C: steps down to hot.
        assert_eq!(temperature_issue(82, Some(2), &l).unwrap().level, 1);
        // 68 °C after hot: still within the band, stays hot; 67 clears.
        assert_eq!(temperature_issue(68, Some(1), &l).unwrap().level, 1);
        assert_eq!(temperature_issue(67, Some(1), &l), None);
        // Straight from critical to 68 °C: hot, not cleared yet.
        assert_eq!(temperature_issue(68, Some(2), &l).unwrap().level, 1);
    }

    fn report() -> SmartReport {
        SmartReport {
            dev: "/dev/nvme0".into(),
            passed: Some(true),
            wear_pct: Some(3),
            spare_pct: Some(100),
            spare_threshold_pct: Some(10),
            critical_warning: Some(0),
            media_errors: Some(0),
            ..SmartReport::default()
        }
    }

    #[test]
    fn a_healthy_report_has_no_issues() {
        assert!(smart_issues(&report(), &l()).is_empty());
    }

    #[test]
    fn smart_issues_new_worse_and_cleared() {
        let l = l();
        let mut active = BTreeMap::new();
        // Temperature is active and must survive SMART-only settling.
        active.insert(Kind::Temperature, 1);

        let mut r = report();
        r.media_errors = Some(2);
        r.wear_pct = Some(81);
        let ch = apply(&mut active, Kind::SMART, smart_issues(&r, &l));
        assert_eq!(ch.len(), 2);
        assert!(matches!(&ch[0], Change::New(i) if i.kind == Kind::MediaErrors && i.critical));
        assert!(matches!(&ch[1], Change::New(i) if i.kind == Kind::Wear && !i.critical));

        // Same reading again: nothing new.
        assert!(apply(&mut active, Kind::SMART, smart_issues(&r, &l)).is_empty());
        // 84 % is the same 5-point bucket; 85 % and more errors are worse.
        r.wear_pct = Some(84);
        assert!(apply(&mut active, Kind::SMART, smart_issues(&r, &l)).is_empty());
        r.wear_pct = Some(85);
        r.media_errors = Some(3);
        let ch = apply(&mut active, Kind::SMART, smart_issues(&r, &l));
        assert_eq!(ch.len(), 2);
        assert!(ch.iter().all(|c| matches!(c, Change::Worse(_))));

        // A new critical-warning bit is worse; the same bits are not.
        r.critical_warning = Some(0x04);
        apply(&mut active, Kind::SMART, smart_issues(&r, &l));
        r.critical_warning = Some(0x04);
        assert!(apply(&mut active, Kind::SMART, smart_issues(&r, &l)).is_empty());
        r.critical_warning = Some(0x01);
        let ch = apply(&mut active, Kind::SMART, smart_issues(&r, &l));
        assert!(matches!(&ch[..], [Change::Worse(i)] if i.kind == Kind::CriticalWarning));

        // Healthy again: every SMART issue clears, temperature stays.
        let ch = apply(&mut active, Kind::SMART, smart_issues(&report(), &l));
        assert_eq!(ch.len(), 3);
        assert!(ch.iter().all(|c| matches!(c, Change::Cleared(_))));
        assert_eq!(
            active.into_iter().collect::<Vec<_>>(),
            [(Kind::Temperature, 1)]
        );
    }

    #[test]
    fn failed_assessment_spare_and_bad_sectors() {
        let mut r = report();
        r.passed = Some(false);
        r.spare_pct = Some(5);
        r.bad_sectors = Some(8);
        let kinds: Vec<Kind> = smart_issues(&r, &l()).iter().map(|i| i.kind).collect();
        assert_eq!(kinds, [Kind::SmartFailed, Kind::SpareLow, Kind::BadSectors]);
    }

    #[test]
    fn temperature_settling_leaves_smart_issues() {
        let mut active = BTreeMap::new();
        active.insert(Kind::MediaErrors, 2);
        let issue = temperature_issue(90, None, &l());
        let ch = apply(&mut active, Kind::TEMPERATURE, issue.into_iter().collect());
        assert!(matches!(&ch[..], [Change::New(i)] if i.kind == Kind::Temperature));
        let ch = apply(&mut active, Kind::TEMPERATURE, Vec::new());
        assert_eq!(ch, [Change::Cleared(Kind::Temperature)]);
        assert_eq!(active.get(&Kind::MediaErrors), Some(&2));
    }
}
