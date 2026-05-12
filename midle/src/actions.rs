//! Execute step commands. Each command is run via `sh -c` so users
//! can compose pipes / && / variables exactly as they would in a
//! terminal. Stdout/stderr are inherited so any output ends up in
//! the daemon's journal alongside our tracing logs.

use std::process::Command;
use tracing::{info, warn};

pub fn spawn_shell(label: &str, command: &str) {
    let cmd = command.trim();
    if cmd.is_empty() {
        return;
    }
    info!(step = label, "running: {cmd}");
    match Command::new("sh").arg("-c").arg(cmd).spawn() {
        Ok(_) => {}
        Err(e) => warn!(step = label, "spawn failed: {e}"),
    }
}

pub fn notify(label: &str, body: &str) {
    let _ = Command::new("notify-send")
        .arg("-a")
        .arg("midle")
        .arg(format!("midle · {label}"))
        .arg(body)
        .arg("-t")
        .arg("2500")
        .spawn();
}
