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
    pub focused_idx: i64,
    pub layouts: Vec<String>,
    pub outputs: Vec<RawOutput>,
    pub clients: Vec<RawClient>,
    pub tag_count: u32,
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

/// Resolve the conventional path to state.json — same logic mlock
/// uses (`XDG_RUNTIME_DIR/margo/state.json`, falling back to
/// `/run/user/<uid>/margo/state.json` when the env-var is unset).
pub fn state_json_path() -> std::path::PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            std::path::PathBuf::from(format!("/run/user/{uid}"))
        });
    runtime.join("margo").join("state.json")
}

/// Read and parse the current state.json. Returns `None` when the
/// file is missing (margo not running) or the parse fails.
pub fn read() -> Option<StateJson> {
    let path = state_json_path();
    let raw = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<StateJson>(&raw) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "state.json parse failed");
            None
        }
    }
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
