//! start-margo — supervisor / watchdog launcher for the margo
//! Wayland compositor.
//!
//! Inspired by Hyprland's `start-hyprland` C++ launcher, but built as a
//! fully **event-driven** supervisor (`poll(2)` over a `signalfd`, the
//! child's `pidfd`, and margo's readiness pipe — no busy-poll, zero idle
//! CPU). On top of that it adds five concrete improvements:
//!
//!   1. **Crash budget + backoff.** A rolling time window caps abnormal
//!      exits; once exceeded, the supervisor exits and the session
//!      returns to the display-manager (optionally after one *safe-mode*
//!      attempt, see `--safe-config`). Between restarts the delay backs
//!      off exponentially so a config that crashes margo on every start
//!      can't pin a CPU (start-hyprland respawns indefinitely).
//!   2. **systemd readiness + watchdog.** Waits for margo's readiness
//!      pipe (socket + backend + environment are ready) before emitting
//!      `READY=1`, and — when the unit sets `WatchdogSec=` — forwards
//!      `WATCHDOG=1` keep-alives driven by a heartbeat margo writes from
//!      its own event loop. That means systemd can detect and recover a
//!      *hung* compositor, not just a crashed one. Emits `STOPPING=1` on
//!      shutdown.
//!   3. **Race-free, signal-preserving forwarding.** SIGTERM / SIGINT /
//!      SIGHUP are blocked process-wide and drained from a `signalfd`, so
//!      a signal that arrives in the tiny window *before* the child is
//!      spawned is still delivered to it — with the *original* signal
//!      preserved — so margo always runs its own graceful teardown
//!      (surface destruction, ext-session-lock cleanup, `session.json`
//!      snapshot). start-hyprland only sends SIGTERM, and races the spawn.
//!   4. **A small readiness protocol.** margo speaks sd_notify-style
//!      lines over the readiness pipe (`READY=1`, `WATCHDOG=1`,
//!      `STATUS=…`, `FATAL=1`); start-margo forwards them to systemd and
//!      treats `FATAL=1` as "don't restart" so an unrecoverable init
//!      failure returns to the DM immediately instead of burning the
//!      crash budget.
//!   5. **Durable logging.** Diagnostics go through the shared
//!      `margo-logging` file sink (the same `~/.local/state/margo/logs`
//!      the compositor and shell use), so the reason a session bounced
//!      back to the DM survives even when there is no systemd journal.
//!
//! Shared with start-hyprland:
//!   * `PR_SET_PDEATHSIG(SIGKILL)` — if the supervisor dies, the
//!     kernel kills the compositor too. No orphaned margo processes
//!     after a `kill -9 start-margo`.
//!   * `--path` flag — override the margo binary path for dev/staging
//!     builds.
//!   * `--` passthrough — anything after `--` is forwarded verbatim
//!     to margo (so `start-margo -- -c /etc/margo/special.conf` is
//!     the canonical way to point at a non-default config).
//!
//! Single binary, single source file. Inherits the workspace's tracing
//! stack — control verbosity with `START_MARGO_LOG=debug` (falls back to
//! `MARGO_LOG` if the start-specific filter isn't set).

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode, ExitStatus};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use margo_logging::LogHandle;
use tracing::{debug, error, info, warn};

/// Keep the logger alive for the whole process (the file sink stops
/// writing if the handle is dropped).
static LOG_HANDLE: OnceLock<LogHandle> = OnceLock::new();

/// Signals we intercept and forward to the child compositor.
const FORWARDED_SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

// ── Exit codes ────────────────────────────────────────────────────────────
// Distinct, documented codes for start-margo's *own* terminal conditions so a
// supervising unit / `systemctl status` can tell them apart from whatever margo
// itself returned. We deliberately use the sysexits.h range to avoid colliding
// with margo's small exit codes.
/// `mctl check-config` rejected the config (sysexits `EX_CONFIG`).
const EXIT_CONFIG_PREFLIGHT: i32 = 78;
/// The crash budget was exhausted — margo kept dying (sysexits `EX_UNAVAILABLE`).
const EXIT_CRASH_BUDGET: i32 = 69;
/// The margo binary could not be spawned (POSIX "command not executable").
const EXIT_SPAWN_FAILED: i32 = 127;

/// Watchdog supervisor for the margo Wayland compositor.
#[derive(Parser, Debug)]
#[command(
    version,
    about = "Watchdog supervisor for the margo Wayland compositor",
    long_about = "Starts margo, waits for it to exit, and — if the exit was abnormal — \
                  restarts it within the crash budget (with exponential backoff). Forwards \
                  SIGTERM/SIGINT/SIGHUP to the child race-free, emits sd_notify READY=1 only \
                  after margo signals real readiness, forwards WATCHDOG=1 keep-alives when the \
                  unit sets WatchdogSec=, and pins the child to die with the supervisor via \
                  PR_SET_PDEATHSIG."
)]
struct Args {
    /// Path to the margo binary. Defaults to `margo` resolved through PATH.
    #[arg(long, value_name = "PATH")]
    path: Option<PathBuf>,

    /// Hard cap on abnormal exits inside the rolling crash window.
    /// After this many crashes in `--restart-window-secs`, the
    /// supervisor exits non-zero and the session returns to the DM.
    #[arg(long, alias = "max-restarts", value_name = "N", default_value = "3")]
    max_crashes: u32,

    /// Length of the rolling crash window (seconds) used by
    /// `--max-restarts`.
    #[arg(long, value_name = "SECONDS", default_value = "60")]
    restart_window_secs: u64,

    /// One-shot mode — disable automatic restart on crash. Useful
    /// when debugging an individual session.
    #[arg(long)]
    no_restart: bool,

    /// Validate the margo config with `mctl check-config` before spawning.
    /// Enabled by default; pass this when testing a broken config path or a
    /// partial install without `mctl`.
    #[arg(long)]
    no_preflight: bool,

    /// Last-resort config to fall back to when the crash budget is exhausted.
    /// Instead of dropping straight back to the display-manager, start-margo
    /// makes one more attempt with `margo -c <PATH>` (a known-good config),
    /// resetting the crash window for that attempt. If margo still can't stay
    /// up, the supervisor finally gives up. Off by default.
    #[arg(long, value_name = "PATH")]
    safe_config: Option<PathBuf>,

    /// How long to wait for margo's real readiness signal before telling
    /// systemd it is up anyway. The child writes this signal after its
    /// Wayland socket, backend, XWayland/portal setup and environment import
    /// are ready. 0 disables the fallback and waits indefinitely.
    #[arg(long, value_name = "SECONDS", default_value = "20")]
    ready_timeout_secs: u64,

    /// How long to wait after forwarding SIGTERM/SIGINT/SIGHUP before
    /// escalating the child compositor to SIGKILL.
    #[arg(long, value_name = "SECONDS", default_value = "5")]
    shutdown_timeout_secs: u64,

    /// Skip sd_notify emission even when `NOTIFY_SOCKET` is set
    /// (the env-var is the normal signal). Mostly for tests.
    #[arg(long)]
    no_notify: bool,

    /// Arguments forwarded verbatim to margo. Place them after `--`:
    ///
    ///   start-margo -- --config /etc/margo/special.conf --no-xwayland
    #[arg(last = true, value_name = "MARGO_ARGS")]
    margo_args: Vec<OsString>,
}

// ── sd_notify ───────────────────────────────────────────────────────────────

/// Send a single sd_notify(3)-formatted message to `$NOTIFY_SOCKET`.
/// No-op when the env-var is absent or `--no-notify` was passed.
fn sd_notify(state: &str, suppress: bool) {
    if suppress {
        return;
    }
    let Some(socket_path) = std::env::var_os("NOTIFY_SOCKET") else {
        return;
    };
    if let Err(e) = send_notify_message(socket_path.as_os_str(), state) {
        warn!(state, ?socket_path, "sd_notify failed: {e}");
    }
}

fn notify_sockaddr(socket_path: &OsStr) -> io::Result<(libc::sockaddr_un, libc::socklen_t)> {
    let bytes = socket_path.as_bytes();
    if bytes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty NOTIFY_SOCKET",
        ));
    }

    // SAFETY: all-zero is a valid initial sockaddr_un before we fill family
    // and path bytes.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let offset = (&addr.sun_path as *const _ as usize) - (&addr as *const _ as usize);

    let len = if bytes[0] == b'@' {
        let name = &bytes[1..];
        if name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty abstract NOTIFY_SOCKET",
            ));
        }
        if name.len() + 1 > addr.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "abstract NOTIFY_SOCKET too long",
            ));
        }
        addr.sun_path[0] = 0;
        for (dst, src) in addr.sun_path[1..].iter_mut().zip(name) {
            *dst = *src as libc::c_char;
        }
        offset + 1 + name.len()
    } else {
        if bytes.len() + 1 > addr.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "NOTIFY_SOCKET path too long",
            ));
        }
        for (dst, src) in addr.sun_path.iter_mut().zip(bytes) {
            *dst = *src as libc::c_char;
        }
        offset + bytes.len() + 1
    };

    Ok((addr, len as libc::socklen_t))
}

fn send_notify_message(socket_path: &OsStr, state: &str) -> io::Result<()> {
    let (addr, len) = notify_sockaddr(socket_path)?;
    // SAFETY: socket(2) returns a new fd or -1 with errno set.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: fd is valid, addr points at a sockaddr_un initialised by
    // notify_sockaddr, and state.as_ptr/len describe a live byte slice.
    let sent = unsafe {
        libc::sendto(
            fd,
            state.as_ptr().cast(),
            state.len(),
            0,
            (&addr as *const libc::sockaddr_un).cast(),
            len,
        )
    };
    let result = if sent < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    };
    // SAFETY: fd was returned by socket above. Close errors are not useful
    // here; sendto's result is the observable notification outcome.
    unsafe {
        libc::close(fd);
    }
    result
}

// ── Signal handling (signalfd) ──────────────────────────────────────────────

/// Block the forwarded signals process-wide and return the set. Blocking is
/// what lets us drain them from a `signalfd` in the poll loop instead of an
/// async-signal-safe handler, and it means a signal that lands *before* the
/// child is spawned stays queued and is forwarded to the child once it exists.
fn block_forwarded_signals() -> Result<libc::sigset_t> {
    // SAFETY: called once at startup while single-threaded. sigemptyset /
    // sigaddset / pthread_sigmask only touch the provided set.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for sig in FORWARDED_SIGNALS {
            libc::sigaddset(&mut set, sig);
        }
        if libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) != 0 {
            anyhow::bail!("pthread_sigmask(SIG_BLOCK): {}", io::Error::last_os_error());
        }
        Ok(set)
    }
}

/// Create a non-blocking, close-on-exec `signalfd` for `set`.
fn create_signalfd(set: &libc::sigset_t) -> io::Result<OwnedFd> {
    // SAFETY: signalfd(2) returns a new fd or -1 with errno set; set is a
    // valid sigset_t built by block_forwarded_signals.
    let fd = unsafe { libc::signalfd(-1, set, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is freshly returned by signalfd and uniquely owned here.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Drain all pending signals from the `signalfd`, returning their numbers.
fn drain_signalfd(fd: RawFd) -> io::Result<Vec<libc::c_int>> {
    let mut sigs = Vec::new();
    let size = std::mem::size_of::<libc::signalfd_siginfo>();
    loop {
        // SAFETY: si is zero-initialised and large enough; read writes at most
        // `size` bytes into it.
        let mut si: libc::signalfd_siginfo = unsafe { std::mem::zeroed() };
        let n = unsafe { libc::read(fd, (&mut si as *mut libc::signalfd_siginfo).cast(), size) };
        if n < 0 {
            let e = io::Error::last_os_error();
            match e.kind() {
                io::ErrorKind::WouldBlock => return Ok(sigs),
                io::ErrorKind::Interrupted => continue,
                _ => return Err(e),
            }
        }
        if n == 0 {
            return Ok(sigs);
        }
        sigs.push(si.ssi_signo as libc::c_int);
    }
}

/// Send a signal to a pid. Best-effort: a dead child is not an error here.
fn send_signal(pid: i32, sig: libc::c_int) {
    if pid > 0 {
        // SAFETY: kill(2) with a pid we own. ESRCH (already gone) is fine.
        unsafe {
            libc::kill(pid as libc::pid_t, sig);
        }
    }
}

// ── pidfd ───────────────────────────────────────────────────────────────────

/// Open a `pidfd` for `pid` so the poll loop can wait on the child's exit as a
/// regular pollable fd. The child is unreaped (a zombie at worst), so the pid is
/// still valid and won't be recycled.
fn pidfd_open(pid: libc::pid_t) -> io::Result<OwnedFd> {
    // SAFETY: pidfd_open is a thin syscall wrapper; it returns a new fd or -1.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a freshly returned pidfd, uniquely owned here.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
}

// ── readiness pipe + protocol ───────────────────────────────────────────────

/// One sd_notify-style line margo can write over the readiness pipe.
#[derive(Debug, PartialEq, Eq)]
enum ProtoLine {
    Ready,
    Watchdog,
    Status(String),
    /// An unrecoverable init failure — start-margo must not restart.
    Fatal,
    /// Anything else; logged at debug and otherwise ignored.
    Other(String),
}

/// Parse a single readiness-pipe line into a [`ProtoLine`].
fn parse_proto_line(line: &str) -> ProtoLine {
    let line = line.trim();
    match line {
        "READY=1" => ProtoLine::Ready,
        "WATCHDOG=1" => ProtoLine::Watchdog,
        "FATAL=1" => ProtoLine::Fatal,
        _ if line.starts_with("STATUS=") => ProtoLine::Status(line["STATUS=".len()..].to_string()),
        // `ERRNO=<nonzero>` is sd_notify's "the service failed" convention.
        _ if line.starts_with("ERRNO=") && line != "ERRNO=0" => ProtoLine::Fatal,
        _ => ProtoLine::Other(line.to_string()),
    }
}

/// Non-blocking, line-buffered reader over the readiness pipe's read end.
struct ProtoReader {
    fd: OwnedFd,
    buf: Vec<u8>,
    eof: bool,
}

impl ProtoReader {
    fn new(fd: OwnedFd) -> Self {
        Self {
            fd,
            buf: Vec::new(),
            eof: false,
        }
    }

    fn raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Drain everything currently readable and return the complete lines.
    /// Sets `eof` when margo closes the write end (it does so on exit, or
    /// right after `READY=1` when no heartbeat was requested).
    fn drain_lines(&mut self) -> io::Result<Vec<ProtoLine>> {
        let mut chunk = [0u8; 256];
        loop {
            // SAFETY: read into a stack buffer we own; fd is the pipe read end.
            let n =
                unsafe { libc::read(self.fd.as_raw_fd(), chunk.as_mut_ptr().cast(), chunk.len()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                match e.kind() {
                    io::ErrorKind::WouldBlock => break,
                    io::ErrorKind::Interrupted => continue,
                    _ => return Err(e),
                }
            }
            if n == 0 {
                self.eof = true;
                break;
            }
            self.buf.extend_from_slice(&chunk[..n as usize]);
        }

        let mut lines = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&raw[..raw.len() - 1]);
            let text = text.trim();
            if !text.is_empty() {
                lines.push(parse_proto_line(text));
            }
        }
        Ok(lines)
    }
}

/// Create the readiness pipe. Returns the reader (read end, CLOEXEC +
/// non-blocking) and the raw write end (inheritable by the child).
fn ready_pipe() -> io::Result<(ProtoReader, RawFd)> {
    let mut fds = [0; 2];
    // SAFETY: pipe2 fills both fd slots or returns -1 with errno set. The read
    // end is CLOEXEC + non-blocking; the write end is left inheritable so the
    // child (margo) can write to it after exec.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // Clear the flags on the write end only: the child must inherit a plain,
    // blocking, non-CLOEXEC fd.
    if let Err(e) = clear_write_end_flags(fds[1]) {
        // SAFETY: both fds came from pipe2 above.
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
        return Err(e);
    }
    // SAFETY: fds[0] is uniquely owned by this OwnedFd from now on.
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    Ok((ProtoReader::new(read_fd), fds[1]))
}

/// Strip O_CLOEXEC and O_NONBLOCK from the pipe write end so the child inherits
/// a clean blocking fd.
fn clear_write_end_flags(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl(F_GETFD/F_SETFD/F_GETFL/F_SETFL) does not take ownership.
    let fdflags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if fdflags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, fdflags & !libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let flflags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flflags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flflags & !libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

// ── systemd watchdog ─────────────────────────────────────────────────────────

/// The systemd watchdog interval in microseconds, if this process is the one
/// systemd expects keep-alives from (`WATCHDOG_USEC` set, and `WATCHDOG_PID`
/// unset or equal to our pid). Returns `None` when no watchdog is configured.
fn watchdog_usec() -> Option<u64> {
    let usec: u64 = std::env::var("WATCHDOG_USEC").ok()?.trim().parse().ok()?;
    if usec == 0 {
        return None;
    }
    if let Ok(pid) = std::env::var("WATCHDOG_PID")
        && let Ok(pid) = pid.trim().parse::<i32>()
        && pid != std::process::id() as i32
    {
        return None;
    }
    Some(usec)
}

// ── spawning ──────────────────────────────────────────────────────────────────

fn spawn_margo(
    path: &std::path::Path,
    args: &[OsString],
    heartbeat_usec: Option<u64>,
) -> Result<(std::process::Child, ProtoReader)> {
    let (ready_reader, ready_write_fd) = ready_pipe().context("create readiness pipe")?;
    let mut cmd = Command::new(path);
    cmd.args(args);
    cmd.env("MARGO_READY_FD", ready_write_fd.to_string());
    // When systemd wants watchdog keep-alives, ask margo to heartbeat the
    // readiness pipe from its event loop at a comfortable fraction of the
    // deadline so a single missed beat doesn't trip the watchdog.
    if let Some(usec) = heartbeat_usec {
        cmd.env("MARGO_HEARTBEAT_USEC", usec.to_string());
    }

    // SAFETY: `pre_exec` runs in the forked child between fork() and
    // execvp(). At that point only the calling thread exists.
    //   * PR_SET_PDEATHSIG survives exec (kernel keeps it on task_struct), so
    //     margo dies if the supervisor does.
    //   * getppid()==1 means the supervisor already died in the fork/exec
    //     window — bail so we don't leak a reparented compositor.
    //   * We unblock the forwarded signals: start-margo blocks them
    //     process-wide for its signalfd, and that mask is inherited across
    //     fork+exec — margo must start with a clean mask or it would never
    //     see the SIGTERM we forward to it.
    unsafe {
        cmd.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() == 1 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "start-margo died before child exec",
                ));
            }
            let mut empty: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut empty);
            if libc::pthread_sigmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            // SAFETY: ready_write_fd is still owned by the parent on spawn
            // failure.
            unsafe {
                libc::close(ready_write_fd);
            }
            return Err(e).context("spawn margo (check --path?)");
        }
    };
    // Parent must not hold the write end; otherwise EOF would never indicate
    // "child exited before signalling ready".
    // SAFETY: ready_write_fd is still owned by the parent here.
    unsafe {
        libc::close(ready_write_fd);
    }
    Ok((child, ready_reader))
}

// ── config preflight ──────────────────────────────────────────────────────────

fn config_arg(args: &[OsString]) -> Option<OsString> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let bytes = arg.as_os_str().as_bytes();
        match bytes {
            b"-c" | b"--config" => return iter.next().cloned(),
            _ if bytes.starts_with(b"--config=") => {
                return Some(OsString::from(
                    String::from_utf8_lossy(&bytes[b"--config=".len()..]).into_owned(),
                ));
            }
            _ if bytes.starts_with(b"-c") && bytes.len() > 2 => {
                return Some(OsString::from(
                    String::from_utf8_lossy(&bytes[2..]).into_owned(),
                ));
            }
            _ => {}
        }
    }
    None
}

fn run_preflight(args: &Args) -> Result<Option<i32>> {
    if args.no_preflight {
        return Ok(None);
    }

    let mut cmd = Command::new("mctl");
    cmd.arg("check-config");
    if let Some(path) = config_arg(&args.margo_args) {
        cmd.arg("--config").arg(path);
    }

    match cmd.status() {
        Ok(status) if status.success() => Ok(None),
        Ok(status) => {
            error!(?status, "config preflight failed; refusing to start margo");
            Ok(Some(EXIT_CONFIG_PREFLIGHT))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            warn!("mctl not found; skipping config preflight");
            Ok(None)
        }
        Err(e) => Err(e).context("run config preflight"),
    }
}

// ── exit / budget / backoff helpers ───────────────────────────────────────────

fn exit_code_from_status(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    status.signal().map_or(1, |sig| 128 + sig)
}

fn crash_budget_exhausted(crash_count: usize, max_crashes: u32) -> bool {
    (crash_count as u32) >= max_crashes
}

/// Exponential backoff between restarts: 250ms, 500ms, 1s, 2s, 4s, capped at
/// 5s. `consecutive` is the number of crashes in the current window (≥1).
/// Keeps a relentlessly-crashing config from hammering the GPU init path while
/// still restarting near-instantly after the first crash.
fn restart_backoff(consecutive: usize) -> Duration {
    let shift = consecutive.saturating_sub(1).min(5) as u32;
    let ms = 250u64.saturating_mul(1u64 << shift).min(5_000);
    Duration::from_millis(ms)
}

// ── the wait loop ──────────────────────────────────────────────────────────────

/// How a single margo invocation ended.
struct WaitResult {
    status: ExitStatus,
    /// `Some(sig)` if we forwarded a shutdown signal during this run — the
    /// session is being torn down, not crashing.
    shutdown_signal: Option<i32>,
    /// margo reported an unrecoverable failure over the readiness pipe.
    fatal: bool,
}

/// Wait for the spawned margo to exit, fully event-driven: `poll(2)` over the
/// signalfd, the child's pidfd, and the readiness pipe. Handles readiness
/// notification, watchdog forwarding, signal forwarding (race-free) and the
/// shutdown-timeout SIGKILL escalation.
fn wait_for_margo(
    child: &mut std::process::Child,
    pidfd: &OwnedFd,
    ready: &mut ProtoReader,
    signalfd: RawFd,
    args: &Args,
) -> Result<WaitResult> {
    let ready_timeout =
        (args.ready_timeout_secs > 0).then(|| Duration::from_secs(args.ready_timeout_secs));
    let shutdown_timeout = Duration::from_secs(args.shutdown_timeout_secs);
    let child_pid = child.id() as i32;
    let started = Instant::now();

    let mut ready_notified = false;
    let mut shutdown_at: Option<Instant> = None;
    let mut shutdown_signal: Option<i32> = None;
    let mut killed = false;
    let mut fatal = false;

    loop {
        // Build the poll set fresh each iteration: drop the readiness pipe once
        // it hits EOF so a closed pipe doesn't spin the loop on POLLHUP.
        let mut fds: [libc::pollfd; 3] = [
            libc::pollfd {
                fd: signalfd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: pidfd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if ready.eof { -1 } else { ready.raw_fd() },
                events: libc::POLLIN,
                revents: 0,
            },
        ];

        let timeout_ms = next_timeout_ms(
            ready_notified,
            ready_timeout,
            started,
            shutdown_at,
            killed,
            shutdown_timeout,
        );

        // SAFETY: fds points at a valid array of 3 pollfd; timeout is in ms.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e).context("poll");
        }

        // Signals first: forward any pending shutdown signal to the child.
        if fds[0].revents != 0 {
            for sig in drain_signalfd(signalfd).unwrap_or_default() {
                if shutdown_at.is_none() {
                    shutdown_at = Some(Instant::now());
                    shutdown_signal = Some(sig);
                    info!(signal = sig, "forwarding shutdown signal to margo");
                    sd_notify("STOPPING=1\nSTATUS=stopping margo\n", args.no_notify);
                }
                send_signal(child_pid, sig);
            }
        }

        // Readiness / watchdog / status lines from margo.
        if fds[2].revents != 0 {
            match ready.drain_lines() {
                Ok(lines) => {
                    for line in lines {
                        match line {
                            ProtoLine::Ready => {
                                if !ready_notified {
                                    sd_notify("READY=1\nSTATUS=margo ready\n", args.no_notify);
                                    ready_notified = true;
                                    info!("margo signalled ready");
                                }
                            }
                            ProtoLine::Watchdog => {
                                sd_notify("WATCHDOG=1\n", args.no_notify);
                            }
                            ProtoLine::Status(s) => {
                                sd_notify(&format!("STATUS={s}\n"), args.no_notify);
                            }
                            ProtoLine::Fatal => {
                                error!("margo reported an unrecoverable failure; will not restart");
                                fatal = true;
                            }
                            ProtoLine::Other(s) => debug!(line = %s, "readiness pipe line"),
                        }
                    }
                }
                Err(e) => warn!("readiness pipe read failed: {e}"),
            }
        }

        // Child exit — reap and return.
        if fds[1].revents != 0 {
            let status = child.wait().context("wait for margo")?;
            return Ok(WaitResult {
                status,
                shutdown_signal,
                fatal,
            });
        }

        // Timeouts (poll returned 0, or we just want to re-check deadlines).
        let now = Instant::now();
        if !ready_notified
            && let Some(timeout) = ready_timeout
            && started.elapsed() >= timeout
        {
            warn!(
                timeout_secs = args.ready_timeout_secs,
                "margo did not signal readiness before timeout; marking service ready anyway"
            );
            sd_notify(
                "READY=1\nSTATUS=margo running (readiness timeout)\n",
                args.no_notify,
            );
            ready_notified = true;
        }
        if let Some(started_at) = shutdown_at
            && !killed
            && shutdown_timeout > Duration::ZERO
            && now.duration_since(started_at) >= shutdown_timeout
        {
            warn!(
                pid = child_pid,
                timeout_secs = shutdown_timeout.as_secs(),
                "margo did not exit after forwarded signal; sending SIGKILL"
            );
            send_signal(child_pid, libc::SIGKILL);
            killed = true;
        }
    }
}

/// Compute the `poll(2)` timeout (ms): the soonest pending deadline
/// (readiness fallback or shutdown-kill escalation), or `-1` to block until an
/// fd is ready when nothing is pending.
fn next_timeout_ms(
    ready_notified: bool,
    ready_timeout: Option<Duration>,
    started: Instant,
    shutdown_at: Option<Instant>,
    killed: bool,
    shutdown_timeout: Duration,
) -> libc::c_int {
    let mut deadline: Option<Duration> = None;
    let mut consider = |remaining: Duration| {
        deadline = Some(deadline.map_or(remaining, |d: Duration| d.min(remaining)));
    };

    if !ready_notified && let Some(timeout) = ready_timeout {
        consider(timeout.saturating_sub(started.elapsed()));
    }
    if let Some(started_at) = shutdown_at
        && !killed
        && shutdown_timeout > Duration::ZERO
    {
        consider(shutdown_timeout.saturating_sub(started_at.elapsed()));
    }

    match deadline {
        None => -1,
        // Clamp to at least 1ms so a just-passed deadline still wakes poll
        // promptly rather than spinning at 0.
        Some(d) => (d.as_millis().max(1)).min(i32::MAX as u128) as libc::c_int,
    }
}

// ── main loop ──────────────────────────────────────────────────────────────────

fn run_loop(args: &Args, signalfd: RawFd) -> Result<i32> {
    let margo_path = args.path.clone().unwrap_or_else(|| PathBuf::from("margo"));

    if let Some(code) = run_preflight(args)? {
        return Ok(code);
    }

    let heartbeat_usec = watchdog_usec().map(|wd| (wd / 4).max(1_000_000));
    if let Some(hb) = heartbeat_usec {
        info!(
            heartbeat_secs = hb / 1_000_000,
            "systemd watchdog active; margo will heartbeat the readiness pipe"
        );
    }

    // The args we actually spawn with — swapped to the safe config if the crash
    // budget is exhausted and `--safe-config` is set.
    let mut active_args: Vec<OsString> = args.margo_args.clone();
    let mut safe_mode = false;

    // Crash log — Instants inside the rolling window.
    let mut crashes: Vec<Instant> = Vec::new();
    let window = Duration::from_secs(args.restart_window_secs);

    loop {
        info!(
            path = %margo_path.display(),
            args = ?active_args,
            safe_mode,
            "spawning margo"
        );

        let (mut child, mut ready_reader) =
            match spawn_margo(&margo_path, &active_args, heartbeat_usec) {
                Ok(spawned) => spawned,
                Err(e) => {
                    error!("spawn failed: {e:#}");
                    return Ok(EXIT_SPAWN_FAILED);
                }
            };
        let pidfd = match pidfd_open(child.id() as libc::pid_t) {
            Ok(fd) => fd,
            Err(e) => {
                error!("pidfd_open failed: {e}; killing child and aborting");
                send_signal(child.id() as i32, libc::SIGKILL);
                let _ = child.wait();
                return Ok(EXIT_SPAWN_FAILED);
            }
        };
        sd_notify("STATUS=starting margo\n", args.no_notify);

        let result = wait_for_margo(&mut child, &pidfd, &mut ready_reader, signalfd, args)?;

        if let Some(signal) = result.shutdown_signal {
            info!(
                ?result.status,
                signal,
                "shutdown requested via signal; margo exited"
            );
            sd_notify("STOPPING=1\n", args.no_notify);
            return Ok(exit_code_from_status(result.status));
        }

        if result.status.success() {
            info!("margo exited cleanly");
            return Ok(0);
        }

        if result.fatal {
            error!(?result.status, "margo reported a fatal failure; not restarting");
            sd_notify("STOPPING=1\nSTATUS=margo fatal failure\n", args.no_notify);
            return Ok(exit_code_from_status(result.status));
        }

        if args.no_restart {
            error!(?result.status, "margo exited; --no-restart set, leaving");
            return Ok(exit_code_from_status(result.status));
        }

        // Slide the rolling crash window before counting.
        let now = Instant::now();
        crashes.retain(|t| now.duration_since(*t) <= window);
        crashes.push(now);

        if crash_budget_exhausted(crashes.len(), args.max_crashes) {
            // One last shot at a known-good config before giving up.
            if !safe_mode && let Some(safe) = &args.safe_config {
                error!(
                    count = crashes.len(),
                    max = args.max_crashes,
                    safe_config = %safe.display(),
                    "crash budget exhausted — entering safe mode with the fallback config"
                );
                sd_notify("STATUS=safe mode (fallback config)\n", args.no_notify);
                safe_mode = true;
                active_args = vec![OsString::from("-c"), safe.clone().into_os_string()];
                crashes.clear();
                continue;
            }

            error!(
                count = crashes.len(),
                max = args.max_crashes,
                window_secs = window.as_secs(),
                "crash budget exhausted — leaving margo down so the DM \
                 can offer another session option"
            );
            sd_notify(
                "STOPPING=1\nSTATUS=crash budget exhausted\n",
                args.no_notify,
            );
            return Ok(EXIT_CRASH_BUDGET);
        }

        let backoff = restart_backoff(crashes.len());
        warn!(
            attempt = crashes.len(),
            max = args.max_crashes,
            backoff_ms = backoff.as_millis() as u64,
            ?result.status,
            "margo exited abnormally; restarting after backoff"
        );
        sd_notify(
            &format!("STATUS=restart attempt {}\n", crashes.len()),
            args.no_notify,
        );
        std::thread::sleep(backoff);
    }
}

fn init_logging() {
    let level = std::env::var("START_MARGO_LOG")
        .or_else(|_| std::env::var("MARGO_LOG"))
        .unwrap_or_else(|_| "info".to_string());
    // app_name doubles as the tracing filter target AND the log-file prefix;
    // it must be the crate target (underscores) so our own events aren't
    // filtered out — `start_margo`, not `start-margo`.
    let handle = margo_logging::init(margo_logging::LogInit {
        app_name: "start_margo".to_string(),
        dir: margo_logging::logs_dir(),
        level,
        enabled: true,
        keep_sessions: 5,
        to_stdout: true,
        env_override: Some("START_MARGO_LOG".to_string()),
    });
    let _ = LOG_HANDLE.set(handle);
}

fn main() -> ExitCode {
    init_logging();

    let args = Args::parse();

    // Block the forwarded signals before anything else so the signalfd catches
    // even a signal that arrives during startup / the spawn window.
    let sigset = match block_forwarded_signals() {
        Ok(set) => set,
        Err(e) => {
            error!("signal setup failed: {e:#}");
            return ExitCode::from(1);
        }
    };
    let signalfd = match create_signalfd(&sigset) {
        Ok(fd) => fd,
        Err(e) => {
            error!("signalfd setup failed: {e}");
            return ExitCode::from(1);
        }
    };

    match run_loop(&args, signalfd.as_raw_fd()) {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(e) => {
            error!("supervisor error: {e:#}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn extracts_config_from_margo_args() {
        assert_eq!(
            config_arg(&os_args(&["--config", "/tmp/margo.conf"])),
            Some(OsString::from("/tmp/margo.conf"))
        );
        assert_eq!(
            config_arg(&os_args(&["--config=/tmp/eq.conf"])),
            Some(OsString::from("/tmp/eq.conf"))
        );
        assert_eq!(
            config_arg(&os_args(&["-c", "/tmp/short.conf"])),
            Some(OsString::from("/tmp/short.conf"))
        );
        assert_eq!(
            config_arg(&os_args(&["-c/tmp/joined.conf"])),
            Some(OsString::from("/tmp/joined.conf"))
        );
        assert_eq!(config_arg(&os_args(&["--winit"])), None);
    }

    #[test]
    fn crash_budget_is_counted_as_crashes_not_restarts() {
        assert!(!crash_budget_exhausted(0, 3));
        assert!(!crash_budget_exhausted(2, 3));
        assert!(crash_budget_exhausted(3, 3));
    }

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(restart_backoff(1), Duration::from_millis(250));
        assert_eq!(restart_backoff(2), Duration::from_millis(500));
        assert_eq!(restart_backoff(3), Duration::from_millis(1_000));
        assert_eq!(restart_backoff(4), Duration::from_millis(2_000));
        assert_eq!(restart_backoff(5), Duration::from_millis(4_000));
        // Capped at 5s thereafter.
        assert_eq!(restart_backoff(6), Duration::from_millis(5_000));
        assert_eq!(restart_backoff(100), Duration::from_millis(5_000));
    }

    #[test]
    fn parses_protocol_lines() {
        assert_eq!(parse_proto_line("READY=1"), ProtoLine::Ready);
        assert_eq!(parse_proto_line("  READY=1 "), ProtoLine::Ready);
        assert_eq!(parse_proto_line("WATCHDOG=1"), ProtoLine::Watchdog);
        assert_eq!(parse_proto_line("FATAL=1"), ProtoLine::Fatal);
        assert_eq!(parse_proto_line("ERRNO=5"), ProtoLine::Fatal);
        assert_eq!(
            parse_proto_line("ERRNO=0"),
            ProtoLine::Other("ERRNO=0".to_string())
        );
        assert_eq!(
            parse_proto_line("STATUS=margo ready"),
            ProtoLine::Status("margo ready".to_string())
        );
        assert_eq!(
            parse_proto_line("garbage"),
            ProtoLine::Other("garbage".to_string())
        );
    }

    #[test]
    fn exit_codes_are_distinct() {
        // start-margo's own terminal conditions must not collide with each
        // other, so a supervising unit can tell them apart.
        let codes = [EXIT_CONFIG_PREFLIGHT, EXIT_CRASH_BUDGET, EXIT_SPAWN_FAILED];
        for (i, a) in codes.iter().enumerate() {
            for b in &codes[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn timeout_blocks_when_nothing_pending() {
        // No readiness deadline, no shutdown in progress → block indefinitely.
        let t = next_timeout_ms(
            true,
            None,
            Instant::now(),
            None,
            false,
            Duration::from_secs(5),
        );
        assert_eq!(t, -1);
    }

    #[test]
    fn timeout_tracks_readiness_deadline() {
        // Readiness not yet notified, 20s budget, just started → ~20s timeout.
        let t = next_timeout_ms(
            false,
            Some(Duration::from_secs(20)),
            Instant::now(),
            None,
            false,
            Duration::from_secs(5),
        );
        assert!(t > 0 && t <= 20_000);
    }

    #[test]
    fn notify_sockaddr_supports_abstract_namespace() {
        let (addr, len) = notify_sockaddr(OsStr::new("@margo-ready")).unwrap();
        let offset = (&addr.sun_path as *const _ as usize) - (&addr as *const _ as usize);
        assert_eq!(addr.sun_path[0], 0);
        assert_eq!(addr.sun_path[1] as u8, b'm');
        assert_eq!(addr.sun_path[2] as u8, b'a');
        assert_eq!(len as usize, offset + 1 + "margo-ready".len());
    }

    #[test]
    fn notify_sockaddr_supports_path_namespace() {
        let (addr, len) = notify_sockaddr(OsStr::new("/tmp/margo-notify.sock")).unwrap();
        let offset = (&addr.sun_path as *const _ as usize) - (&addr as *const _ as usize);
        assert_eq!(addr.sun_path[0] as u8, b'/');
        assert_eq!(addr.sun_path[1] as u8, b't');
        assert_eq!(len as usize, offset + "/tmp/margo-notify.sock".len() + 1);
    }

    #[test]
    fn ready_reader_splits_lines() {
        // Drive ProtoReader through a real pipe to exercise the line buffering
        // and EOF handling.
        let (mut reader, write_fd) = ready_pipe().unwrap();
        let msg = b"READY=1\nWATCHDOG=1\npartial";
        let n = unsafe { libc::write(write_fd, msg.as_ptr().cast(), msg.len()) };
        assert_eq!(n, msg.len() as isize);
        let lines = reader.drain_lines().unwrap();
        assert_eq!(lines, vec![ProtoLine::Ready, ProtoLine::Watchdog]);
        assert!(!reader.eof);
        // Finish the partial line, then close to signal EOF.
        let rest = b" =ignored\nSTATUS=done\n";
        unsafe {
            libc::write(write_fd, rest.as_ptr().cast(), rest.len());
            libc::close(write_fd);
        }
        let lines = reader.drain_lines().unwrap();
        assert_eq!(
            lines,
            vec![
                ProtoLine::Other("partial =ignored".to_string()),
                ProtoLine::Status("done".to_string())
            ]
        );
        assert!(reader.eof);
    }
}
