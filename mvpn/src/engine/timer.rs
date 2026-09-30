//! Auto-switch timer: change to a random relay every N minutes.
//!
//! `start` spawns a detached `mvpn __timer-run <min>` child and records its PID;
//! `stop` kills it. The child loop lives in `run` (driven by the hidden CLI
//! subcommand) so the timer survives the launching shell.

use std::path::PathBuf;
use std::process::Command;

use super::actions;

fn pid_path() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(base).join("mvpn-timer.pid")
}

/// Spawn the detached switcher. Replaces any existing timer.
pub fn start(minutes: u64) -> std::io::Result<()> {
    stop();
    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .args(["__timer-run", &minutes.to_string()])
        .spawn()?;
    let _ = std::fs::write(pid_path(), child.id().to_string());
    Ok(())
}

/// `true` when `pid` still looks like our own detached `__timer-run`
/// child, checked via `/proc/<pid>/cmdline`. The pid file just holds a
/// bare number with nothing tying it back to us — if the timer process
/// has since died and the OS recycled that pid for something unrelated
/// (routine on Linux over time), `stop()` would otherwise SIGTERM an
/// arbitrary process owned by this user. `is_running()` uses the same
/// check so a stale pid file doesn't report a phantom running timer.
fn is_our_timer(pid: i32) -> bool {
    let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    // `cmdline` is NUL-separated argv; argv[1] is our subcommand name.
    cmdline
        .split(|&b| b == 0)
        .nth(1)
        .is_some_and(|arg| arg == b"__timer-run")
}

/// Kill the running timer, if any. Returns true if one was stopped.
pub fn stop() -> bool {
    let p = pid_path();
    let Ok(s) = std::fs::read_to_string(&p) else {
        return false;
    };
    let _ = std::fs::remove_file(&p);
    if let Ok(pid) = s.trim().parse::<i32>()
        && is_our_timer(pid)
    {
        // Best-effort SIGTERM via `kill` (no nix dep).
        return super::sys::ok("kill", &[&pid.to_string()]);
    }
    false
}

pub fn is_running() -> bool {
    std::fs::read_to_string(pid_path())
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .is_some_and(is_our_timer)
}

/// The child loop: every `minutes`, switch to a random relay. Runs until killed.
pub fn run(minutes: u64) -> ! {
    let dur = std::time::Duration::from_secs(minutes.max(1) * 60);
    loop {
        std::thread::sleep(dur);
        actions::random("", "", super::relays::Ownership::Any);
    }
}
