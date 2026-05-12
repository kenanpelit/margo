//! Public IP via api.ipify.org.
//!
//! Stage-equivalent of Noctalia's `plugin:6ee06e:nip`. Uses `curl`
//! with a short timeout — the reasoning is the same as audio /
//! network / mpris: no native HTTP client in the bar dep tree yet,
//! curl is already on every supported distro, and the response
//! lives until the next watchdog tick (default 15 min in Noctalia).

use std::process::Command;
use std::time::Duration;

/// Plain `String` rather than `Result` — the bar widget needs a
/// short value or an empty string; it doesn't need to know which
/// failure mode triggered the empty case.
pub fn fetch() -> String {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "5",
            "https://api.ipify.org",
        ])
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = out else {
        return String::new();
    };
    if !out.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[allow(dead_code)]
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
