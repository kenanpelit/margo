//! Pending package updates via `checkupdates` (pacman-contrib).
//!
//! Returns the line count or 0 when the helper isn't installed.
//! `checkupdates` is the standard "what's pending" CLI on Arch + AUR
//! derivatives (CachyOS, EndeavourOS, …) and writes one update per
//! line, so a simple wc-style fold is enough.

use std::process::Command;

pub fn count() -> u32 {
    let Ok(out) = Command::new("checkupdates")
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return 0;
    };
    if !out.status.success() {
        return 0;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines().filter(|l| !l.trim().is_empty()).count() as u32
}
