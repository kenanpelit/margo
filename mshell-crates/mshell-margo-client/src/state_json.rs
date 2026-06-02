//! Schema for `$XDG_RUNTIME_DIR/margo/state.json`.
//!
//! margo writes the file on every state change (focus / tag / arrange
//! / hotplug / config reload). We deserialize the subset we care
//! about and project it into the reactive `Workspace` / `Client` /
//! `Monitor` properties the mshell widgets read.

use serde::Deserialize;

/// Top-level state.json document.
#[derive(Debug, Clone, Deserialize)]
pub struct StateJson {
    pub active_output: String,
    /// `null` in the JSON when no client is focused — common at
    /// session start, on every workspace that has no windows, and
    /// briefly during focus-out transitions. Margo serialises the
    /// raw `Option<usize>` as `null`/integer; the old `i64`-only
    /// type rejected the whole document with `invalid type: null,
    /// expected i64` and `apply_snapshot` never ran. Symptom: at
    /// first login the tag-pill row stayed empty until the user
    /// opened a window (which gave focused_idx a real integer and
    /// the parser finally accepted the snapshot).
    #[serde(default, deserialize_with = "deserialize_focused_idx")]
    pub focused_idx: Option<i64>,
    pub layouts: Vec<String>,
    pub outputs: Vec<RawOutput>,
    pub clients: Vec<RawClient>,
    pub tag_count: u32,
    /// Human-readable name of the active xkb keyboard layout (e.g.
    /// "English (US)", "Turkish"). Empty until the compositor has
    /// observed a key event. `#[serde(default)]` for forward-compat
    /// with older margo builds that don't emit the field.
    #[serde(default)]
    pub keyboard_layout: String,
}

/// Coerce `null` / missing / integer into `Option<i64>`. serde's
/// default behaviour for `Option<i64>` already accepts `null` and
/// missing, but margo emits `focused_idx` as `null` (not absent)
/// in the JSON, and an older version of this schema declared the
/// field as bare `i64`, so we keep the explicit deserializer for
/// belt-and-suspenders (and to document the wire shape).
fn deserialize_focused_idx<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<i64>::deserialize(deserializer)
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawOutput {
    pub name: String,
    pub active: bool,
    pub active_tag_mask: u32,
    pub occupied_tag_mask: u32,
    #[serde(default)]
    pub focus_history: Vec<String>,
    pub layout_idx: usize,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    #[serde(default)]
    pub mode: Option<RawMode>,
    #[serde(default)]
    pub wallpaper: String,
    #[serde(default)]
    pub wallpapers_by_tag: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawMode {
    pub physical_width: u32,
    pub physical_height: u32,
    pub refresh_mhz: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawClient {
    pub app_id: String,
    pub title: String,
    pub pid: i32,
    pub focused: bool,
    pub floating: bool,
    pub fullscreen: bool,
    pub minimized: bool,
    #[serde(default)]
    pub urgent: bool,
    #[serde(default)]
    pub global: bool,
    #[serde(default)]
    pub scratchpad: bool,
    pub tags: u32,
    pub monitor: String,
    pub monitor_idx: i32,
    pub idx: i32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Resolve margo's IPC socket: `$MARGO_SOCKET` if set, else
/// `$XDG_RUNTIME_DIR/margo/margo-ipc.sock`. This is the live state
/// source — the shell subscribes to `watch state` over it.
pub fn socket_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("MARGO_SOCKET") {
        return std::path::PathBuf::from(p);
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            std::path::PathBuf::from(format!("/run/user/{uid}"))
        });
    runtime.join("margo").join("margo-ipc.sock")
}

/// Hard cap on a single synchronous `get state` round-trip. A healthy
/// compositor answers in microseconds; this only bites when margo's
/// single-threaded event loop is too busy to service the IPC socket
/// promptly — most visibly right after resume, when it's draining an
/// input backlog. Several callers (`margo_layout` bar pill on a 500 ms
/// `glib::timeout_add_local` tick, launcher tag/window providers) run
/// this on the **GTK main thread**, so without a deadline an
/// unresponsive compositor froze the whole shell — no log, just a dead
/// UI until margo caught up (the observed post-suspend "mshell 1-2 dk
/// dondu" symptom). With the cap the worst case is a 250 ms main-thread
/// stall per tick; every caller already treats `None` as "use the
/// last-known / default state".
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

/// One-shot `get state` over margo's IPC socket. Returns `None` when
/// the compositor isn't running, is too slow to answer within
/// [`READ_TIMEOUT`], or the reply doesn't parse. Used by the
/// synchronous callers (launcher tag/window providers, layout pill)
/// that want a quick snapshot without subscribing.
pub fn read() -> Option<StateJson> {
    use std::io::{BufRead, BufReader, Write};
    let mut sock = std::os::unix::net::UnixStream::connect(socket_path()).ok()?;
    // Bound both directions so a busy/suspended compositor can never
    // block a synchronous (often main-thread) caller indefinitely.
    sock.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    sock.set_write_timeout(Some(READ_TIMEOUT)).ok()?;
    sock.write_all(b"get state\n").ok()?;
    let mut reader = BufReader::new(sock);
    let mut line = String::new();
    // A timeout surfaces as a `WouldBlock`/`TimedOut` error here, which
    // `.ok()?` turns into `None` — the graceful "no snapshot" path.
    reader.read_line(&mut line).ok()?;
    serde_json::from_str::<StateJson>(line.trim()).ok()
}

/// Margo encodes tag IDs as a bitmask — convert the lowest-set bit
/// to a 1-indexed tag number (1..=9). Returns 0 when the mask has
/// no bits set, mirroring the "no active workspace" sentinel.
pub fn lowest_tag(mask: u32) -> u32 {
    if mask == 0 {
        0
    } else {
        mask.trailing_zeros() + 1
    }
}

/// Stable hash for an output's connector name → `MonitorId` slot.
/// Hyprland gives each monitor a kernel-assigned `i64`; margo
/// publishes names like `DP-3` / `eDP-1`. We just hash to keep
/// the existing widget code that uses `MonitorId` as a HashMap
/// key / sort key happy. Collisions are extremely unlikely with
/// at most a handful of outputs.
pub fn monitor_id(name: &str) -> i64 {
    let mut hash: i64 = 5381;
    for b in name.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(b as i64);
    }
    // Keep positive — the upstream MonitorId is positive in practice.
    hash & i64::MAX
}
