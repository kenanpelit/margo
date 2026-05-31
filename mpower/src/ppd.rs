//! power-profiles-daemon control via `powerprofilesctl`.
//!
//! We shell out to the CLI rather than speaking D-Bus directly: it is the
//! same path the retired script used, has zero extra dependencies, and the
//! daemon's tick rate makes a fork-per-tick negligible. Profile names are
//! ppd's own: `performance` / `balanced` / `power-saver`.

use std::process::Command;

/// The currently active profile, or `None` if ppd / the CLI is unavailable.
pub fn get() -> Option<String> {
    let out = Command::new("powerprofilesctl").arg("get").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Set the active profile. Returns `true` on success.
pub fn set(profile: &str) -> bool {
    Command::new("powerprofilesctl")
        .arg("set")
        .arg(profile)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
