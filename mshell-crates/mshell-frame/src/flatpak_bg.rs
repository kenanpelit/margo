//! Flatpak Background — Flatpak apps that run without a window.
//!
//! A scan runs `flatpak ps --columns=application,child-pid`, compares the
//! running app ids with margo's windows and publishes the ones that have
//! none. A window belongs to a running app when its app id equals the
//! Flatpak id or its last segment (`org.ferdium.Ferdium` / `ferdium`,
//! case-insensitive), or when the window's process lives in that app's
//! sandbox (`/proc/<pid>/root/.flatpak-info` names it) — the reference
//! plugin's heuristics, minus its niri-specific parts. Apps in
//! `flatpak_bg.ignored_apps` are left out.
//!
//! The task scans every `flatpak_bg.interval_seconds` but only while a pill
//! or the menu holds a receiver; without `flatpak` on `PATH` it publishes
//! `available: false` once and the pill hides. Stop runs `flatpak kill`.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, FlatpakBgConfig};
use reactive_graph::prelude::GetUntracked;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BgApp {
    pub(crate) id: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FlatpakBgState {
    /// `flatpak` is on `PATH` (false: the pill hides).
    pub(crate) available: bool,
    pub(crate) apps: Vec<BgApp>,
    pub(crate) error: Option<String>,
}

/// `flatpak ps --columns=application,child-pid` → app id → pids.
pub(crate) fn parse_ps(out: &str) -> BTreeMap<String, Vec<u32>> {
    let mut m: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for line in out.lines() {
        let mut f = line.split_whitespace();
        let Some(id) = f.next() else { continue };
        if !id.contains('.') || id.eq_ignore_ascii_case("application") {
            continue;
        }
        let e = m.entry(id.to_string()).or_default();
        if let Some(pid) = f.next().and_then(|p| p.parse().ok()) {
            e.push(pid);
        }
    }
    m
}

/// `[Application] name=<id>` of a `.flatpak-info` file.
pub(crate) fn flatpak_info_app(info: &str) -> Option<String> {
    let mut in_app = false;
    for line in info.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_app = line == "[Application]";
        } else if in_app && let Some(v) = line.strip_prefix("name=") {
            return Some(v.trim().to_string()).filter(|v| v.contains('.'));
        }
    }
    None
}

/// A window: its app id and, when it lives in a sandbox, that Flatpak id.
#[derive(Debug, Clone)]
pub(crate) struct Win {
    pub(crate) app_id: String,
    pub(crate) flatpak: Option<String>,
}

/// The running ids without a window, minus `ignored`, sorted.
pub(crate) fn background(running: &[String], windows: &[Win], ignored: &[String]) -> Vec<String> {
    let mut keys: HashMap<String, &str> = HashMap::new();
    for id in running {
        if let Some(last) = id.rsplit('.').next() {
            keys.insert(last.to_lowercase(), id);
        }
    }
    // Full ids last so they win over a short name another app shares.
    for id in running {
        keys.insert(id.to_lowercase(), id);
    }
    let mut windowed: HashSet<&str> = HashSet::new();
    for w in windows {
        if let Some(id) = keys.get(&w.app_id.to_lowercase()) {
            windowed.insert(id);
        }
        if let Some(f) = &w.flatpak {
            windowed.insert(f.as_str());
        }
    }
    let ignored: HashSet<String> = ignored.iter().map(|s| s.to_lowercase()).collect();
    let mut out: Vec<String> = running
        .iter()
        .filter(|id| !windowed.contains(id.as_str()) && !ignored.contains(&id.to_lowercase()))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

static STATE: LazyLock<watch::Sender<FlatpakBgState>> =
    LazyLock::new(|| watch::channel(FlatpakBgState::default()).0);

/// The settings the scan task reads (written on the GTK thread).
static SETTINGS: LazyLock<Mutex<FlatpakBgConfig>> =
    LazyLock::new(|| Mutex::new(FlatpakBgConfig::default()));

pub(crate) fn subscribe() -> watch::Receiver<FlatpakBgState> {
    static STARTED: OnceLock<()> = OnceLock::new();
    // Main thread: snapshot the config for the task.
    if let Ok(mut s) = SETTINGS.lock() {
        *s = config_manager().config().flatpak_bg().get_untracked();
    }
    STARTED.get_or_init(|| {
        mshell_services::tokio_rt_spawn(run());
    });
    STATE.subscribe()
}

/// Re-read the config (main thread) and scan now.
pub(crate) fn refresh() {
    if let Ok(mut s) = SETTINGS.lock() {
        *s = config_manager().config().flatpak_bg().get_untracked();
    }
    NUDGE.notify_one();
}

static NUDGE: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

/// `flatpak kill <id>` then rescan.
pub(crate) fn stop(id: String) {
    mshell_services::tokio_rt_spawn(async move {
        let r = tokio::process::Command::new("flatpak")
            .args(["kill", &id])
            .stdin(std::process::Stdio::null())
            .output()
            .await;
        if let Ok(o) = &r
            && !o.status.success()
        {
            tracing::warn!(app = %id, err = %String::from_utf8_lossy(&o.stderr).trim(), "flatpak kill failed");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        NUDGE.notify_one();
    });
}

async fn names() -> HashMap<String, String> {
    let out = tokio::process::Command::new("flatpak")
        .args(["list", "--app", "--columns=application,name"])
        .output()
        .await;
    let mut m = HashMap::new();
    if let Ok(o) = out {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Some((id, name)) = line.split_once('\t') {
                m.insert(id.trim().to_string(), name.trim().to_string());
            }
        }
    }
    m
}

fn sandbox_of(pid: i32) -> Option<String> {
    if pid <= 0 {
        return None;
    }
    let info = std::fs::read_to_string(format!("/proc/{pid}/root/.flatpak-info")).ok()?;
    flatpak_info_app(&info)
}

async fn scan(names: &mut HashMap<String, String>) -> FlatpakBgState {
    let out = match tokio::process::Command::new("flatpak")
        .args(["ps", "--columns=application,child-pid"])
        .stdin(std::process::Stdio::null())
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            return FlatpakBgState {
                available: true,
                apps: Vec::new(),
                error: Some(e.to_string()),
            };
        }
    };
    let running: Vec<String> = parse_ps(&String::from_utf8_lossy(&out.stdout))
        .into_keys()
        .collect();
    let clients = mshell_services::margo_service().clients.get();
    let windows: Vec<Win> = clients
        .iter()
        .map(|c| Win {
            app_id: c.class.get(),
            flatpak: sandbox_of(c.pid.get()),
        })
        .collect();
    let ignored = SETTINGS
        .lock()
        .map(|s| s.ignored_apps.clone())
        .unwrap_or_default();
    let ids = background(&running, &windows, &ignored);
    if ids.iter().any(|id| !names.contains_key(id)) {
        *names = self::names().await;
    }
    FlatpakBgState {
        available: true,
        apps: ids
            .into_iter()
            .map(|id| BgApp {
                name: names.get(&id).cloned().unwrap_or_else(|| id.clone()),
                id,
            })
            .collect(),
        error: (!out.status.success())
            .then(|| String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

async fn run() {
    if !margo_fs::path::on_path("flatpak") {
        STATE.send_replace(FlatpakBgState::default());
        return;
    }
    let mut names = HashMap::new();
    loop {
        if STATE.receiver_count() > 0 {
            let s = scan(&mut names).await;
            STATE.send_if_modified(|cur| {
                let changed = *cur != s;
                *cur = s;
                changed
            });
        }
        let secs = SETTINGS
            .lock()
            .map(|s| s.interval_seconds)
            .unwrap_or(10)
            .clamp(5, 600);
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(u64::from(secs))) => {}
            () = NUDGE.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_output_is_parsed_and_headers_skipped() {
        let m = parse_ps(
            "Application\tChild-PID\norg.ferdium.Ferdium\t4242\norg.ferdium.Ferdium\t4300\nio.ente.auth\t\n\n",
        );
        assert_eq!(m["org.ferdium.Ferdium"], [4242, 4300]);
        assert!(m["io.ente.auth"].is_empty());
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn flatpak_info_names_the_app() {
        let info =
            "[Application]\nname=org.ferdium.Ferdium\nruntime=x\n\n[Instance]\ninstance-id=1\n";
        assert_eq!(
            flatpak_info_app(info).as_deref(),
            Some("org.ferdium.Ferdium")
        );
        assert_eq!(flatpak_info_app("[Runtime]\nname=org.x.Platform\n"), None);
    }

    #[test]
    fn windows_match_by_id_last_segment_or_sandbox() {
        let running: Vec<String> = [
            "org.ferdium.Ferdium",
            "io.ente.auth",
            "com.valvesoftware.Steam",
            "org.x.Sync",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let win = |a: &str, f: Option<&str>| Win {
            app_id: a.into(),
            flatpak: f.map(str::to_string),
        };
        let windows = [
            win("Ferdium", None),
            win("steam", None),
            win("some-wrapper", Some("io.ente.auth")),
        ];
        assert_eq!(background(&running, &windows, &[]), ["org.x.Sync"]);
        assert!(background(&running, &windows, &["ORG.X.SYNC".into()]).is_empty());
        assert_eq!(background(&running, &[], &[]).len(), 4);
    }
}
