//! Singleton privacy monitor.
//!
//! One detection driver, shared by every Privacy pill **and** the panel,
//! so sensor detection runs exactly once regardless of how many monitors
//! (and therefore how many pills) exist — otherwise each pill would run
//! its own loop and the access log would gain N duplicate entries per
//! event.
//!
//! It is event-driven. One main-loop task re-evaluates the three sources
//! whenever one of them signals a change (and on a slow [`IDLE_TICK`] that
//! re-reads config and the cheap in-memory state):
//!
//!   * **Mic** — `audio_service().recording_streams` (wayle already
//!     filters this to real capture streams). Read synchronously; its
//!     `watch()` stream wakes the task.
//!   * **Camera** — an inotify watch on the real (non-metadata)
//!     `/dev/video*` capture nodes (`IN_OPEN` / `IN_CLOSE_*`, plus `/dev`
//!     create/delete for hotplug) keeps an open-minus-close count per node
//!     ([`OpenCounter`]). After a [`CAM_DEBOUNCE`] quiet window it decides:
//!     a balanced open+close (a browser or PipeWire enumerating devices)
//!     costs nothing, every node back at zero clears the camera without a
//!     scan, and only a node left open (or an overflow / hotplug) scans
//!     `/proc/<pid>/fd` (off the UI thread, our own processes only) to map
//!     holders to process names. Without inotify it falls back to a scan
//!     every [`FALLBACK_TICK`].
//!   * **Screen-share** — one long-lived `pw-dump --monitor` (only while
//!     `detect_screen_share` is on) streams PipeWire registry changes; the
//!     node table it maintains is matched against the usual screencast
//!     `media.name` patterns (xdg-desktop-portal, gpu-screen-recorder,
//!     webrtc, …). Nothing is spawned or parsed while the graph is quiet.
//!
//! On every change it updates the reactive [`privacy_live_store`], records
//! started/stopped edges into the persisted access log
//! ([`mshell_cache::privacy_history`]), and — gated on config — raises a toast
//! on the shell's own toast bus when a sensor first goes active.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use futures::StreamExt;
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

use mshell_cache::privacy_history::{PrivacyEvent, push_event};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, PrivacyWidgetConfigStoreFields,
};
use mshell_osd::toast::{ToastEvent, ToastSeverity, push_toast};
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

/// Latest detection snapshot (a cheap in-memory `read_untracked` clone).
pub fn live_snapshot() -> PrivacyLive {
    (*LIVE.read_untracked()).clone()
}

/// Subscribe the running reactive effect to snapshot changes: the pills
/// repaint from an effect that calls this instead of polling.
pub fn track_live() {
    LIVE.track();
}

static STARTED: AtomicBool = AtomicBool::new(false);

/// Re-evaluation tick while nothing signals: re-reads config and the cheap
/// in-memory state only (no `/proc` scan, no subprocess).
const IDLE_TICK: Duration = Duration::from_secs(10);
/// Tick used instead when the camera can't be watched with inotify and must
/// be scanned by polling (the old behaviour).
const FALLBACK_TICK: Duration = Duration::from_secs(2);

/// Woken by every source when its state may have changed. `Notify` keeps one
/// permit, so a burst of events collapses into one re-evaluation.
static WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);
/// The camera watch's latest pending decision ([`CamAction`] as `u8`), taken
/// by the driver loop.
static CAM_ACTION: AtomicU8 = AtomicU8::new(CamAction::None as u8);
/// Open-minus-close counts of the watched capture nodes. Shared between the
/// inotify task (events) and the driver loop (scan baselines).
static CAM_COUNTER: LazyLock<Mutex<OpenCounter>> =
    LazyLock::new(|| Mutex::new(OpenCounter::default()));
/// Quiet time after the last camera event before deciding: an
/// enumeration burst (open+close on every node) settles well inside it.
const CAM_DEBOUNCE: Duration = Duration::from_millis(400);

/// Start the detection driver once. Idempotent — safe to call from every
/// pill's `init`.
pub fn ensure_started() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    // Mic: the recording-stream list is a wayle property; wake on change.
    // (No audio service: the mic indicator never lights.)
    glib::spawn_future_local(async move {
        let Some(audio) = audio_service() else {
            return;
        };
        let mut streams = audio.recording_streams.watch();
        while streams.next().await.is_some() {
            WAKE.notify_one();
        }
    });

    let camera_watched = camera_watch::start();

    glib::spawn_future_local(async move {
        let mut prev = PrivacyLive::default();
        let mut cam_cache: Option<Vec<String>> = None;
        loop {
            let cfg = CfgSnapshot::read();
            screen_watch::set_wanted(cfg.detect_screen_share);

            let mic_apps = if cfg.track_mic {
                filter_apps(probe_mic(), &cfg.mic_filter)
            } else {
                Vec::new()
            };
            let cam_apps = if cfg.track_camera {
                let action = take_cam_action();
                if cam_cache.is_none() || !camera_watched || action == CamAction::Scan {
                    let scan = camera_apps().await;
                    if let Ok(mut c) = CAM_COUNTER.lock() {
                        c.set_baseline(&scan.open);
                    }
                    cam_cache = Some(scan.apps);
                } else if action == CamAction::Clear {
                    cam_cache = Some(Vec::new());
                }
                filter_apps(cam_cache.clone().unwrap_or_default(), &cfg.cam_filter)
            } else {
                cam_cache = None;
                Vec::new()
            };
            let scr_apps = if cfg.detect_screen_share {
                screen_watch::apps()
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

            let tick = if cfg.track_camera && !camera_watched {
                FALLBACK_TICK
            } else {
                IDLE_TICK
            };
            let _ = glib::future_with_timeout(tick, WAKE.notified()).await;
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
    let edges = sensor_edges(old, new);
    for app in edges.started {
        log_event(app, kind, "started", cfg.history_limit);
    }
    for app in edges.stopped {
        log_event(app, kind, "stopped", cfg.history_limit);
    }
    if cfg.enable_toast && edges.activated {
        toast(kind, new);
    }
}

/// What changed for one sensor between two samples of its app list.
#[derive(Debug, PartialEq, Eq)]
struct SensorEdges<'a> {
    /// In `new` but not `old`, in `new`'s order.
    started: Vec<&'a str>,
    /// In `old` but not `new`, in `old`'s order.
    stopped: Vec<&'a str>,
    /// The sensor went from idle to in use (the toast edge).
    activated: bool,
}

fn sensor_edges<'a>(old: &'a [String], new: &'a [String]) -> SensorEdges<'a> {
    SensorEdges {
        started: new
            .iter()
            .filter(|a| !old.contains(a))
            .map(String::as_str)
            .collect(),
        stopped: old
            .iter()
            .filter(|a| !new.contains(a))
            .map(String::as_str)
            .collect(),
        activated: old.is_empty() && !new.is_empty(),
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

/// Raise the shell's own toast on the activation edge.
///
/// This used to shell out to `notify-send`, which routed a privacy alert
/// through the notification daemon: it queued behind whatever else was on
/// screen, Do Not Disturb swallowed it, and it settled into the notification
/// history like a chat message. A sensor going live is a transient,
/// high-salience signal about the machine itself — the same class as "battery
/// critical" or "VPN disconnected" — so it belongs on the toast bus with them.
///
/// Gated by the widget's own `enable_toast` and deliberately *not* by the global
/// `toasts.enabled`: privacy already has a dedicated switch, and a sensor alert
/// should not be silenced by a preference about charging popups. The per-output
/// toast surfaces are created unconditionally, so the bus always has a renderer.
fn toast(kind: &str, apps: &[String]) {
    let (title, icon) = match kind {
        "Camera" => ("Camera in use", "camera-video-symbolic"),
        "Screen" => ("Screen sharing started", "video-display-symbolic"),
        _ => ("Microphone in use", "microphone-sensitivity-high-symbolic"),
    };
    push_toast(ToastEvent {
        icon: icon.to_string(),
        title: title.to_string(),
        body: Some(apps.join(", ")),
        severity: ToastSeverity::Warn,
    });
}

// ── Mic probe (sync) ─────────────────────────────────────────────────────

fn probe_mic() -> Vec<String> {
    audio_service()
        .map(|a| a.recording_streams.get())
        .unwrap_or_default()
        .iter()
        .map(|s| s.application_name.get().unwrap_or_else(|| s.name.get()))
        .filter(|n| !n.is_empty())
        .collect()
}

// ── Camera probe (off-thread) ────────────────────────────────────────────

/// Run the blocking `/proc` scan on relm4's runtime, return via oneshot so
/// the UI thread never blocks.
async fn camera_apps() -> CameraScan {
    let (tx, rx) = tokio::sync::oneshot::channel();
    relm4::spawn_blocking(move || {
        let _ = tx.send(probe_camera_blocking());
    });
    rx.await.unwrap_or_default()
}

fn probe_camera_blocking() -> CameraScan {
    let cam_devices = real_camera_devices();
    if cam_devices.is_empty() {
        return CameraScan::default();
    }
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    camera_holders(Path::new("/proc"), &cam_devices, uid)
}

/// What one `/proc` scan saw of the capture nodes.
#[derive(Debug, Default, PartialEq, Eq)]
struct CameraScan {
    /// `comm` names of the holders, sorted and deduplicated.
    apps: Vec<String>,
    /// Open fds per capture node (only nodes with at least one).
    open: HashMap<PathBuf, i64>,
}

/// The processes under `proc_root` owned by `uid` holding an fd on one of
/// `cam_devices`. Other users' fd dirs are unreadable anyway, so their
/// entries are skipped on the cheap `stat` instead of an `EACCES` per pid.
fn camera_holders(proc_root: &Path, cam_devices: &[PathBuf], uid: u32) -> CameraScan {
    use std::os::unix::fs::MetadataExt;
    let mut pids: HashSet<u32> = HashSet::new();
    let mut open: HashMap<PathBuf, i64> = HashMap::new();
    let Ok(proc_dir) = std::fs::read_dir(proc_root) else {
        return CameraScan::default();
    };
    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
            continue;
        };
        if entry.metadata().map(|m| m.uid()).ok() != Some(uid) {
            continue;
        }
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = std::fs::read_dir(&fd_dir) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(target) = std::fs::read_link(fd.path())
                && cam_devices.contains(&target)
            {
                pids.insert(pid);
                *open.entry(target).or_default() += 1;
            }
        }
    }
    let mut names: Vec<String> = pids
        .into_iter()
        .filter_map(|pid| {
            std::fs::read_to_string(proc_root.join(pid.to_string()).join("comm")).ok()
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    names.sort();
    names.dedup();
    CameraScan { apps: names, open }
}

/// Real `/dev/video*` capture nodes, excluding metadata-only devices (the
/// odd-numbered nodes UVC webcams expose alongside the capture node).
fn real_camera_devices() -> Vec<PathBuf> {
    real_camera_devices_in(Path::new("/sys/class/video4linux"))
}

/// [`real_camera_devices`] for a `video4linux` class dir at `sysfs`.
fn real_camera_devices_in(sysfs: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir(sysfs) else {
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
        out.push(PathBuf::from(format!("/dev/{dev}")));
    }
    out
}

// ── Camera watch (inotify) ───────────────────────────────────────────────

mod camera_watch {
    use super::{
        CAM_COUNTER, CAM_DEBOUNCE, CAM_DEVICE_MASK as DEVICE_MASK, CAM_DIR_MASK as DIR_MASK,
        CameraWake, WAKE, classify_camera_events, post_cam_action, real_camera_devices,
    };
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::path::PathBuf;

    use tokio::io::Interest;
    use tokio::io::unix::AsyncFd;

    /// Watch the capture nodes; `false` when inotify is unavailable (the
    /// caller then polls). The watch task lives for the process, on relm4's
    /// tokio runtime, and sleeps in epoll until a node is opened or closed.
    pub(super) fn start() -> bool {
        // SAFETY: plain syscall; the fd is owned below.
        let raw = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if raw < 0 {
            tracing::warn!("privacy: inotify unavailable, polling the camera");
            return false;
        }
        // SAFETY: `raw` is a fresh, valid fd nobody else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        if add_watch(&fd, "/dev", DIR_MASK).is_none() {
            return false;
        }
        watch_devices(&fd);

        relm4::spawn(async move {
            // SAFETY: the AsyncFd takes ownership of the `OwnedFd`, so the fd
            // stays open and names the same inotify instance until the
            // AsyncFd drops it; nothing else closes or replaces it.
            let Ok(afd) = (unsafe { AsyncFd::register_with_interest(fd, Interest::READABLE) })
            else {
                tracing::warn!("privacy: cannot register inotify fd; camera stays as last seen");
                return;
            };
            let mut buf = [0u8; 4096];
            loop {
                let Ok(guard) = afd.readable().await else {
                    return;
                };
                let mut wake = drain(&afd, guard, &mut buf);
                // Debounce: keep folding events in until the nodes have been
                // quiet for a window, so an enumeration burst is one decision.
                loop {
                    match tokio::time::timeout(CAM_DEBOUNCE, afd.readable()).await {
                        Err(_) => break,
                        Ok(Err(_)) => return,
                        Ok(Ok(guard)) => wake.merge(drain(&afd, guard, &mut buf)),
                    }
                }
                if wake.hotplug {
                    // A new node gets its own watch; removed ones drop theirs.
                    watch_devices(afd.get_ref());
                }
                let action = CAM_COUNTER
                    .lock()
                    .map(|mut c| c.decide())
                    .unwrap_or(super::CamAction::Scan);
                if post_cam_action(action) {
                    WAKE.notify_one();
                }
            }
        });
        true
    }

    /// Read every queued event, fold the opens/closes into the counter and
    /// return the batch's hotplug flag.
    fn drain(
        afd: &AsyncFd<OwnedFd>,
        mut guard: tokio::io::unix::AsyncFdReadyGuard<'_, OwnedFd>,
        buf: &mut [u8],
    ) -> CameraWake {
        let mut wake = CameraWake::default();
        loop {
            // SAFETY: reading into a local buffer of the given length.
            let n = unsafe {
                libc::read(
                    afd.get_ref().as_raw_fd(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                )
            };
            if n <= 0 {
                break;
            }
            let events = super::parse_inotify_events(&buf[..n as usize]);
            wake.merge(classify_camera_events(&events));
            if let Ok(mut c) = CAM_COUNTER.lock() {
                c.apply(&events);
            }
        }
        guard.clear_ready();
        wake
    }

    /// (Re-)watch every capture node and tell the counter which watch
    /// descriptor is which node. `inotify_add_watch` on an already watched
    /// path returns its existing descriptor.
    fn watch_devices(fd: &OwnedFd) {
        let nodes: Vec<(i32, PathBuf)> = real_camera_devices()
            .into_iter()
            .filter_map(|dev| {
                let wd = add_watch(fd, dev.to_str()?, DEVICE_MASK)?;
                Some((wd, dev))
            })
            .collect();
        if let Ok(mut c) = CAM_COUNTER.lock() {
            c.set_nodes(nodes);
        }
    }

    fn add_watch(fd: &OwnedFd, path: &str, mask: u32) -> Option<i32> {
        let c_path = CString::new(path).ok()?;
        // SAFETY: valid fd and NUL-terminated path.
        let wd = unsafe { libc::inotify_add_watch(fd.as_raw_fd(), c_path.as_ptr(), mask) };
        (wd >= 0).then_some(wd)
    }
}

/// What the camera watch asks the driver loop to do after a quiet window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum CamAction {
    /// Holders unchanged (balanced opens/closes): keep the cached names.
    None = 0,
    /// Every node is closed again: no holders, no scan needed.
    Clear = 1,
    /// A node is (still) open or the counts are unknown: scan `/proc`.
    Scan = 2,
}

impl CamAction {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Clear,
            2 => Self::Scan,
            _ => Self::None,
        }
    }

    /// Combine a pending (not yet taken) action with a newer decision. A
    /// newer real decision describes the current state and wins; `None`
    /// only says nothing changed since, so the pending one still applies.
    fn merge(self, newer: CamAction) -> CamAction {
        if newer == CamAction::None {
            self
        } else {
            newer
        }
    }
}

/// Record a decision for the driver loop; `true` when there is something
/// to do (wake it).
fn post_cam_action(action: CamAction) -> bool {
    if action == CamAction::None {
        return false;
    }
    let _ = CAM_ACTION.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |cur| {
        Some(CamAction::from_u8(cur).merge(action) as u8)
    });
    true
}

fn take_cam_action() -> CamAction {
    CamAction::from_u8(CAM_ACTION.swap(CamAction::None as u8, Ordering::SeqCst))
}

/// Open-minus-close count per watched capture node.
///
/// inotify reports `IN_OPEN` for every `open(2)` and one `IN_CLOSE_*` when
/// an open file description is finally released, so the difference is the
/// number of open descriptions. The baseline comes from a `/proc` scan,
/// which only sees our own processes and counts fds (a `dup` counts twice):
/// counts can be off either way, so they are clamped at zero and any doubt
/// resolves to another scan rather than a wrong answer.
#[derive(Debug, Default)]
struct OpenCounter {
    /// Watch descriptor → node.
    nodes: HashMap<i32, PathBuf>,
    /// Best-known open count per node.
    held: HashMap<PathBuf, i64>,
    /// Net opens per node since the last decision.
    net: HashMap<PathBuf, i64>,
    /// Counts unknown (no scan yet, overflow, hotplug): scan next.
    rescan: bool,
    /// A baseline scan has been applied.
    baselined: bool,
}

impl OpenCounter {
    fn set_nodes(&mut self, nodes: impl IntoIterator<Item = (i32, PathBuf)>) {
        self.nodes = nodes.into_iter().collect();
    }

    /// Fold one read's events in.
    fn apply(&mut self, events: &[InotifyEvent]) {
        for ev in events {
            if ev.mask & libc::IN_Q_OVERFLOW != 0 {
                self.rescan = true;
            } else if ev.mask & CAM_DIR_MASK != 0 {
                if ev.name.starts_with("video") {
                    self.rescan = true;
                }
            } else if let Some(node) = self.nodes.get(&ev.wd) {
                let d = if ev.mask & libc::IN_OPEN != 0 {
                    1
                } else if ev.mask & (libc::IN_CLOSE_WRITE | libc::IN_CLOSE_NOWRITE) != 0 {
                    -1
                } else {
                    continue;
                };
                *self.held.entry(node.clone()).or_default() += d;
                *self.net.entry(node.clone()).or_default() += d;
            }
        }
    }

    /// Decide what the quiet window's events mean, and start a new window.
    fn decide(&mut self) -> CamAction {
        let rescan = std::mem::take(&mut self.rescan) || !self.baselined;
        let changed = std::mem::take(&mut self.net).values().any(|&n| n != 0);
        for h in self.held.values_mut() {
            *h = (*h).max(0);
        }
        if rescan {
            CamAction::Scan
        } else if !changed {
            CamAction::None
        } else if self.held.values().all(|&h| h == 0) {
            CamAction::Clear
        } else {
            CamAction::Scan
        }
    }

    /// Replace the counts with what a `/proc` scan saw (nodes missing from
    /// `open` are closed).
    fn set_baseline(&mut self, open: &HashMap<PathBuf, i64>) {
        self.held = open.clone();
        self.baselined = true;
    }
}

/// Inotify events on a watched capture node: someone opened or closed it.
const CAM_DEVICE_MASK: u32 = libc::IN_OPEN | libc::IN_CLOSE_WRITE | libc::IN_CLOSE_NOWRITE;
/// Inotify events on `/dev`: a node appeared or went away.
const CAM_DIR_MASK: u32 = libc::IN_CREATE | libc::IN_DELETE;

/// What a batch of camera inotify events asks the watcher to do.
#[derive(Debug, Default, PartialEq, Eq)]
struct CameraWake {
    /// Re-probe who holds the camera.
    changed: bool,
    /// Re-scan the capture nodes and (re-)add their watches.
    hotplug: bool,
}

impl CameraWake {
    fn merge(&mut self, other: CameraWake) {
        self.changed |= other.changed;
        self.hotplug |= other.hotplug;
    }
}

/// Classify one read's worth of events from the camera inotify fd: a
/// `video*` node created/deleted in `/dev` is a hotplug, an open/close on a
/// watched node a change. Other `/dev` churn is ignored. A queue overflow
/// means events were dropped, so it counts as both.
fn classify_camera_events(events: &[InotifyEvent]) -> CameraWake {
    let mut wake = CameraWake::default();
    for ev in events {
        if ev.mask & libc::IN_Q_OVERFLOW != 0 {
            wake.hotplug = true;
            wake.changed = true;
        } else if ev.mask & CAM_DIR_MASK != 0 {
            if ev.name.starts_with("video") {
                wake.hotplug = true;
                wake.changed = true;
            }
        } else if ev.mask & CAM_DEVICE_MASK != 0 {
            wake.changed = true;
        }
    }
    wake
}

/// One decoded `struct inotify_event`.
#[derive(Debug, PartialEq, Eq)]
struct InotifyEvent {
    /// Watch descriptor the event is for (`-1` for a queue overflow).
    wd: i32,
    mask: u32,
    name: String,
}

/// Split a `read(2)` of an inotify fd into events: each is a 16-byte header
/// (`wd`, `mask`, `cookie`, `len`, native-endian) followed by `len` bytes of
/// NUL-padded name. A truncated tail is ignored.
fn parse_inotify_events(buf: &[u8]) -> Vec<InotifyEvent> {
    const HEADER: usize = 16;
    let u32_at =
        |i: usize| -> Option<u32> { Some(u32::from_ne_bytes(buf.get(i..i + 4)?.try_into().ok()?)) };
    let mut out = Vec::new();
    let mut off = 0;
    while off + HEADER <= buf.len() {
        let (Some(wd), Some(mask), Some(len)) = (u32_at(off), u32_at(off + 4), u32_at(off + 12))
        else {
            break;
        };
        let name_start = off + HEADER;
        let Some(name_bytes) = buf.get(name_start..name_start + len as usize) else {
            break;
        };
        let name_end = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(name_bytes.len());
        out.push(InotifyEvent {
            wd: wd as i32,
            mask,
            name: String::from_utf8_lossy(&name_bytes[..name_end]).into_owned(),
        });
        off = name_start + len as usize;
    }
    out
}

// ── Screen-share watch (pw-dump --monitor) ──────────────────────────────

static SCREEN_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)^(xdph-streaming|gsr-|game capture|screen|desktop|display|cast|webrtc|v4l2)|screen-cast|screen-capture|desktop-capture|monitor-capture|window-capture|game-capture",
    )
    .expect("valid screencast regex")
});

/// The props of one PipeWire object that screen-share detection needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NodeProps {
    media_class: String,
    media_name: String,
    app_name: String,
    node_name: String,
}

/// PipeWire's object table as `pw-dump --monitor` reports it: a full dump
/// first, then arrays of changed objects (`"info": null` = removed). Only
/// objects with a `media.class` are kept.
#[derive(Debug, Default)]
struct ScreenNodes {
    nodes: BTreeMap<u64, NodeProps>,
}

impl ScreenNodes {
    /// Fold one top-level `pw-dump` array into the table.
    fn apply(&mut self, dump: &serde_json::Value) {
        let Some(items) = dump.as_array() else {
            return;
        };
        for item in items {
            let Some(id) = item.get("id").and_then(|v| v.as_u64()) else {
                continue;
            };
            let info = item.get("info");
            if info.is_none_or(|i| i.is_null()) {
                self.nodes.remove(&id);
                continue;
            }
            // An update without props (e.g. a state change) keeps the old ones.
            let Some(props) = info.and_then(|i| i.get("props")) else {
                continue;
            };
            let s = |key: &str| {
                props
                    .get(key)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let node = NodeProps {
                media_class: s("media.class"),
                media_name: s("media.name"),
                app_name: s("application.name"),
                node_name: s("node.name"),
            };
            if node.media_class.is_empty() {
                self.nodes.remove(&id);
            } else {
                self.nodes.insert(id, node);
            }
        }
    }

    /// Apps owning a video (non-audio) stream whose `media.name` looks like
    /// a screencast, in object-id order, deduplicated.
    fn apps(&self) -> Vec<String> {
        let mut apps: Vec<String> = Vec::new();
        for node in self.nodes.values() {
            if node.media_class.contains("Audio") || !node.media_class.contains("Video") {
                continue;
            }
            if !SCREEN_RE.is_match(&node.media_name) {
                continue;
            }
            let app = if node.app_name.is_empty() {
                &node.node_name
            } else {
                &node.app_name
            };
            if !app.is_empty() && !apps.contains(app) {
                apps.push(app.clone());
            }
        }
        apps
    }
}

/// Pull every complete top-level JSON value off the front of `buf`, leaving
/// an incomplete trailing value in place for the next read. Unparseable
/// garbage clears the buffer.
fn drain_json_values(buf: &mut Vec<u8>) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut consumed = 0;
    let mut stream = serde_json::Deserializer::from_slice(buf).into_iter::<serde_json::Value>();
    loop {
        match stream.next() {
            Some(Ok(v)) => {
                out.push(v);
                consumed = stream.byte_offset();
            }
            Some(Err(e)) if e.is_eof() => break,
            Some(Err(_)) => {
                consumed = buf.len();
                break;
            }
            None => {
                consumed = buf.len();
                break;
            }
        }
    }
    buf.drain(..consumed);
    out
}

mod screen_watch {
    use super::{AsyncReadExt, Duration, LazyLock, Mutex, Notify, ScreenNodes, WAKE};
    use std::sync::atomic::{AtomicBool, Ordering};

    static WANTED: AtomicBool = AtomicBool::new(false);
    static WANTED_CHANGED: LazyLock<Notify> = LazyLock::new(Notify::new);
    static STARTED: AtomicBool = AtomicBool::new(false);
    static APPS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Backoff before restarting a `pw-dump` that exited (PipeWire restart).
    const RESTART_DELAY: Duration = Duration::from_secs(5);

    /// Latest screencast app list.
    pub(super) fn apps() -> Vec<String> {
        APPS.lock().map(|a| a.clone()).unwrap_or_default()
    }

    /// Run the monitor while `wanted`; stop the `pw-dump` child when not.
    pub(super) fn set_wanted(wanted: bool) {
        if WANTED.swap(wanted, Ordering::SeqCst) != wanted {
            WANTED_CHANGED.notify_one();
        }
        if wanted && !STARTED.swap(true, Ordering::SeqCst) {
            relm4::spawn(run());
        }
    }

    fn publish(apps: Vec<String>) {
        let changed = APPS
            .lock()
            .map(|mut cur| {
                let changed = *cur != apps;
                *cur = apps;
                changed
            })
            .unwrap_or(false);
        if changed {
            WAKE.notify_one();
        }
    }

    async fn wait_until(wanted: bool) {
        while WANTED.load(Ordering::SeqCst) != wanted {
            WANTED_CHANGED.notified().await;
        }
    }

    async fn run() {
        loop {
            wait_until(true).await;
            let child = tokio::process::Command::new("pw-dump")
                .arg("--monitor")
                .arg("--no-colors")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("privacy: cannot run pw-dump ({e}); screen-share detection off");
                    publish(Vec::new());
                    return;
                }
            };
            if let Some(stdout) = child.stdout.take() {
                tokio::select! {
                    () = follow(stdout) => {}
                    () = wait_until(false) => {}
                }
            }
            let _ = child.kill().await;
            publish(Vec::new());
            if WANTED.load(Ordering::SeqCst) {
                tokio::time::sleep(RESTART_DELAY).await;
            }
        }
    }

    /// Fold `pw-dump --monitor` output into the node table until EOF.
    async fn follow(mut stdout: tokio::process::ChildStdout) {
        let mut nodes = ScreenNodes::default();
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            match stdout.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    let values = super::drain_json_values(&mut buf);
                    if values.is_empty() {
                        continue;
                    }
                    for v in &values {
                        nodes.apply(v);
                    }
                    publish(nodes.apps());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: u64, class: &str, name: &str, app: &str) -> serde_json::Value {
        json!({
            "id": id,
            "type": "PipeWire:Interface:Node",
            "info": { "props": {
                "media.class": class,
                "media.name": name,
                "application.name": app,
                "node.name": format!("node-{id}"),
            }}
        })
    }

    #[test]
    fn full_dump_finds_screencasts_only() {
        let mut nodes = ScreenNodes::default();
        nodes.apply(&json!([
            node(30, "Stream/Input/Audio", "screen-cast", "obs"),
            node(31, "Stream/Input/Video", "webrtc-consume", "firefox"),
            node(32, "Video/Source", "xdph-streaming", ""),
            node(33, "Stream/Input/Video", "camera", "cheese"),
            node(34, "Stream/Input/Video", "desktop-capture", "firefox"),
            { "id": 2, "type": "PipeWire:Interface:Core", "info": { "props": {} } },
        ]));
        assert_eq!(nodes.apps(), vec!["firefox", "node-32"]);
    }

    #[test]
    fn monitor_updates_add_keep_and_remove_nodes() {
        let mut nodes = ScreenNodes::default();
        nodes.apply(&json!([node(40, "Stream/Input/Video", "screen", "obs")]));
        assert_eq!(nodes.apps(), vec!["obs"]);
        // A state-only change carries no props: the node stays.
        nodes.apply(
            &json!([{ "id": 40, "info": { "change-mask": ["state"], "state": "running" } }]),
        );
        assert_eq!(nodes.apps(), vec!["obs"]);
        // Removal.
        nodes.apply(&json!([{ "id": 40, "info": null }]));
        assert!(nodes.apps().is_empty());
    }

    #[test]
    fn drain_keeps_partial_values_for_the_next_read() {
        let mut buf = b"[{\"id\": 1}]\n[{\"id\": 2".to_vec();
        let values = drain_json_values(&mut buf);
        assert_eq!(values, vec![json!([{ "id": 1 }])]);
        assert_eq!(buf, b"\n[{\"id\": 2".to_vec());
        buf.extend_from_slice(b"}]\n");
        assert_eq!(drain_json_values(&mut buf), vec![json!([{ "id": 2 }])]);
        assert!(buf.iter().all(u8::is_ascii_whitespace));
        // Garbage is dropped rather than wedging the reader.
        let mut bad = b"}{oops".to_vec();
        assert!(drain_json_values(&mut bad).is_empty());
        assert!(bad.is_empty());
    }

    fn raw_event(mask: u32, name: &str, pad_to: usize) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&1i32.to_ne_bytes());
        v.extend_from_slice(&mask.to_ne_bytes());
        v.extend_from_slice(&0u32.to_ne_bytes());
        v.extend_from_slice(&(pad_to as u32).to_ne_bytes());
        let mut n = name.as_bytes().to_vec();
        n.resize(pad_to, 0);
        v.extend_from_slice(&n);
        v
    }

    #[test]
    fn inotify_events_parse_with_and_without_names() {
        let mut buf = raw_event(libc::IN_OPEN, "", 0);
        buf.extend(raw_event(libc::IN_CREATE, "video2", 16));
        buf.extend(raw_event(libc::IN_CLOSE_NOWRITE, "", 0));
        buf.extend_from_slice(&[1, 2, 3]); // truncated tail
        assert_eq!(
            parse_inotify_events(&buf),
            vec![
                InotifyEvent {
                    wd: 1,
                    mask: libc::IN_OPEN,
                    name: String::new()
                },
                InotifyEvent {
                    wd: 1,
                    mask: libc::IN_CREATE,
                    name: "video2".into()
                },
                InotifyEvent {
                    wd: 1,
                    mask: libc::IN_CLOSE_NOWRITE,
                    name: String::new()
                },
            ]
        );
    }

    #[test]
    fn inotify_names_without_nul_and_oversized_lengths() {
        // A name filling its whole `len` with no NUL terminator.
        let mut buf = raw_event(libc::IN_DELETE, "video10", 7);
        // A header claiming more name bytes than were read: dropped.
        let mut short = raw_event(libc::IN_CREATE, "video3", 16);
        short.truncate(16 + 4);
        buf.extend(short);
        assert_eq!(
            parse_inotify_events(&buf),
            vec![InotifyEvent {
                wd: 1,
                mask: libc::IN_DELETE,
                name: "video10".into()
            }]
        );
        assert!(parse_inotify_events(&[]).is_empty());
        // Non-UTF-8 names are kept lossily instead of failing the batch.
        let ev = parse_inotify_events(&raw_event(libc::IN_CREATE, "vid\u{fffd}", 16));
        assert_eq!(ev.len(), 1);
    }

    fn ev(mask: u32, name: &str) -> InotifyEvent {
        InotifyEvent {
            wd: 1,
            mask,
            name: name.to_string(),
        }
    }

    #[test]
    fn camera_events_classify_into_change_and_hotplug() {
        let none = CameraWake::default();
        let changed = CameraWake {
            changed: true,
            hotplug: false,
        };
        let plugged = CameraWake {
            changed: true,
            hotplug: true,
        };
        assert_eq!(classify_camera_events(&[]), none);
        assert_eq!(classify_camera_events(&[ev(libc::IN_OPEN, "")]), changed);
        assert_eq!(
            classify_camera_events(&[ev(libc::IN_CLOSE_WRITE, "")]),
            changed
        );
        assert_eq!(
            classify_camera_events(&[ev(libc::IN_CLOSE_NOWRITE, "")]),
            changed
        );
        assert_eq!(
            classify_camera_events(&[ev(libc::IN_CREATE, "video2")]),
            plugged
        );
        assert_eq!(
            classify_camera_events(&[ev(libc::IN_DELETE, "video0")]),
            plugged
        );
        // Unrelated /dev churn doesn't wake anything.
        assert_eq!(
            classify_camera_events(&[ev(libc::IN_CREATE, "ttyUSB0")]),
            none
        );
        assert_eq!(
            classify_camera_events(&[ev(libc::IN_DELETE, "v4l-subdev0")]),
            none
        );
        // A watch dropped with its (unplugged) node is not itself a change.
        assert_eq!(classify_camera_events(&[ev(libc::IN_IGNORED, "")]), none);
        // The kernel queue overflowed: opens/closes and hotplugs may have
        // been lost, so re-probe and re-watch.
        assert_eq!(
            classify_camera_events(&[ev(libc::IN_Q_OVERFLOW, "")]),
            plugged
        );
        // Batches combine.
        assert_eq!(
            classify_camera_events(&[ev(libc::IN_CREATE, "tty1"), ev(libc::IN_OPEN, "")]),
            changed
        );
    }

    #[test]
    fn sensor_edges_report_starts_stops_and_activation() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (idle, a, ab, b) = (v(&[]), v(&["a"]), v(&["a", "b"]), v(&["b"]));
        assert_eq!(
            sensor_edges(&idle, &ab),
            SensorEdges {
                started: vec!["a", "b"],
                stopped: vec![],
                activated: true
            }
        );
        // A second app joining an active sensor is not a new activation.
        assert_eq!(
            sensor_edges(&a, &ab),
            SensorEdges {
                started: vec!["b"],
                stopped: vec![],
                activated: false
            }
        );
        // Hand-over between apps: one stops, one starts, still active.
        assert_eq!(
            sensor_edges(&a, &b),
            SensorEdges {
                started: vec!["b"],
                stopped: vec!["a"],
                activated: false
            }
        );
        assert_eq!(
            sensor_edges(&ab, &idle),
            SensorEdges {
                started: vec![],
                stopped: vec!["a", "b"],
                activated: false
            }
        );
        assert_eq!(
            sensor_edges(&idle, &idle),
            SensorEdges {
                started: vec![],
                stopped: vec![],
                activated: false
            }
        );
    }

    #[test]
    fn app_filter_drops_matches_and_tolerates_bad_regexes() {
        let apps = || {
            vec![
                "Firefox".to_string(),
                "obs".to_string(),
                "speech-dispatcher".to_string(),
            ]
        };
        assert_eq!(filter_apps(apps(), ""), apps());
        assert_eq!(filter_apps(apps(), "^speech"), vec!["Firefox", "obs"]);
        assert_eq!(
            filter_apps(apps(), "(?i)firefox|obs"),
            vec!["speech-dispatcher"]
        );
        // An invalid pattern filters nothing rather than hiding every app.
        assert_eq!(filter_apps(apps(), "(unclosed"), apps());
    }

    #[test]
    fn camera_devices_skip_metadata_nodes_and_non_video_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let mk = |dev: &str, name: Option<&str>| {
            let d = tmp.path().join(dev);
            std::fs::create_dir(&d).unwrap();
            if let Some(n) = name {
                std::fs::write(d.join("name"), format!("{n}\n")).unwrap();
            }
        };
        mk("video0", Some("Integrated Camera: Integrated C"));
        mk("video1", Some("Integrated Camera: Metadata"));
        mk("video2", None); // unreadable name → still a capture node
        mk("v4l-subdev0", Some("ov5675"));
        let mut devs = real_camera_devices_in(tmp.path());
        devs.sort();
        assert_eq!(
            devs,
            vec![PathBuf::from("/dev/video0"), PathBuf::from("/dev/video2")]
        );
        assert!(real_camera_devices_in(&tmp.path().join("missing")).is_empty());
    }

    #[test]
    fn camera_holders_match_fd_links_to_capture_nodes() {
        use std::os::unix::fs::symlink;
        let proc = tempfile::tempdir().unwrap();
        let task = |pid: &str, comm: Option<&str>, fds: &[&str]| {
            let dir = proc.path().join(pid);
            std::fs::create_dir_all(dir.join("fd")).unwrap();
            if let Some(c) = comm {
                std::fs::write(dir.join("comm"), format!("{c}\n")).unwrap();
            }
            for (i, target) in fds.iter().enumerate() {
                symlink(target, dir.join("fd").join(i.to_string())).unwrap();
            }
        };
        task("100", Some("firefox"), &["/dev/null", "/dev/video0"]);
        task("101", Some("firefox"), &["/dev/video0"]); // second process, same name
        task("102", Some("cheese"), &["/dev/video2"]);
        task("103", Some("metadata-reader"), &["/dev/video1"]);
        task("104", Some("bash"), &["/dev/pts/0"]);
        task("105", None, &["/dev/video0"]); // exited before comm was read
        task("self", Some("not-a-pid"), &["/dev/video0"]);
        let devices = [PathBuf::from("/dev/video0"), PathBuf::from("/dev/video2")];
        // SAFETY: getuid has no preconditions.
        let me = unsafe { libc::getuid() };
        let scan = camera_holders(proc.path(), &devices, me);
        assert_eq!(scan.apps, vec!["cheese", "firefox"]);
        // Every visible open fd is counted, as the inotify counter's baseline.
        assert_eq!(
            scan.open,
            HashMap::from([
                (PathBuf::from("/dev/video0"), 3),
                (PathBuf::from("/dev/video2"), 1),
            ])
        );
        assert!(camera_holders(proc.path(), &[], me).apps.is_empty());
        assert!(
            camera_holders(&proc.path().join("missing"), &devices, me)
                .apps
                .is_empty()
        );
        // Processes owned by another uid are skipped without reading their
        // fd dir (the kernel would refuse it anyway).
        assert_eq!(
            camera_holders(proc.path(), &devices, me.wrapping_add(1)),
            CameraScan::default()
        );
    }

    fn wd_ev(wd: i32, mask: u32) -> InotifyEvent {
        InotifyEvent {
            wd,
            mask,
            name: String::new(),
        }
    }

    fn counter_on(devs: &[(i32, &str)]) -> OpenCounter {
        let mut c = OpenCounter::default();
        c.set_nodes(devs.iter().map(|(wd, p)| (*wd, PathBuf::from(p))));
        c
    }

    #[test]
    fn probe_open_close_pairs_need_no_scan() {
        let mut c = counter_on(&[(1, "/dev/video0"), (2, "/dev/video2")]);
        c.set_baseline(&HashMap::new());
        // A browser enumerating devices: open+close on every node.
        c.apply(&[
            wd_ev(1, libc::IN_OPEN),
            wd_ev(1, libc::IN_CLOSE_NOWRITE),
            wd_ev(2, libc::IN_OPEN),
            wd_ev(2, libc::IN_CLOSE_WRITE),
        ]);
        assert_eq!(c.decide(), CamAction::None);
        // Split over two reads inside one debounce window: still nothing.
        c.apply(&[wd_ev(1, libc::IN_OPEN)]);
        c.apply(&[wd_ev(1, libc::IN_CLOSE_NOWRITE)]);
        assert_eq!(c.decide(), CamAction::None);
        // No events at all.
        assert_eq!(c.decide(), CamAction::None);
    }

    #[test]
    fn a_node_left_open_asks_for_names_and_closing_clears_without_scan() {
        let mut c = counter_on(&[(1, "/dev/video0")]);
        c.set_baseline(&HashMap::new());
        c.apply(&[wd_ev(1, libc::IN_OPEN)]);
        assert_eq!(c.decide(), CamAction::Scan);
        // The scan found the holder.
        c.set_baseline(&HashMap::from([(PathBuf::from("/dev/video0"), 1)]));
        // A probe while the camera is in use changes nothing.
        c.apply(&[wd_ev(1, libc::IN_OPEN), wd_ev(1, libc::IN_CLOSE_NOWRITE)]);
        assert_eq!(c.decide(), CamAction::None);
        // The holder closes: count back to zero, no scan needed.
        c.apply(&[wd_ev(1, libc::IN_CLOSE_WRITE)]);
        assert_eq!(c.decide(), CamAction::Clear);
    }

    #[test]
    fn one_of_two_holders_closing_rescans_for_the_remaining_names() {
        let mut c = counter_on(&[(1, "/dev/video0"), (2, "/dev/video2")]);
        c.set_baseline(&HashMap::from([
            (PathBuf::from("/dev/video0"), 1),
            (PathBuf::from("/dev/video2"), 1),
        ]));
        c.apply(&[wd_ev(1, libc::IN_CLOSE_WRITE)]);
        assert_eq!(c.decide(), CamAction::Scan);
    }

    #[test]
    fn undercounted_nodes_clamp_instead_of_hiding_another_open_node() {
        // A holder we couldn't see (other uid) closes node 1 while node 2 is
        // newly opened: node 1 must not cancel node 2 out.
        let mut c = counter_on(&[(1, "/dev/video0"), (2, "/dev/video2")]);
        c.set_baseline(&HashMap::new());
        c.apply(&[wd_ev(1, libc::IN_CLOSE_WRITE), wd_ev(2, libc::IN_OPEN)]);
        assert_eq!(c.decide(), CamAction::Scan);
        // The stray close alone, from zero, is just a clear.
        let mut c = counter_on(&[(1, "/dev/video0")]);
        c.set_baseline(&HashMap::new());
        c.apply(&[wd_ev(1, libc::IN_CLOSE_WRITE)]);
        assert_eq!(c.decide(), CamAction::Clear);
        // ...and it doesn't leave the node at -1 to swallow the next open.
        c.apply(&[wd_ev(1, libc::IN_OPEN)]);
        assert_eq!(c.decide(), CamAction::Scan);
    }

    #[test]
    fn overflow_hotplug_and_unknown_state_force_a_rescan() {
        let mut c = counter_on(&[(1, "/dev/video0")]);
        // Before the first scan nothing is known.
        assert_eq!(c.decide(), CamAction::Scan);
        c.set_baseline(&HashMap::new());
        c.apply(&[InotifyEvent {
            wd: -1,
            mask: libc::IN_Q_OVERFLOW,
            name: String::new(),
        }]);
        assert_eq!(c.decide(), CamAction::Scan);
        c.set_baseline(&HashMap::new());
        c.apply(&[InotifyEvent {
            wd: 9,
            mask: libc::IN_CREATE,
            name: "video4".into(),
        }]);
        assert_eq!(c.decide(), CamAction::Scan);
        c.set_baseline(&HashMap::new());
        // Unrelated /dev churn and events on unknown watches are ignored.
        c.apply(&[
            InotifyEvent {
                wd: 9,
                mask: libc::IN_CREATE,
                name: "ttyUSB0".into(),
            },
            wd_ev(42, libc::IN_OPEN),
        ]);
        assert_eq!(c.decide(), CamAction::None);
    }

    #[test]
    fn pending_actions_keep_the_latest_real_decision() {
        assert_eq!(CamAction::Scan.merge(CamAction::None), CamAction::Scan);
        assert_eq!(CamAction::Scan.merge(CamAction::Clear), CamAction::Clear);
        assert_eq!(CamAction::Clear.merge(CamAction::Scan), CamAction::Scan);
        assert_eq!(CamAction::None.merge(CamAction::None), CamAction::None);
    }
}
