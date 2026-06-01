//! Screen-time tracking service — a port of the noctalia v5
//! `ScreenTimeService` idea onto margo's shell.
//!
//! It subscribes to the compositor's authoritative `focused_client`
//! reactive (mirrored by `mshell-margo-client`) and accumulates the
//! wall-clock time each application holds keyboard focus into
//! per-day, per-app buckets. The buckets persist to a small JSON file
//! under `$XDG_DATA_HOME/mshell/screentime.json` so the numbers
//! survive a shell restart.
//!
//! Everything runs on the shared services tokio runtime; the GTK side
//! only ever calls [`ScreenTimeService::snapshot`], which is a cheap
//! locked read that folds the in-progress focus session in live so
//! "today" keeps ticking even between flushes.

use futures::StreamExt;
use mshell_margo_client::MargoService;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::warn;

/// How often the background task flushes the in-progress session into
/// the day buckets (and persists to disk). Focus changes flush
/// immediately regardless; this just bounds drift for long sessions on
/// a single window.
const FLUSH_INTERVAL: Duration = Duration::from_secs(15);

/// Days of history to keep. Older day buckets are pruned on load.
const RETENTION_DAYS: i64 = 30;

/// One application's accumulated focus time within a day.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AppRecord {
    /// Human-readable name (window class as reported by the compositor).
    display: String,
    /// Total focused seconds.
    secs: u64,
}

/// Persisted state: `day_key (YYYY-MM-DD) -> app_key -> record`.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct Persisted {
    days: HashMap<String, HashMap<String, AppRecord>>,
}

struct Inner {
    persisted: Persisted,
    /// `(app_key, display)` of the window currently holding focus.
    current: Option<(String, String)>,
    /// When the current session started accruing (monotonic).
    since: Instant,
}

/// Aggregated, GTK-facing view of screen time over a window of days.
#[derive(Debug, Clone, Default)]
pub struct ScreenTimeSnapshot {
    /// Sum of all app time across the range, in seconds.
    pub total_secs: u64,
    /// Per-app totals, sorted descending by time.
    pub apps: Vec<ScreenTimeApp>,
}

#[derive(Debug, Clone)]
pub struct ScreenTimeApp {
    pub display: String,
    pub secs: u64,
}

pub struct ScreenTimeService {
    inner: Arc<Mutex<Inner>>,
}

impl ScreenTimeService {
    /// Load persisted history, start the background focus watcher on the
    /// services runtime, and return the live service handle.
    pub fn new_started(margo: Arc<MargoService>) -> Arc<Self> {
        let mut persisted = load_persisted();
        prune_old(&mut persisted);

        let inner = Arc::new(Mutex::new(Inner {
            persisted,
            current: None,
            since: Instant::now(),
        }));

        let service = Arc::new(ScreenTimeService {
            inner: inner.clone(),
        });

        spawn_watcher(inner, margo);
        service
    }

    /// Aggregate the last `range_days` days (1 = today only). The
    /// in-progress focus session is folded in so today stays live.
    pub fn snapshot(&self, range_days: i64) -> ScreenTimeSnapshot {
        let keys = recent_day_keys(range_days.max(1));
        let guard = self.inner.lock().expect("screen-time lock");

        // Fold persisted buckets.
        let mut totals: HashMap<String, AppRecord> = HashMap::new();
        for key in &keys {
            if let Some(day) = guard.persisted.days.get(key) {
                for (app_key, rec) in day {
                    let entry = totals.entry(app_key.clone()).or_insert_with(|| AppRecord {
                        display: rec.display.clone(),
                        secs: 0,
                    });
                    entry.secs += rec.secs;
                }
            }
        }

        // Fold the live in-progress session (always lands in today, which
        // is keys[0] for any range that includes today).
        if let Some((app_key, display)) = &guard.current {
            let pending = guard.since.elapsed().as_secs();
            if pending > 0 {
                let entry = totals.entry(app_key.clone()).or_insert_with(|| AppRecord {
                    display: display.clone(),
                    secs: 0,
                });
                entry.secs += pending;
            }
        }
        drop(guard);

        let mut apps: Vec<ScreenTimeApp> = totals
            .into_values()
            .map(|r| ScreenTimeApp {
                display: r.display,
                secs: r.secs,
            })
            .collect();
        apps.sort_by_key(|a| std::cmp::Reverse(a.secs));
        let total_secs = apps.iter().map(|a| a.secs).sum();

        ScreenTimeSnapshot { total_secs, apps }
    }
}

/// Spawn the focus watcher: prime from the current focus, then react to
/// every `focused_client` change and a periodic flush tick.
fn spawn_watcher(inner: Arc<Mutex<Inner>>, margo: Arc<MargoService>) {
    crate::tokio_rt_spawn(async move {
        // Prime with whatever is focused right now.
        let initial = margo.focused_client.get();
        switch_focus(&inner, initial.and_then(app_identity));

        let mut stream = margo.focused_client.watch();
        let mut tick = tokio::time::interval(FLUSH_INTERVAL);
        // Skip the immediate first tick.
        tick.tick().await;

        loop {
            tokio::select! {
                next = stream.next() => {
                    match next {
                        Some(client) => switch_focus(&inner, client.and_then(app_identity)),
                        None => break,
                    }
                }
                _ = tick.tick() => {
                    flush(&inner, false);
                }
            }
        }
    });
}

/// Map a focused client to its `(app_key, display)`, or `None` when the
/// class is empty (nothing meaningful to attribute time to).
fn app_identity(client: Arc<mshell_margo_client::Client>) -> Option<(String, String)> {
    let class = client.class.get();
    let display = if class.is_empty() {
        client.initial_class.get()
    } else {
        class
    };
    let display = display.trim().to_string();
    if display.is_empty() {
        return None;
    }
    let key = display.to_lowercase();
    Some((key, display))
}

/// Flush the in-progress session into the day bucket and reset the
/// session clock. Called on every focus change (with the *old* current)
/// and on the periodic tick.
fn flush(inner: &Arc<Mutex<Inner>>, _changing: bool) {
    let mut guard = inner.lock().expect("screen-time lock");
    let elapsed = guard.since.elapsed().as_secs();
    guard.since = Instant::now();
    if elapsed == 0 {
        return;
    }
    if let Some((app_key, display)) = guard.current.clone() {
        let day_key = today_key();
        let day = guard.persisted.days.entry(day_key).or_default();
        let rec = day.entry(app_key).or_insert_with(|| AppRecord {
            display: display.clone(),
            secs: 0,
        });
        rec.display = display;
        rec.secs += elapsed;
        let snapshot = guard.persisted.clone();
        drop(guard);
        save_persisted(&snapshot);
    }
}

/// Attribute pending time to the outgoing app, then switch the current
/// focus to `next`.
fn switch_focus(inner: &Arc<Mutex<Inner>>, next: Option<(String, String)>) {
    flush(inner, true);
    let mut guard = inner.lock().expect("screen-time lock");
    guard.current = next;
    guard.since = Instant::now();
}

// ── Day-key helpers (local time) ────────────────────────────────────

fn today_key() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// The `range_days` most-recent local day keys, newest first.
fn recent_day_keys(range_days: i64) -> Vec<String> {
    let today = chrono::Local::now().date_naive();
    (0..range_days)
        .filter_map(|d| today.checked_sub_days(chrono::Days::new(d as u64)))
        .map(|date| date.format("%Y-%m-%d").to_string())
        .collect()
}

fn prune_old(persisted: &mut Persisted) {
    let cutoff = chrono::Local::now()
        .date_naive()
        .checked_sub_days(chrono::Days::new(RETENTION_DAYS as u64));
    let Some(cutoff) = cutoff else { return };
    let cutoff_key = cutoff.format("%Y-%m-%d").to_string();
    persisted
        .days
        .retain(|k, _| k.as_str() >= cutoff_key.as_str());
}

// ── Persistence ─────────────────────────────────────────────────────

fn data_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".local/share")
        });
    base.join("mshell").join("screentime.json")
}

fn load_persisted() -> Persisted {
    let path = data_path();
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            warn!("screen-time: parse {path:?} failed: {e}; starting fresh");
            Persisted::default()
        }),
        Err(_) => Persisted::default(),
    }
}

fn save_persisted(persisted: &Persisted) {
    let path = data_path();
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        warn!("screen-time: mkdir {dir:?} failed: {e}");
        return;
    }
    match serde_json::to_vec(persisted) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&path, bytes) {
                warn!("screen-time: write {path:?} failed: {e}");
            }
        }
        Err(e) => warn!("screen-time: serialize failed: {e}"),
    }
}
