//! Fire-and-forget child processes that never become zombies.
//!
//! `std::process::Child`'s `Drop` neither kills nor waits, so a shell that
//! does `Command::new(..).spawn()` and drops the handle keeps every exited
//! child as a `<defunct>` entry until mshell itself exits — the long-lived
//! shell collected them by the dozen (autostart `sh -c` wrappers, `mctl
//! reload`, `xdg-open`, …). [`SpawnReaped::spawn_reaped`] hands the child to
//! a small detached thread that blocks in `wait()`, so the exit status is
//! collected the moment the process ends, without blocking the caller (often
//! the GTK main thread) and without a polling wakeup.
//!
//! `tokio::process` children don't need this: tokio's runtime reaps dropped
//! children itself.

use std::io;
use std::process::{Child, Command};

/// Stack for a reaper thread: it only calls `waitpid`.
const REAPER_STACK: usize = 64 * 1024;

/// Wait for `child` on a detached thread so it is reaped as soon as it exits.
pub fn reap(mut child: Child) {
    let pid = child.id();
    let spawned = std::thread::Builder::new()
        .name(format!("reap-{pid}"))
        .stack_size(REAPER_STACK)
        .spawn(move || {
            let _ = child.wait();
        });
    if let Err(e) = spawned {
        // Thread creation failed (resource exhaustion): the child stays
        // unreaped until mshell exits — log it rather than block here.
        tracing::warn!(pid, error = %e, "process: could not start reaper thread");
    }
}

/// [`Command::spawn`] for fire-and-forget children: the child is reaped in
/// the background (see [`reap`]). Returns the child's pid.
pub trait SpawnReaped {
    fn spawn_reaped(&mut self) -> io::Result<u32>;
}

impl SpawnReaped for Command {
    fn spawn_reaped(&mut self) -> io::Result<u32> {
        let child = self.spawn()?;
        let pid = child.id();
        reap(child);
        Ok(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Process state letter from `/proc/<pid>/stat` (`Z` = zombie), or
    /// `None` once the pid is gone (reaped).
    fn proc_state(pid: u32) -> Option<char> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // Format: `pid (comm) S …` — comm may contain spaces/parens.
        stat.rsplit_once(") ")?.1.chars().next()
    }

    fn wait_until_gone(pid: u32) -> Option<char> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match proc_state(pid) {
                None => return None,
                Some(s) if Instant::now() >= deadline => return Some(s),
                Some(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    #[test]
    fn exited_child_is_reaped() {
        let pid = Command::new("true").spawn_reaped().expect("spawn true");
        assert_eq!(
            wait_until_gone(pid),
            None,
            "an exited child must not linger as a zombie"
        );
    }

    /// Control: a plain `spawn()` whose handle is dropped does leave a zombie
    /// — the bug this crate exists for.
    #[test]
    fn dropped_plain_child_stays_a_zombie() {
        let child = Command::new("true").spawn().expect("spawn true");
        let pid = child.id();
        drop(child);
        let deadline = Instant::now() + Duration::from_secs(5);
        while proc_state(pid) != Some('Z') && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(proc_state(pid), Some('Z'));
    }

    #[test]
    fn spawn_error_is_returned() {
        assert!(
            Command::new("/nonexistent/mshell-process-test")
                .spawn_reaped()
                .is_err()
        );
    }
}
