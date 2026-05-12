//! Twilight (margo's built-in blue-light filter) state reader.
//!
//! margo writes the live LUT phase + temperature into
//! `state.json:twilight`. We just project it onto something the
//! bar can render. Toggle / set / reset round-trip through
//! `mctl twilight …` — same dispatch surface the iced mshell used.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub enabled: bool,
    /// "day" / "night" / "transition" / "manual" (set-pinned).
    pub phase: String,
    /// Current colour temperature, kelvin. ~2300 K dim red, ~6500 K
    /// daylight neutral.
    pub temp_k: u32,
}

pub fn current() -> Option<Snapshot> {
    let path = state_path();
    let bytes = std::fs::read(&path).ok()?;
    let json: Value = serde_json::from_slice(&bytes).ok()?;
    let tw = json.get("twilight")?;
    Some(Snapshot {
        enabled: tw.get("enabled").and_then(Value::as_bool).unwrap_or(false),
        phase: tw
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or("day")
            .to_string(),
        temp_k: tw
            .get("current_temp_k")
            .and_then(Value::as_u64)
            .unwrap_or(6500) as u32,
    })
}

pub fn toggle() {
    let _ = Command::new("mctl")
        .args(["twilight", "toggle"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn state_path() -> PathBuf {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("margo").join("state.json");
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/margo/state.json"))
}
