//! UFW firewall status via `ufw status`.
//!
//! Returns `true` when the firewall is active. `ufw` normally
//! needs sudo, but plenty of setups (this one included) wire a
//! NOPASSWD rule for `ufw status` so the bar can read it without
//! elevation. If the call fails we just report `false`.

use std::process::Command;

pub fn enabled() -> bool {
    let Ok(out) = Command::new("ufw")
        .arg("status")
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    // "Status: active" / "Status: inactive"
    s.lines()
        .find_map(|l| l.strip_prefix("Status:"))
        .map(|rest| rest.trim() == "active")
        .unwrap_or(false)
}
