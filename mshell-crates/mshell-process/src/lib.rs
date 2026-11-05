//! Fire-and-forget child processes that never become zombies, and
//! long-lived ones that outlive an mshell restart ([`outliving_shell`]).
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
//!
//! [`identity`] finds the [`outliving_shell`] processes an earlier mshell
//! started, after a restart, by uid + exact scope name + argv + start time.

pub mod identity;

use std::ffi::{OsStr, OsString};
use std::io;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};

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

// ── Outliving mshell ────────────────────────────────────────────────────
//
// mshell runs as `mshell.service`; stopping or restarting it kills every
// process in the service's cgroup, and a plain `Command::spawn` child stays
// in that cgroup. Long-lived things the user started from the shell (the
// RNNoise filter, a resumed Claude terminal, a phone mirror window) would
// die with a shell restart. [`outliving_shell`] starts them through
// `systemd-run --user --scope`, which registers a transient
// `app-mshell-<name>-….scope` and then execs the program in place: same
// pid, still our child (so `kill` / `wait` keep working), but in its own
// cgroup, so stopping mshell.service leaves it running. This is what GIO
// does for launched `.desktop` apps.

/// Unique suffix for transient unit names.
static UNIT_SEQ: AtomicU32 = AtomicU32::new(0);

/// Wrap only when mshell is itself a systemd unit (`INVOCATION_ID` is set
/// for every unit's processes; a dev run from a terminal has no cgroup to
/// escape) and `systemd-run` exists.
fn should_wrap(invocation_id: Option<&OsStr>, systemd_run_on_path: bool) -> bool {
    invocation_id.is_some_and(|id| !id.is_empty()) && systemd_run_on_path
}

/// A transient unit name from a free-form label: systemd allows
/// `[A-Za-z0-9:_.-]`; anything else becomes `-`.
fn unit_name(label: &str, pid: u32, seq: u32) -> String {
    let label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("app-mshell-{label}-{pid}-{seq}")
}

/// `(program, args)` to run: `program args` itself, or wrapped in
/// `systemd-run --user --scope … -- program args`. Arguments pass through
/// untouched: `--expand-environment=no` stops systemd-run (≥ 254) from
/// expanding `$VAR` in them.
fn outliving_argv(
    program: &OsStr,
    args: &[OsString],
    unit: &str,
    wrap: bool,
) -> (OsString, Vec<OsString>) {
    if !wrap {
        return (program.to_owned(), args.to_vec());
    }
    let mut argv: Vec<OsString> = [
        "--user",
        "--scope",
        "--collect",
        "--quiet",
        "--expand-environment=no",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    argv.push(format!("--unit={unit}").into());
    argv.push("--".into());
    argv.push(program.to_owned());
    argv.extend(args.iter().cloned());
    ("systemd-run".into(), argv)
}

/// A [`Command`] for `program args` that survives an mshell restart (see
/// the section comment above). `label` names the transient scope
/// (`app-mshell-<label>-<pid>-<n>.scope`). Set stdio / env / cwd on the
/// result and spawn it as usual; the pid is the program's own.
pub fn outliving_shell<I, S>(program: impl AsRef<OsStr>, args: I, label: &str) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
    let wrap = should_wrap(
        std::env::var_os("INVOCATION_ID").as_deref(),
        margo_fs::path::on_path("systemd-run"),
    );
    let unit = unit_name(
        label,
        std::process::id(),
        UNIT_SEQ.fetch_add(1, Ordering::Relaxed),
    );
    let (program, args) = outliving_argv(program.as_ref(), &args, &unit, wrap);
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    /// The bug: the RNNoise filter, a resumed Claude terminal and scrcpy
    /// were spawned straight into mshell.service's cgroup and died with
    /// every shell restart.
    #[test]
    fn under_a_unit_the_program_runs_in_its_own_scope() {
        let (prog, args) = outliving_argv(
            OsStr::new("scrcpy"),
            &os(&["--serial=abc", "--window-title=$HOME phone"]),
            "scrcpy-1-0",
            true,
        );
        assert_eq!(prog, "systemd-run");
        assert_eq!(
            args,
            os(&[
                "--user",
                "--scope",
                "--collect",
                "--quiet",
                "--expand-environment=no",
                "--unit=scrcpy-1-0",
                "--",
                "scrcpy",
                "--serial=abc",
                "--window-title=$HOME phone",
            ])
        );
    }

    #[test]
    fn outside_a_unit_the_program_runs_as_is() {
        let (prog, args) = outliving_argv(OsStr::new("pipewire"), &os(&["-c", "x"]), "u", false);
        assert_eq!(prog, "pipewire");
        assert_eq!(args, os(&["-c", "x"]));
    }

    #[test]
    fn wraps_only_inside_a_systemd_unit_with_systemd_run() {
        assert!(should_wrap(Some(OsStr::new("4f2a")), true));
        assert!(!should_wrap(None, true));
        assert!(!should_wrap(Some(OsStr::new("")), true));
        assert!(!should_wrap(Some(OsStr::new("4f2a")), false));
    }

    #[test]
    fn unit_names_are_valid_and_unique() {
        assert_eq!(unit_name("rnnoise", 42, 0), "app-mshell-rnnoise-42-0");
        assert_eq!(
            unit_name("claude resume/x", 42, 3),
            "app-mshell-claude-resume-x-42-3"
        );
    }

    #[test]
    fn outside_a_unit_the_command_is_the_program() {
        // Test processes are not systemd units unless run under one.
        if std::env::var_os("INVOCATION_ID").is_none() {
            let cmd = outliving_shell("true", ["a"], "t");
            assert_eq!(cmd.get_program(), "true");
            assert_eq!(cmd.get_args().collect::<Vec<_>>(), vec![OsStr::new("a")]);
        }
    }
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
