//! Schema for margo's state document.
//!
//! Delivered over margo's IPC socket — pushed on every state change via
//! `watch state`, or fetched one-shot via `get state` ([`read`]). The
//! polled `$XDG_RUNTIME_DIR/margo/state.json` file this once described was
//! removed with dwl-ipc-v2 (2026-06-01); the `StateJson` type name is kept
//! for the wire schema. We deserialize the subset we care about and project
//! it into the reactive `Workspace` / `Client` / `Monitor` properties the
//! mshell widgets read.

use serde::Deserialize;

/// Top-level margo state document (the `get`/`watch state` reply body).
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

/// Send a one-shot `dispatch …` frame over margo's IPC socket and read
/// (then discard) the `{"ok"/"error"}` reply. Same newline protocol as
/// [`read`], bounded by [`READ_TIMEOUT`] in both directions. Lets the
/// shell issue compositor actions over the socket it already speaks,
/// instead of forking an `mctl` process per dispatch. `req` is the frame
/// without its trailing newline (e.g. `"dispatch view 4"`).
pub fn send_dispatch(req: &str) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader, Write};
    let mut sock = std::os::unix::net::UnixStream::connect(socket_path())?;
    sock.set_read_timeout(Some(READ_TIMEOUT))?;
    sock.set_write_timeout(Some(READ_TIMEOUT))?;
    sock.write_all(req.as_bytes())?;
    sock.write_all(b"\n")?;
    // Drain the single-line reply so the compositor's writer doesn't
    // block; the outcome is best-effort (the old `mctl` path only logged
    // a non-zero exit), so a read timeout here is not an error.
    let mut reader = BufReader::new(sock);
    let mut line = String::new();
    let _ = reader.read_line(&mut line);
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowest_tag_maps_lowest_set_bit() {
        assert_eq!(lowest_tag(0), 0, "no bits → no active workspace");
        assert_eq!(lowest_tag(0b1), 1);
        assert_eq!(lowest_tag(0b100), 3);
        assert_eq!(lowest_tag(0b1010), 2, "takes the lowest set bit");
        assert_eq!(lowest_tag(1 << 8), 9);
    }

    #[test]
    fn monitor_id_is_deterministic_positive_and_distinct() {
        assert_eq!(monitor_id("DP-1"), monitor_id("DP-1"));
        assert!(monitor_id("eDP-1") >= 0);
        assert_ne!(monitor_id("DP-1"), monitor_id("DP-2"));
    }

    #[test]
    fn parses_minimal_document() {
        let json = r#"{"active_output":"DP-1","layouts":["tile"],
            "outputs":[],"clients":[],"tag_count":9}"#;
        let s: StateJson = serde_json::from_str(json).expect("minimal doc parses");
        assert_eq!(s.active_output, "DP-1");
        assert_eq!(s.focused_idx, None, "absent focused_idx defaults to None");
        assert_eq!(
            s.keyboard_layout, "",
            "absent keyboard_layout defaults empty"
        );
        assert_eq!(s.tag_count, 9);
    }

    #[test]
    fn accepts_null_focused_idx() {
        // Regression: margo emits `focused_idx: null` (not absent) with no
        // window focused; an i64-only field rejected the whole snapshot and
        // the tag-pill row stayed empty until a window opened.
        let json = r#"{"active_output":"DP-1","focused_idx":null,"layouts":[],
            "outputs":[],"clients":[],"tag_count":9}"#;
        let s: StateJson = serde_json::from_str(json).expect("null focused_idx parses");
        assert_eq!(s.focused_idx, None);

        let json2 = r#"{"active_output":"DP-1","focused_idx":3,"layouts":[],
            "outputs":[],"clients":[],"tag_count":9}"#;
        let s2: StateJson = serde_json::from_str(json2).unwrap();
        assert_eq!(s2.focused_idx, Some(3));
    }

    #[test]
    fn output_and_client_optional_fields_default() {
        // Older margo builds omit urgent/global/scratchpad/mode/wallpaper(s)
        // and focus_history — the shell must still parse the snapshot.
        let json = r#"{
            "active_output":"DP-1","layouts":["tile"],"tag_count":9,
            "outputs":[{"name":"DP-1","active":true,"active_tag_mask":1,
                "occupied_tag_mask":1,"layout_idx":0,"width":1920,"height":1080,
                "x":0,"y":0,"scale":1.0}],
            "clients":[{"app_id":"kitty","title":"t","pid":42,"focused":true,
                "floating":false,"fullscreen":false,"minimized":false,"tags":1,
                "monitor":"DP-1","monitor_idx":0,"idx":0,"x":0,"y":0,
                "width":800,"height":600}]
        }"#;
        let s: StateJson = serde_json::from_str(json).expect("optional fields default");
        assert_eq!(s.outputs.len(), 1);
        assert!(s.outputs[0].focus_history.is_empty());
        assert!(s.outputs[0].mode.is_none(), "absent mode defaults to None");
        assert!(s.outputs[0].wallpaper.is_empty());
        assert!(!s.clients[0].urgent && !s.clients[0].global && !s.clients[0].scratchpad);
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(serde_json::from_str::<StateJson>("not json").is_err());
        // Missing a required field (active_output) → reject.
        assert!(serde_json::from_str::<StateJson>(r#"{"layouts":[]}"#).is_err());
    }
}
