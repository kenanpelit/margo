//! The GUI greeter's startup command: launch mgreet with the greeter socket,
//! everything around it without.
//!
//! margo hands the runner's PAM conversation socket (`$MLOGIND_SOCK_FD`) to
//! the one command it is told to start and to nothing else it spawns. That
//! command used to be a shell line, `mkeys & …; mgreet; kill …; mctl dispatch
//! quit`, so the shell and every program it ran inherited the socket that
//! carries PAM's prompts and the typed password: the on-screen keyboard for as
//! long as it floated over the card, and `mctl`. A shell cannot close a
//! descriptor for some children and not others without knowing its number,
//! and multi-digit fd redirections are not portable `sh`.
//!
//! So the startup command is this binary instead, run as the greeter user:
//!
//! ```text
//! margo ──(fd inheritable in this child only)──▶ mlogind __greeter-startup
//!   adopt the fd, close-on-exec
//!   spawn mkeys                          (no fd, no MLOGIND_SOCK_FD)
//!   spawn mgreet  pre_exec: clear CLOEXEC on the fd, MLOGIND_SOCK_FD=<n>
//!   close our copy
//!   wait mgreet → SIGTERM mkeys, reap it → mctl dispatch quit (no fd)
//! ```
//!
//! Dropping our copy right after the spawn keeps the runner's EOF tied to
//! mgreet alone, and stopping the OSK before the quit keeps it from
//! repainting over a session that is trying to start.
//!
//! It runs before any config is parsed or log opened (like the theme-sync
//! reader): there is nothing here to configure, and the greeter user cannot
//! write mlogind's logs. Errors go to stderr, which margo's log captures.

use std::ffi::OsString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// `argv[1]` that turns mlogind into the greeter launcher. Followed by the
/// mgreet path, the mctl path and, with an on-screen keyboard, the mkeys path.
pub const ARG: &str = "__greeter-startup";

/// The variable the socket's number travels in, runner → margo → us → mgreet.
pub const SOCK_ENV: &str = "MLOGIND_SOCK_FD";

const EXIT_BAD_ARGS: i32 = 64;
const EXIT_NO_GREETER: i32 = 69;

/// How long the OSK gets to exit on SIGTERM before it is killed.
const OSK_GRACE: Duration = Duration::from_secs(1);

/// What to run, parsed from the arguments after [`ARG`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub mgreet: PathBuf,
    pub mctl: PathBuf,
    pub mkeys: Option<PathBuf>,
}

impl Plan {
    pub(crate) fn parse(args: &[OsString]) -> Option<Plan> {
        match args {
            [mgreet, mctl] => Some(Plan {
                mgreet: mgreet.into(),
                mctl: mctl.into(),
                mkeys: None,
            }),
            [mgreet, mctl, mkeys] => Some(Plan {
                mgreet: mgreet.into(),
                mctl: mctl.into(),
                mkeys: Some(mkeys.into()),
            }),
            _ => None,
        }
    }
}

/// Adopt the descriptor named by `value` (a decimal fd number) and mark it
/// close-on-exec, so nothing this process spawns inherits it by accident.
/// `None` for a missing or bogus value, a std stream, or a closed fd.
fn adopt(value: Option<&str>) -> Option<OwnedFd> {
    let raw: RawFd = value?.trim().parse().ok()?;
    if raw <= 2 {
        return None;
    }
    // SAFETY: F_GETFD / F_SETFD only read and set the descriptor's flags.
    let flags = unsafe { libc::fcntl(raw, libc::F_GETFD) };
    if flags < 0 {
        return None;
    }
    // SAFETY: as above.
    if unsafe { libc::fcntl(raw, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return None;
    }
    // SAFETY: margo handed us this open descriptor for exactly this purpose;
    // nothing else in this process refers to it.
    Some(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// A child that must not see the socket: no inherited copy (ours is
/// close-on-exec) and no number in its environment.
fn without_socket(program: &PathBuf) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_remove(SOCK_ENV);
    cmd
}

/// Stop the OSK: SIGTERM, a short grace, then SIGKILL, and reap it either way.
fn stop(mut child: Child) {
    // A pid_t always fits: the kernel hands them out below 2^22.
    let pid = child.id() as libc::pid_t;
    // SAFETY: signalling our own, not yet reaped, child.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let deadline = Instant::now() + OSK_GRACE;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Run the greeter as [`Plan`] says; returns mgreet's exit code.
pub(crate) fn run(plan: &Plan, sock: Option<OwnedFd>) -> i32 {
    let osk = plan.mkeys.as_ref().and_then(|mkeys| {
        without_socket(mkeys)
            .spawn()
            .map_err(|err| eprintln!("mlogind: cannot start {}: {err}", mkeys.display()))
            .ok()
    });

    let mut greeter = Command::new(&plan.mgreet);
    match sock.as_ref() {
        Some(fd) => {
            let raw = fd.as_raw_fd();
            greeter.env(SOCK_ENV, raw.to_string());
            // SAFETY: the hook makes one fcntl call on a descriptor number.
            unsafe { greeter.pre_exec(move || crate::runner::inherit_across_exec(raw)) };
        }
        // Nobody orchestrating: mgreet runs as a UI dry-run.
        None => {
            greeter.env_remove(SOCK_ENV);
        }
    }
    let spawned = greeter.spawn();
    // mgreet has its copy now (or never will). Ours goes, so the runner's EOF
    // means mgreet exited.
    drop(sock);
    let code = match spawned {
        Ok(mut child) => match child.wait() {
            Ok(status) => status.code().unwrap_or(1),
            Err(err) => {
                eprintln!("mlogind: waiting for mgreet: {err}");
                1
            }
        },
        Err(err) => {
            eprintln!("mlogind: cannot start {}: {err}", plan.mgreet.display());
            EXIT_NO_GREETER
        }
    };

    if let Some(osk) = osk {
        stop(osk);
    }
    // However mgreet ended, quit margo so the runner's greeter host returns.
    if let Err(err) = without_socket(&plan.mctl)
        .args(["dispatch", "quit"])
        .status()
    {
        eprintln!("mlogind: cannot run {}: {err}", plan.mctl.display());
    }
    code
}

/// The `__greeter-startup` entry point: `args` are what follows [`ARG`].
pub fn main(args: &[OsString]) -> i32 {
    let Some(plan) = Plan::parse(args) else {
        eprintln!("usage: mlogind {ARG} <mgreet> <mctl> [<mkeys>]");
        return EXIT_BAD_ARGS;
    };
    let sock = adopt(std::env::var(SOCK_ENV).ok().as_deref());
    run(&plan, sock)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_plan_takes_two_or_three_paths() {
        assert_eq!(
            Plan::parse(&os(&["/usr/bin/mgreet", "/usr/bin/mctl"])),
            Some(Plan {
                mgreet: "/usr/bin/mgreet".into(),
                mctl: "/usr/bin/mctl".into(),
                mkeys: None,
            })
        );
        assert_eq!(
            Plan::parse(&os(&["/g", "/c", "/k"])).and_then(|p| p.mkeys),
            Some(PathBuf::from("/k"))
        );
        assert_eq!(Plan::parse(&os(&["/g"])), None);
        assert_eq!(Plan::parse(&os(&["/g", "/c", "/k", "/x"])), None);
    }

    #[test]
    fn a_missing_or_bogus_handoff_is_not_adopted() {
        assert!(adopt(None).is_none());
        assert!(adopt(Some("")).is_none());
        assert!(adopt(Some("two")).is_none());
        // A std stream is never the socket.
        assert!(adopt(Some("1")).is_none());
    }

    #[test]
    fn an_adopted_socket_is_close_on_exec() {
        let (_a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let fd: OwnedFd = b.into();
        // SAFETY: clearing the flags on a descriptor we own.
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, 0) };
        let raw = fd.as_raw_fd();
        // adopt takes ownership below; forget ours first.
        std::mem::forget(fd);
        let fd = adopt(Some(&raw.to_string())).unwrap();
        // SAFETY: F_GETFD only reads the flags.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
    }
}
