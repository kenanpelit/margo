//! `mshell restart` — graceful self-respawn.
//!
//! margo's autostart line is `exec-once = mshell`, so when the bar
//! dies the compositor does not bring it back. This subcommand fills
//! that gap: scans `/proc` for sibling `mshell` processes (excluding
//! the one running this code), SIGTERMs them, waits up to ~3s for
//! the surfaces to tear down, then spawns a fresh detached instance.
//! A SIGKILL fallback keeps it bounded if a process refuses to exit.

use anyhow::{Context, Result};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Settle time after the last sibling dies before we exec the
/// replacement — gives the compositor a beat to drop the bar's
/// layer surfaces so the new instance doesn't race over them.
const SETTLE_AFTER_DEATH: Duration = Duration::from_millis(200);

/// Upper bound for waiting on graceful SIGTERM shutdown before we
/// escalate to SIGKILL. mshell's normal exit path is fast (< 500ms);
/// 3s leaves plenty of headroom for slow shutdowns without making
/// the user feel like the command hung.
const GRACEFUL_KILL_TIMEOUT: Duration = Duration::from_secs(3);

pub fn run() -> Result<()> {
    let siblings = find_sibling_mshell_pids();
    if !siblings.is_empty() {
        eprintln!("mshell restart: stopping {} running instance(s)…", siblings.len());
        for pid in &siblings {
            // Best-effort SIGTERM; missing target (ESRCH) just means
            // the process already exited between scan and signal.
            unsafe { libc::kill(*pid, libc::SIGTERM) };
        }
        wait_for_exit(&siblings);
        std::thread::sleep(SETTLE_AFTER_DEATH);
    }

    let exe = std::env::current_exe().context("locating mshell binary")?;
    let mut cmd = Command::new(&exe);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Drop the controlling terminal and process group: the new
    // mshell must survive even if the user's terminal closes right
    // after invoking `mshell restart`. setsid() creates a fresh
    // session with no controlling tty.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.spawn().context("spawning new mshell instance")?;
    eprintln!("mshell restart: new instance launched");
    Ok(())
}

/// Scan `/proc/<pid>/comm` for processes named exactly `mshell`,
/// excluding the calling process. Linux truncates `comm` to 15
/// bytes, so the literal `mshell` fits with room to spare.
fn find_sibling_mshell_pids() -> Vec<i32> {
    let self_pid = std::process::id() as i32;
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        let Ok(pid) = name_str.parse::<i32>() else { continue };
        if pid == self_pid {
            continue;
        }
        let comm_path = entry.path().join("comm");
        if let Ok(comm) = std::fs::read_to_string(&comm_path)
            && comm.trim() == "mshell"
        {
            out.push(pid);
        }
    }
    out
}

/// Poll the given pids with `kill(pid, 0)` (probes for liveness
/// without actually signalling) until none remain or the deadline
/// passes. Anything still alive at the deadline gets SIGKILLed.
fn wait_for_exit(pids: &[i32]) {
    let deadline = Instant::now() + GRACEFUL_KILL_TIMEOUT;
    loop {
        let alive: Vec<i32> = pids
            .iter()
            .copied()
            .filter(|&pid| unsafe { libc::kill(pid, 0) } == 0)
            .collect();
        if alive.is_empty() {
            return;
        }
        if Instant::now() >= deadline {
            eprintln!(
                "mshell restart: {} instance(s) did not exit in {}s, escalating to SIGKILL",
                alive.len(),
                GRACEFUL_KILL_TIMEOUT.as_secs()
            );
            for pid in alive {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
