//! Background drive-health monitoring (`drive_health.alerts` /
//! `drive_health.history`), behind the CPU dashboard's Drives tab.
//!
//! * Every minute while either is on, [`scan`] reads sysfs (no root):
//!   which internal drives are attached and how hot they
//!   are. [`settle_scan`] turns that into temperature issues (with
//!   hysteresis) and a missing-drive alert once an established drive has
//!   been absent for `missing_grace_scans` scans; [`history`] keeps a
//!   temperature sample per `history_interval_minutes`.
//! * Full SMART needs root, so it is never polled: every full read the
//!   Drives tab makes (its "Read SMART health…" button, or the progress
//!   reads of a self-test) is handed to [`smart_read`], which settles the
//!   SMART issues (wear, spare, media errors, bad sectors, critical
//!   warnings, a failed assessment) against the last persisted read and
//!   records a history sample.
//! * New and worsening issues raise a desktop notification; with
//!   `notify_recovery` a cleared one does too. State and history live in
//!   `$XDG_STATE_HOME/mshell/drive-health/` (written only when something
//!   changed, never replaced while unparseable).

pub(crate) mod alerts;
pub(crate) mod history;
pub(crate) mod scan;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use alerts::{Change, Issue, Kind};
use history::History;
use mshell_config::schema::config::DriveHealthConfig;
use relm4::gtk::glib;
use scan::Seen;
use serde::{Deserialize, Serialize};

use crate::menus::menu_widgets::cpu_dashboard::drives::{Limits, SmartReport};

/// How often the sysfs scan runs while monitoring is on.
const SCAN_INTERVAL: Duration = Duration::from_secs(60);
/// Scans a drive must have been seen in before its disappearance alerts
/// (a drive that showed up once and left is not "established").
const ESTABLISHED_SCANS: u32 = 3;
/// A drive absent this long is forgotten (its alert state dropped).
const FORGET_SECS: u64 = 30 * 86_400;

/// Persisted alert state of one drive.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct DriveState {
    #[serde(default)]
    pub dev: String,
    #[serde(default)]
    pub model: String,
    /// Scans seen in, counted up to [`ESTABLISHED_SCANS`].
    #[serde(default)]
    pub seen_scans: u32,
    #[serde(default)]
    pub absent_scans: u32,
    /// Unix seconds.
    #[serde(default)]
    pub last_seen: u64,
    /// Active issues: kind → level.
    #[serde(default)]
    pub active: BTreeMap<Kind, u64>,
}

impl DriveState {
    fn name(&self) -> &str {
        if self.model.is_empty() {
            &self.dev
        } else {
            &self.model
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct AlertState {
    #[serde(default)]
    pub drives: BTreeMap<String, DriveState>,
}

/// The `drive_health.*` settings one pass needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Settings {
    pub alerts: bool,
    pub notify_recovery: bool,
    pub missing_alerts: bool,
    pub grace_scans: u32,
    pub limits: Limits,
    pub history: bool,
    pub interval_secs: u64,
    pub retention_secs: u64,
}

impl Settings {
    pub(crate) fn from_config(c: &DriveHealthConfig) -> Self {
        Self {
            alerts: c.alerts,
            notify_recovery: c.notify_recovery,
            missing_alerts: c.missing_alerts,
            grace_scans: c.grace_scans(),
            limits: Limits::from_config(c),
            history: c.history,
            interval_secs: c.history_interval_secs(),
            retention_secs: c.retention_secs(),
        }
    }

    fn current() -> Self {
        use mshell_config::config_manager::config_manager;
        use mshell_config::schema::config::ConfigStoreFields;
        use reactive_graph::prelude::GetUntracked;
        Self::from_config(&config_manager().config().drive_health().get_untracked())
    }

    fn monitoring(&self) -> bool {
        self.alerts || self.history
    }
}

/// One change worth telling the user about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Event {
    pub name: String,
    pub dev: String,
    pub change: Change,
}

/// Settle one sysfs scan: presence (missing-drive alerts) and temperature.
/// Returns the events and whether the state changed enough to save
/// (per-scan bookkeeping alone is not worth a disk write every minute).
pub(crate) fn settle_scan(
    state: &mut AlertState,
    seen: &[Seen],
    now: u64,
    s: &Settings,
) -> (Vec<Event>, bool) {
    let mut events = Vec::new();
    let mut changed = false;
    for d in seen {
        let st = state.drives.entry(d.id.clone()).or_insert_with(|| {
            changed = true;
            DriveState::default()
        });
        if st.dev != d.dev || st.model != d.model {
            st.dev = d.dev.clone();
            st.model = d.model.clone();
            changed = true;
        }
        if st.seen_scans < ESTABLISHED_SCANS {
            st.seen_scans += 1;
            changed |= st.seen_scans == ESTABLISHED_SCANS;
        }
        st.absent_scans = 0;
        st.last_seen = now;
        let mut ch = Vec::new();
        if st.active.remove(&Kind::Missing).is_some() {
            ch.push(Change::Cleared(Kind::Missing));
        }
        if let Some(t) = d.temp_c {
            let issue =
                alerts::temperature_issue(t, st.active.get(&Kind::Temperature).copied(), &s.limits);
            let before = st.active.get(&Kind::Temperature).copied();
            ch.extend(alerts::apply(
                &mut st.active,
                Kind::TEMPERATURE,
                issue.into_iter().collect(),
            ));
            changed |= before != st.active.get(&Kind::Temperature).copied();
        }
        changed |= !ch.is_empty();
        events.extend(ch.into_iter().map(|change| Event {
            name: st.name().to_string(),
            dev: st.dev.clone(),
            change,
        }));
    }
    let present: Vec<&str> = seen.iter().map(|d| d.id.as_str()).collect();
    state.drives.retain(|id, st| {
        if present.contains(&id.as_str()) {
            return true;
        }
        if st.seen_scans < ESTABLISHED_SCANS {
            changed = true;
            return false;
        }
        st.absent_scans = st.absent_scans.saturating_add(1);
        // No sensor while gone: the temperature issue just ends.
        changed |= st.active.remove(&Kind::Temperature).is_some();
        if s.missing_alerts
            && st.absent_scans >= s.grace_scans
            && !st.active.contains_key(&Kind::Missing)
        {
            st.active.insert(Kind::Missing, 1);
            changed = true;
            events.push(Event {
                name: st.name().to_string(),
                dev: st.dev.clone(),
                change: Change::New(Issue {
                    kind: Kind::Missing,
                    level: 1,
                    text: "is no longer detected".to_string(),
                    critical: true,
                }),
            });
        }
        let keep = now.saturating_sub(st.last_seen) < FORGET_SECS;
        changed |= !keep;
        keep
    });
    (events, changed)
}

/// Settle one full SMART read of drive `id`.
pub(crate) fn settle_smart(
    state: &mut AlertState,
    id: &str,
    r: &SmartReport,
    limits: &Limits,
) -> Vec<Event> {
    let st = state.drives.entry(id.to_string()).or_default();
    if st.dev.is_empty() {
        st.dev = r.dev.clone();
    }
    if st.model.is_empty() {
        st.model = r.model.clone();
    }
    let ch = alerts::apply(&mut st.active, Kind::SMART, alerts::smart_issues(r, limits));
    ch.into_iter()
        .map(|change| Event {
            name: st.name().to_string(),
            dev: st.dev.clone(),
            change,
        })
        .collect()
}

/// `(summary, body, critical)` of the notification for `e`; `None` for a
/// recovery when `notify_recovery` is off.
pub(crate) fn notice_text(e: &Event, notify_recovery: bool) -> Option<(String, String, bool)> {
    Some(match &e.change {
        Change::New(i) if i.kind == Kind::Missing => (
            format!("Drive missing: {}", e.name),
            format!("{} ({}) {}", e.name, e.dev, i.text),
            true,
        ),
        Change::New(i) => (
            format!("Drive needs attention: {}", e.name),
            format!("{} · {}", i.text, e.dev),
            i.critical,
        ),
        Change::Worse(i) => (
            format!("Drive getting worse: {}", e.name),
            format!("{} · {}", i.text, e.dev),
            i.critical,
        ),
        Change::Cleared(_) if !notify_recovery => return None,
        Change::Cleared(Kind::Missing) => (
            format!("Drive back: {}", e.name),
            format!("{} is detected again", e.dev),
            false,
        ),
        Change::Cleared(k) => (
            format!("Drive recovered: {}", e.name),
            format!("{} is back to normal · {}", k.label(), e.dev),
            false,
        ),
    })
}

fn announce(events: &[Event], notify_recovery: bool) {
    use mshell_process::desktop::{Notice, Urgency, notify};
    for e in events {
        if let Some((summary, body, critical)) = notice_text(e, notify_recovery) {
            notify(
                Notice::new("Drive health", summary, body)
                    .icon("drive-harddisk-symbolic")
                    .urgency(if critical {
                        Urgency::Critical
                    } else {
                        Urgency::Normal
                    }),
            );
        }
    }
}

// ── Store ───────────────────────────────────────────────────────────────

/// In-memory state + history, loaded on first use, shared by the scan
/// (blocking pool), the Drives tab and SMART reads.
struct Store {
    alerts: AlertState,
    history: History,
    /// A file that didn't parse is never written over (side_file rule).
    alerts_broken: bool,
    history_broken: bool,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn dir() -> PathBuf {
    margo_fs::xdg::state_home()
        .join("mshell")
        .join("drive-health")
}

fn load<T: serde::de::DeserializeOwned + Default>(path: &Path) -> (T, bool) {
    use margo_fs::side_file::SideLoad;
    match margo_fs::side_file::load(path, |raw| serde_json::from_str::<T>(raw)) {
        SideLoad::Loaded(v) => (v, false),
        SideLoad::Missing => (T::default(), false),
        SideLoad::Broken { error, .. } => {
            tracing::warn!(
                "drive-health: {} does not parse ({error}); not using it",
                path.display()
            );
            (T::default(), true)
        }
    }
}

fn save<T: Serialize>(path: &Path, value: &T) {
    let Ok(body) = serde_json::to_vec(value) else {
        return;
    };
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = margo_fs::side_file::save(path, &body, |raw| {
        serde_json::from_str::<serde_json::Value>(raw).map(|_| ())
    }) {
        tracing::warn!("drive-health: {e}");
    }
}

fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut guard = STORE.lock().unwrap_or_else(|p| p.into_inner());
    let store = guard.get_or_insert_with(|| {
        let d = dir();
        let (alerts, alerts_broken) = load(&d.join("alerts.json"));
        let (history, history_broken) = load(&d.join("history.json"));
        Store {
            alerts,
            history,
            alerts_broken,
            history_broken,
        }
    });
    f(store)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One scan pass (blocking: sysfs reads + maybe a file write).
fn run_scan(s: Settings) {
    let seen = scan::scan(Path::new("/sys"));
    let now = now_secs();
    let events = with_store(|st| {
        let mut events = Vec::new();
        if s.alerts {
            let (ev, changed) = settle_scan(&mut st.alerts, &seen, now, &s);
            events = ev;
            if changed && !st.alerts_broken {
                save(&dir().join("alerts.json"), &st.alerts);
            }
        }
        if s.history {
            let mut added = false;
            for d in &seen {
                if let Some(t) = d.temp_c {
                    added |= st
                        .history
                        .record_temp(&d.id, &d.model, now, t, s.interval_secs);
                }
            }
            if added && !st.history_broken {
                st.history.prune(now, s.retention_secs);
                save(&dir().join("history.json"), &st.history);
            }
        }
        events
    });
    announce(&events, s.notify_recovery);
}

/// Hand a full SMART read from the Drives tab to the monitor: settle its
/// issues and record history (whichever is on). GTK thread; the work runs
/// on the blocking pool.
pub(crate) fn smart_read(reports: Vec<SmartReport>) {
    let s = Settings::current();
    if !s.monitoring() || reports.is_empty() {
        return;
    }
    relm4::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            let seen = scan::scan(Path::new("/sys"));
            let now = now_secs();
            let events = with_store(|st| {
                let mut events = Vec::new();
                for r in &reports {
                    let id = seen
                        .iter()
                        .find(|d| d.dev == r.dev)
                        .map(|d| d.id.clone())
                        .unwrap_or_else(|| scan::drive_id(&r.serial, &r.dev));
                    if s.alerts {
                        events.extend(settle_smart(&mut st.alerts, &id, r, &s.limits));
                    }
                    if s.history {
                        st.history.record_smart(&id, now, r);
                    }
                }
                if s.alerts && !st.alerts_broken {
                    save(&dir().join("alerts.json"), &st.alerts);
                }
                if s.history && !st.history_broken {
                    st.history.prune(now, s.retention_secs);
                    save(&dir().join("history.json"), &st.history);
                }
                events
            });
            announce(&events, s.notify_recovery);
        })
        .await;
    });
}

/// The trend history (blocking: may load the file).
pub(crate) fn history_snapshot() -> History {
    with_store(|st| st.history.clone())
}

thread_local! {
    /// The minute timer while monitoring is on (GTK thread).
    static TIMER: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
    static SCANNING: Cell<bool> = const { Cell::new(false) };
}

/// Start or stop the monitor for `cfg` (idempotent; the config effect
/// calls it on every `drive_health` change). Starting scans at once.
pub(crate) fn configure(cfg: &DriveHealthConfig) {
    let on = Settings::from_config(cfg).monitoring();
    let started = TIMER.with(|t| {
        let mut t = t.borrow_mut();
        match (on, t.is_some()) {
            (true, false) => {
                *t = Some(glib::timeout_add_local(SCAN_INTERVAL, || {
                    tick();
                    glib::ControlFlow::Continue
                }));
                true
            }
            (false, true) => {
                if let Some(id) = t.take() {
                    id.remove();
                }
                false
            }
            _ => false,
        }
    });
    if started {
        tick();
    }
}

/// One scan off the GTK thread; skipped while the previous one runs.
fn tick() {
    if SCANNING.with(|s| s.replace(true)) {
        return;
    }
    let s = Settings::current();
    relm4::spawn_local(async move {
        let _ = relm4::spawn(async move {
            let _ = tokio::task::spawn_blocking(move || run_scan(s)).await;
        })
        .await;
        SCANNING.with(|c| c.set(false));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::from_config(&DriveHealthConfig {
            alerts: true,
            ..DriveHealthConfig::default()
        })
    }

    fn seen(id: &str, temp: Option<i64>) -> Seen {
        Seen {
            id: id.into(),
            dev: format!("/dev/{id}"),
            model: format!("Model {id}"),
            temp_c: temp,
        }
    }

    #[test]
    fn a_drive_goes_missing_only_after_it_was_established_and_the_grace() {
        let s = settings();
        let mut st = AlertState::default();
        // Seen in three scans: established. Quiet: no events.
        for t in 0..3 {
            let (ev, _) = settle_scan(&mut st, &[seen("a", Some(40))], t, &s);
            assert!(ev.is_empty());
        }
        // Absent for grace-1 scans: still quiet.
        for t in 3..5 {
            assert!(settle_scan(&mut st, &[], t, &s).0.is_empty());
        }
        // Third absent scan: missing.
        let (ev, changed) = settle_scan(&mut st, &[], 5, &s);
        assert!(changed);
        assert!(
            matches!(&ev[..], [Event { change: Change::New(i), .. }] if i.kind == Kind::Missing)
        );
        let (summary, body, critical) = notice_text(&ev[0], true).unwrap();
        assert_eq!(summary, "Drive missing: Model a");
        assert_eq!(body, "Model a (/dev/a) is no longer detected");
        assert!(critical);
        // Once only.
        assert!(settle_scan(&mut st, &[], 6, &s).0.is_empty());
        // Back: cleared.
        let (ev, _) = settle_scan(&mut st, &[seen("a", Some(40))], 7, &s);
        assert_eq!(ev[0].change, Change::Cleared(Kind::Missing));
        assert_eq!(notice_text(&ev[0], true).unwrap().0, "Drive back: Model a");
        assert_eq!(notice_text(&ev[0], false), None);
    }

    #[test]
    fn a_passing_drive_is_forgotten_quietly_and_missing_alerts_can_be_off() {
        let s = settings();
        let mut st = AlertState::default();
        settle_scan(&mut st, &[seen("usb", None)], 0, &s);
        let (ev, changed) = settle_scan(&mut st, &[], 1, &s);
        assert!(ev.is_empty() && changed);
        assert!(st.drives.is_empty(), "never established: dropped");

        let off = Settings {
            missing_alerts: false,
            ..s
        };
        for t in 0..3 {
            settle_scan(&mut st, &[seen("a", None)], t, &off);
        }
        for t in 3..20 {
            assert!(settle_scan(&mut st, &[], t, &off).0.is_empty());
        }
        // Absent past the forget window: dropped.
        settle_scan(&mut st, &[], FORGET_SECS + 10, &off);
        assert!(st.drives.is_empty());
    }

    #[test]
    fn temperature_alerts_and_steady_scans_do_not_save() {
        let s = settings();
        let mut st = AlertState::default();
        for t in 0..3 {
            settle_scan(&mut st, &[seen("a", Some(40))], t, &s);
        }
        // Steady and established: nothing to save every minute.
        assert_eq!(
            settle_scan(&mut st, &[seen("a", Some(41))], 4, &s),
            (vec![], false)
        );
        let (ev, changed) = settle_scan(&mut st, &[seen("a", Some(72))], 5, &s);
        assert!(changed);
        let (summary, body, critical) = notice_text(&ev[0], true).unwrap();
        assert_eq!(summary, "Drive needs attention: Model a");
        assert_eq!(body, "running hot: 72°C (limit 70°C) · /dev/a");
        assert!(!critical);
        let (ev, _) = settle_scan(&mut st, &[seen("a", Some(90))], 6, &s);
        assert!(matches!(&ev[0].change, Change::Worse(i) if i.critical));
        // Unreadable sensor: no change either way.
        assert_eq!(settle_scan(&mut st, &[seen("a", None)], 7, &s).0, vec![]);
        let (ev, _) = settle_scan(&mut st, &[seen("a", Some(50))], 8, &s);
        assert_eq!(ev[0].change, Change::Cleared(Kind::Temperature));
        assert_eq!(
            notice_text(&ev[0], true).unwrap().1,
            "Temperature is back to normal · /dev/a"
        );
    }

    #[test]
    fn smart_reads_settle_against_the_last_read() {
        let l = settings().limits;
        let mut st = AlertState::default();
        let mut r = SmartReport {
            dev: "/dev/nvme0".into(),
            model: "Disk".into(),
            media_errors: Some(0),
            ..SmartReport::default()
        };
        assert!(settle_smart(&mut st, "S1", &r, &l).is_empty());
        r.media_errors = Some(1);
        let ev = settle_smart(&mut st, "S1", &r, &l);
        let (summary, body, critical) = notice_text(&ev[0], true).unwrap();
        assert_eq!(summary, "Drive needs attention: Disk");
        assert_eq!(body, "1 media errors · /dev/nvme0");
        assert!(critical);
        r.media_errors = Some(4);
        let ev = settle_smart(&mut st, "S1", &r, &l);
        assert_eq!(
            notice_text(&ev[0], true).unwrap().0,
            "Drive getting worse: Disk"
        );
        // Survives a round trip through the state file.
        let json = serde_json::to_string(&st).unwrap();
        let mut back: AlertState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, st);
        assert!(settle_smart(&mut back, "S1", &r, &l).is_empty());
    }
}
