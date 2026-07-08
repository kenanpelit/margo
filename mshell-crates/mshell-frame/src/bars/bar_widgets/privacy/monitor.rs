//! Singleton privacy monitor.
//!
//! One detection driver, shared by every Privacy pill **and** the panel,
//! so sensor detection runs exactly once regardless of how many monitors
//! (and therefore how many pills) exist — otherwise each pill would run
//! its own loop and the access log would gain N duplicate entries per
//! event.
//!
//! It polls three sources on a single 2 s main-loop timer:
//!
//!   * **Mic** — `audio_service().recording_streams` (wayle already
//!     filters this to real capture streams). Read synchronously.
//!   * **Camera** — scans `/proc/<pid>/fd` for symlinks to the real
//!     (non-metadata) `/dev/video*` capture nodes and maps the holders to
//!     process names. Run off the UI thread.
//!   * **Screen-share** — parses `pw-dump` for PipeWire video stream nodes
//!     whose `media.name` matches the usual screencast patterns
//!     (xdg-desktop-portal, gpu-screen-recorder, webrtc, …). Off-thread,
//!     and skippable via config.
//!
//! On every change it updates the reactive [`privacy_live_store`], records
//! started/stopped edges into the persisted access log
//! ([`mshell_cache::privacy_history`]), and — gated on config — fires a
//! `notify-send` toast when a sensor first goes active.

use std::collections::HashSet;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mshell_cache::privacy_history::{PrivacyEvent, push_event};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, PrivacyWidgetConfigStoreFields,
};
use mshell_services::audio_service;
use reactive_graph::prelude::*;
use reactive_stores::{ArcStore, Store};
use relm4::gtk::glib;

/// Live "in use right now" snapshot — the app names touching each sensor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Store)]
pub struct PrivacyLive {
    pub mic_apps: Vec<String>,
    pub cam_apps: Vec<String>,
    pub scr_apps: Vec<String>,
}

impl PrivacyLive {
    pub fn is_active(&self) -> bool {
        !self.mic_apps.is_empty() || !self.cam_apps.is_empty() || !self.scr_apps.is_empty()
    }
}

static LIVE: LazyLock<ArcStore<PrivacyLive>> =
    LazyLock::new(|| ArcStore::new(PrivacyLive::default()));

/// Latest detection snapshot — pills and the panel read this on their own
/// tick (a cheap in-memory `read_untracked` clone).
pub fn live_snapshot() -> PrivacyLive {
    (*LIVE.read_untracked()).clone()
}

static STARTED: AtomicBool = AtomicBool::new(false);

/// Start the detection driver once. Idempotent — safe to call from every
/// pill's `init`.
pub fn ensure_started() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    glib::spawn_future_local(async move {
        let mut prev = PrivacyLive::default();
        loop {
            let cfg = CfgSnapshot::read();

            let mic_apps = if cfg.track_mic {
                filter_apps(probe_mic(), &cfg.mic_filter)
            } else {
                Vec::new()
            };
            let cam_apps = if cfg.track_camera {
                filter_apps(camera_apps().await, &cfg.cam_filter)
            } else {
                Vec::new()
            };
            let scr_apps = if cfg.detect_screen_share {
                screen_apps().await
            } else {
                Vec::new()
            };

            let next = PrivacyLive {
                mic_apps,
                cam_apps,
                scr_apps,
            };

            if next != prev {
                record_edges(&prev, &next, &cfg);
                let n = next.clone();
                LIVE.update(|s| *s = n);
                prev = next;
            }

            glib::timeout_future(Duration::from_secs(2)).await;
        }
    });
}

// ── Config snapshot ─────────────────────────────────────────────────────

struct CfgSnapshot {
    enable_toast: bool,
    track_mic: bool,
    track_camera: bool,
    detect_screen_share: bool,
    mic_filter: String,
    cam_filter: String,
    history_limit: usize,
}

impl CfgSnapshot {
    fn read() -> Self {
        macro_rules! p {
            ($f:ident) => {
                config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .privacy()
                    .$f()
                    .get_untracked()
            };
        }
        Self {
            enable_toast: p!(enable_toast),
            track_mic: p!(track_mic),
            track_camera: p!(track_camera),
            detect_screen_share: p!(detect_screen_share),
            mic_filter: p!(mic_filter),
            cam_filter: p!(cam_filter),
            history_limit: p!(history_limit) as usize,
        }
    }
}

/// Drop app names matching the (optional) ignore regex.
fn filter_apps(apps: Vec<String>, filter: &str) -> Vec<String> {
    if filter.is_empty() {
        return apps;
    }
    match regex::Regex::new(filter) {
        Ok(re) => apps.into_iter().filter(|a| !re.is_match(a)).collect(),
        Err(_) => apps,
    }
}

// ── Edge recording (history + toasts) ───────────────────────────────────

fn record_edges(prev: &PrivacyLive, next: &PrivacyLive, cfg: &CfgSnapshot) {
    edges_for(&prev.mic_apps, &next.mic_apps, "Microphone", cfg);
    edges_for(&prev.cam_apps, &next.cam_apps, "Camera", cfg);
    edges_for(&prev.scr_apps, &next.scr_apps, "Screen", cfg);
}

fn edges_for(old: &[String], new: &[String], kind: &str, cfg: &CfgSnapshot) {
    let was_active = !old.is_empty();
    // Started: in `new` but not `old`.
    for app in new {
        if !old.contains(app) {
            log_event(app, kind, "started", cfg.history_limit);
        }
    }
    // Stopped: in `old` but not `new`.
    for app in old {
        if !new.contains(app) {
            log_event(app, kind, "stopped", cfg.history_limit);
        }
    }
    // Toast on the activation edge (idle → in use).
    if cfg.enable_toast && !was_active && !new.is_empty() {
        toast(kind, new);
    }
}

fn log_event(app: &str, kind: &str, action: &str, limit: usize) {
    let now = chrono::Local::now();
    push_event(
        PrivacyEvent {
            app: app.to_string(),
            kind: kind.to_string(),
            action: action.to_string(),
            time: now.format("%H:%M:%S").to_string(),
            timestamp: now.timestamp(),
        },
        limit,
    );
}

fn toast(kind: &str, apps: &[String]) {
    let (summary, icon) = match kind {
        "Camera" => ("Camera in use", "camera-video-symbolic"),
        "Screen" => ("Screen sharing started", "video-display-symbolic"),
        _ => ("Microphone in use", "microphone-sensitivity-high-symbolic"),
    };
    let body = apps.join(", ");
    let summary = summary.to_string();
    let icon = icon.to_string();
    relm4::spawn(async move {
        let _ = tokio::process::Command::new("notify-send")
            .args([
                "-a",
                "mshell",
                "-i",
                &icon,
                "-h",
                "string:x-canonical-private-synchronous:mshell-privacy",
                &summary,
                &body,
            ])
            .status()
            .await;
    });
}

// ── Mic probe (sync) ─────────────────────────────────────────────────────

fn probe_mic() -> Vec<String> {
    audio_service()
        .recording_streams
        .get()
        .iter()
        .map(|s| s.application_name.get().unwrap_or_else(|| s.name.get()))
        .filter(|n| !n.is_empty())
        .collect()
}

// ── Camera probe (off-thread) ────────────────────────────────────────────

/// Run the blocking `/proc` scan on relm4's runtime, return via oneshot so
/// the UI thread never blocks.
async fn camera_apps() -> Vec<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    relm4::spawn_blocking(move || {
        let _ = tx.send(probe_camera_blocking());
    });
    rx.await.unwrap_or_default()
}

fn probe_camera_blocking() -> Vec<String> {
    let cam_devices = real_camera_devices();
    if cam_devices.is_empty() {
        return Vec::new();
    }
    let mut pids: HashSet<u32> = HashSet::new();
    let Ok(proc_dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
            continue;
        };
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = std::fs::read_dir(&fd_dir) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(target) = std::fs::read_link(fd.path())
                && cam_devices.contains(&target)
            {
                pids.insert(pid);
                break;
            }
        }
    }
    let mut names: Vec<String> = pids
        .into_iter()
        .filter_map(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Real `/dev/video*` capture nodes, excluding metadata-only devices (the
/// odd-numbered nodes UVC webcams expose alongside the capture node).
fn real_camera_devices() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/video4linux") else {
        return out;
    };
    for entry in dir.flatten() {
        let dev = entry.file_name();
        let dev = dev.to_string_lossy();
        if !dev.starts_with("video") {
            continue;
        }
        let name = std::fs::read_to_string(entry.path().join("name")).unwrap_or_default();
        if name.contains("Metadata") {
            continue;
        }
        out.push(std::path::PathBuf::from(format!("/dev/{dev}")));
    }
    out
}

// ── Screen-share probe (off-thread, pw-dump) ─────────────────────────────

static SCREEN_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)^(xdph-streaming|gsr-|game capture|screen|desktop|display|cast|webrtc|v4l2)|screen-cast|screen-capture|desktop-capture|monitor-capture|window-capture|game-capture",
    )
    .expect("valid screencast regex")
});

async fn screen_apps() -> Vec<String> {
    // Run BOTH the pw-dump subprocess and the (potentially hundreds-of-KB)
    // JSON parse off the GTK main thread — this poller lives on
    // spawn_future_local, so parsing after the await would land the parse
    // back on the main loop and jank a frame every 2 s on PipeWire-heavy
    // sessions.
    relm4::spawn(async move {
        let bytes = tokio::process::Command::new("pw-dump")
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| o.stdout);
        match bytes {
            Some(bytes) => parse_screen_nodes(&bytes),
            None => Vec::new(),
        }
    })
    .await
    .unwrap_or_default()
}

fn parse_screen_nodes(bytes: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut apps: Vec<String> = Vec::new();
    for item in items {
        let Some(props) = item.get("info").and_then(|i| i.get("props")) else {
            continue;
        };
        let media_class = props
            .get("media.class")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if media_class.contains("Audio") || !media_class.contains("Video") {
            continue;
        }
        let media_name = props
            .get("media.name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !SCREEN_RE.is_match(media_name) {
            continue;
        }
        let app = props
            .get("application.name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| props.get("node.name").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        if !app.is_empty() && !apps.contains(&app) {
            apps.push(app);
        }
    }
    apps
}
