//! Power profile via `powerprofilesctl get`.
//!
//! Returns the current profile name (`balanced` / `performance` /
//! `power-saver`) or `None` when the daemon isn't installed.

use std::process::Command;

pub fn current() -> Option<String> {
    let out = Command::new("powerprofilesctl")
        .arg("get")
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

pub fn cycle() {
    // balanced → performance → power-saver → balanced
    let next = match current().as_deref() {
        Some("balanced") => "performance",
        Some("performance") => "power-saver",
        _ => "balanced",
    };
    let _ = Command::new("powerprofilesctl")
        .args(["set", next])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
