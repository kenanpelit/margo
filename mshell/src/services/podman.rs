//! Running podman containers via `podman ps -q`.
//!
//! Each running container prints one short id; counting the lines
//! gives us the running-container count without parsing the longer
//! human-readable table.

use std::process::Command;

pub fn running_count() -> u32 {
    let Ok(out) = Command::new("podman")
        .args(["ps", "-q"])
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
