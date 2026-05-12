//! `mshell restart` — graceful self-respawn.
//!
//! margo starts mshell via `exec-once`, so when the bar dies the
//! compositor doesn't bring it back. This subcommand replaces the
//! `pkill mshell && setsid -f mshell …` incantation: scan /proc for
//! sibling mshell processes (excluding self), SIGTERM them, poll
//! until they're gone with a 3 s budget, give the compositor 200 ms
//! to tear down the bar's layer surfaces, then spawn a detached
//! fresh instance via setsid().

use anyhow::{Context, Result};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SETTLE_AFTER_DEATH: Duration = Duration::from_millis(200);
const GRACEFUL_KILL_TIMEOUT: Duration = Duration::from_secs(3);

pub fn run() -> Result<()> {
    let siblings = find_sibling_mshell_pids();
    if !siblings.is_empty() {
        eprintln!(
            "mshell restart: stopping {} running instance(s)…",
            siblings.len()
        );
        for pid in &siblings {
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
                "mshell restart: {} instance(s) didn't exit in {}s, escalating to SIGKILL",
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
