//! The daemon's signal-driven wait loop (phase B).
//!
//! Before this module the orchestrator blocked blindly in `waitpid` and slept
//! its crash backoff in `thread::sleep`, so a SIGTERM at the wrong moment
//! (`systemctl stop mlogind`, a reboot) killed the daemon mid-hold: no
//! destructor ran, and the VT stayed in `KD_GRAPHICS` with — now that
//! [`crate::vt_guard`] also suppresses the keyboard — its keys off. A console
//! that neither draws nor types is the lockout a login manager exists to
//! never cause.
//!
//! [`Events`] turns the daemon's fate signals into readable events: it blocks
//! `{SIGTERM, SIGINT, SIGHUP, SIGCHLD}` and every wait — for the runner, or
//! for a backoff delay on a timerfd — becomes a `poll` over a `signalfd`.
//! Termination is therefore always *observed*, at a point where the runner
//! can be stopped and the VT guard dropped in order.
//!
//! Mask hygiene is load-bearing: the blocked mask is inherited across fork
//! AND exec, so every forked runner resets it first thing
//! ([`reset_signal_mask`]) — otherwise the user's own session would run with
//! SIGTERM blocked. The TTY greeter path never sees any of this: `main` drops
//! the `Events` (restoring the old mask) before drawing it, so the classic
//! path keeps its classic signal behaviour.

use std::io;
use std::os::unix::io::AsRawFd;
use std::time::Duration;

use log::{error, info, warn};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, poll};
use nix::sys::signal::{SigSet, SigmaskHow, Signal, sigprocmask};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use nix::sys::time::TimeSpec;
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};

/// How the wait for a runner ended.
pub enum Wait {
    /// The runner exited with this code ([`crate::wait_for`]'s convention).
    Exited(i32),
    /// SIGTERM / SIGINT / SIGHUP: the machine wants the login manager gone.
    Terminated,
}

/// How a timed sleep ended.
pub enum Sleep {
    Elapsed,
    Terminated,
}

/// How long a SIGTERMed runner gets to die on its own before SIGKILL.
const TERM_GRACE: Duration = Duration::from_secs(5);

/// What [`Events::drain`] found queued on the signalfd.
#[derive(Default)]
struct Drained {
    term: bool,
    chld: bool,
}

pub struct Events {
    sfd: SignalFd,
    timer: TimerFd,
    /// The process mask from before [`Events::new`] blocked ours, restored on
    /// drop so the TTY greeter (and anything after us) keeps default signal
    /// delivery.
    old_mask: SigSet,
}

impl Events {
    /// Block the daemon's fate signals and open the fds the wait loop polls.
    ///
    /// Call before the first fork so no signal can slip through unhandled.
    /// Every forked runner must undo the inherited mask — see
    /// [`reset_signal_mask`].
    pub fn new() -> io::Result<Events> {
        let mask = Events::mask();
        let mut old_mask = SigSet::empty();
        sigprocmask(SigmaskHow::SIG_BLOCK, Some(&mask), Some(&mut old_mask)).map_err(io_error)?;

        let restore = |e: Errno| {
            let _ = sigprocmask(SigmaskHow::SIG_SETMASK, Some(&old_mask), None);
            io_error(e)
        };
        let sfd = SignalFd::with_flags(&mask, SfdFlags::SFD_CLOEXEC | SfdFlags::SFD_NONBLOCK)
            .map_err(restore)?;
        let timer =
            TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::TFD_CLOEXEC).map_err(restore)?;

        Ok(Events {
            sfd,
            timer,
            old_mask,
        })
    }

    fn mask() -> SigSet {
        let mut set = SigSet::empty();
        set.add(Signal::SIGTERM);
        set.add(Signal::SIGINT);
        set.add(Signal::SIGHUP);
        set.add(Signal::SIGCHLD);
        set
    }

    /// Wait until `pid` exits or a termination signal arrives.
    ///
    /// The `WNOHANG` reap runs *before* every poll: a SIGCHLD consumed by an
    /// earlier drain (they coalesce) can therefore never lose an exit.
    pub fn wait_child(&mut self, pid: libc::pid_t) -> Wait {
        loop {
            if let Some(code) = try_wait(pid) {
                return Wait::Exited(code);
            }
            let drained = self.drain();
            if drained.term {
                return Wait::Terminated;
            }
            if drained.chld {
                continue; // re-check try_wait at the loop head
            }
            if self.poll_fds(false).is_err() {
                // poll itself failing means the loop can no longer observe
                // anything; fall back to the pre-B blocking wait rather than
                // spinning.
                return Wait::Exited(crate::wait_for(pid));
            }
        }
    }

    /// Sleep for `dur`, or return early when a termination signal arrives.
    pub fn sleep(&mut self, dur: Duration) -> Sleep {
        if self
            .timer
            .set(
                Expiration::OneShot(TimeSpec::from(dur)),
                TimerSetTimeFlags::empty(),
            )
            .is_err()
        {
            // Degraded but correct: an uninterruptible backoff was the status
            // quo before phase B.
            std::thread::sleep(dur);
            return Sleep::Elapsed;
        }
        loop {
            if self.drain().term {
                let _ = self.timer.unset();
                return Sleep::Terminated;
            }
            match self.poll_fds(true) {
                Ok(timer_fired) if timer_fired => {
                    let _ = self.timer.wait(); // clear the expiration
                    return Sleep::Elapsed;
                }
                Ok(_) => continue, // a signal woke us; drain at the loop head
                Err(_) => {
                    let _ = self.timer.unset();
                    std::thread::sleep(dur);
                    return Sleep::Elapsed;
                }
            }
        }
    }

    /// The shutdown path: SIGTERM the runner, give it [`TERM_GRACE`], then
    /// SIGKILL. Returns the runner's exit code.
    ///
    /// Runs while termination is already decided, so queued signals are
    /// drained and discarded — nothing outranks the teardown.
    pub fn terminate_child(&mut self, pid: libc::pid_t) -> i32 {
        // SAFETY: plain kill(2); ESRCH (the runner beat us to death's door)
        // is fine — try_wait below still reaps the zombie.
        unsafe { libc::kill(pid, libc::SIGTERM) };

        if self
            .timer
            .set(
                Expiration::OneShot(TimeSpec::from(TERM_GRACE)),
                TimerSetTimeFlags::empty(),
            )
            .is_err()
        {
            // No timer, no patience: a runner normally dies on TERM well
            // before a human notices, and shutdown must not hang forever.
            return crate::wait_for(pid);
        }
        loop {
            if let Some(code) = try_wait(pid) {
                let _ = self.timer.unset();
                return code;
            }
            self.drain();
            match self.poll_fds(true) {
                Ok(timer_fired) if timer_fired => {
                    let _ = self.timer.wait();
                    warn!("daemon: runner {pid} ignored SIGTERM for {TERM_GRACE:?}; killing it");
                    // SAFETY: plain kill(2), as above.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                    return crate::wait_for(pid);
                }
                Ok(_) => continue,
                Err(_) => return crate::wait_for(pid),
            }
        }
    }

    /// Read every signal queued on the signalfd.
    fn drain(&mut self) -> Drained {
        let mut drained = Drained::default();
        loop {
            match self.sfd.read_signal() {
                Ok(Some(si)) => match si.ssi_signo as libc::c_int {
                    libc::SIGCHLD => drained.chld = true,
                    libc::SIGTERM | libc::SIGINT | libc::SIGHUP => {
                        info!("daemon: received termination signal {}", si.ssi_signo);
                        drained.term = true;
                    }
                    other => warn!("daemon: unexpected signal {other} on the signalfd"),
                },
                Ok(None) => return drained, // SFD_NONBLOCK: queue empty
                Err(Errno::EINTR) => continue,
                Err(err) => {
                    error!("daemon: signalfd read failed: {err}");
                    return drained;
                }
            }
        }
    }

    /// Block until the signalfd — and, when `with_timer`, the timerfd — is
    /// readable. Returns whether the *timer* is the readable one.
    fn poll_fds(&self, with_timer: bool) -> io::Result<bool> {
        let mut fds = [
            PollFd::new(self.sfd.as_raw_fd(), PollFlags::POLLIN),
            PollFd::new(self.timer.as_raw_fd(), PollFlags::POLLIN),
        ];
        let watched = if with_timer { 2 } else { 1 };
        loop {
            match poll(&mut fds[..watched], -1) {
                Ok(_) => {
                    let timer_fired = with_timer
                        && fds[1]
                            .revents()
                            .map(|r| r.contains(PollFlags::POLLIN))
                            .unwrap_or(false);
                    return Ok(timer_fired);
                }
                Err(Errno::EINTR) => continue,
                Err(err) => {
                    error!("daemon: poll failed: {err}");
                    return Err(io_error(err));
                }
            }
        }
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        // Hand back the mask we found. `main` relies on this running before
        // the TTY greeter draws, so a plain SIGTERM keeps killing the TUI
        // daemon the way it always has.
        let _ = sigprocmask(SigmaskHow::SIG_SETMASK, Some(&self.old_mask), None);
    }
}

/// Undo the daemon's blocked mask in a forked child.
///
/// The mask survives fork *and* exec; without this reset the session
/// compositor itself would run with SIGTERM blocked — a desktop no
/// `systemctl` could ever stop. The runner calls it before anything else.
/// Best-effort: a child that cannot touch its mask is no worse off than
/// before phase B.
pub fn reset_signal_mask() {
    let _ = sigprocmask(SigmaskHow::SIG_SETMASK, Some(&SigSet::empty()), None);
}

/// Reduce a `waitpid` status to an exit code: a runner killed by a signal
/// reports `128 + signo`, the shell convention, so it can never collide with
/// one of the runner's own codes.
pub(crate) fn decode_wait_status(status: libc::c_int) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        crate::runner::EXIT_SESSION_FAILED
    }
}

/// Reap `pid` without blocking. `None` while it still runs.
fn try_wait(pid: libc::pid_t) -> Option<i32> {
    let mut status: libc::c_int = 0;
    // SAFETY: `status` is a valid out-pointer; `pid` is our direct child.
    // WNOHANG never blocks, so EINTR cannot occur.
    let rc = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    if rc == 0 {
        return None;
    }
    if rc < 0 {
        // ECHILD and friends: the child is unaccounted for. Report a failure
        // rather than waiting forever on a pid that will never turn up.
        error!(
            "daemon: waitpid({pid}) failed: {}",
            io::Error::last_os_error()
        );
        return Some(crate::runner::EXIT_SESSION_FAILED);
    }
    Some(decode_wait_status(status))
}

fn io_error(errno: Errno) -> io::Error {
    io::Error::from_raw_os_error(errno as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Deliberately no `Events::new()` test: it mutates the *process* signal
    // mask, and under cargo's threaded test runner a process-directed SIGTERM
    // landing on an unmasked sibling thread would kill the whole harness.
    // The timerfd-through-poll path is the same one `sleep`/`terminate_child`
    // ride, and it is safe to pin here.
    #[test]
    fn an_armed_timerfd_becomes_readable_through_poll() {
        let timer = TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::TFD_CLOEXEC).unwrap();
        timer
            .set(
                Expiration::OneShot(TimeSpec::from(Duration::from_millis(5))),
                TimerSetTimeFlags::empty(),
            )
            .unwrap();

        let mut fds = [PollFd::new(timer.as_raw_fd(), PollFlags::POLLIN)];
        let ready = poll(&mut fds, 1000).unwrap();
        assert_eq!(ready, 1, "the timer never fired");
        timer.wait().unwrap(); // clears the expiration without blocking
    }
}
