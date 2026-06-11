//! start-margo — supervisor / watchdog launcher for the margo
//! Wayland compositor.
//!
//! Inspired by Hyprland's `start-hyprland` C++ launcher, with three
//! concrete improvements:
//!
//!   1. **Crash budget.** A rolling time window caps abnormal exits;
//!      once exceeded, the supervisor exits and
//!      the session returns to the display-manager. Prevents a CPU-
//!      pinning respawn loop when a config tweak crashes margo on
//!      every startup (start-hyprland will respawn indefinitely
//!      in safe-mode).
//!   2. **systemd notification.** Waits for margo's readiness pipe
//!      (socket + backend + environment are ready) before emitting
//!      `READY=1`, and emits `STOPPING=1` on shutdown, so a
//!      `Type=notify` systemd user service (e.g. uwsm's
//!      `wayland-wm@margo.service` template) knows the session is up
//!      without polling.
//!   3. **Standardized signal forwarding.** SIGTERM / SIGINT / SIGHUP
//!      are forwarded to the child compositor with the *original*
//!      signal preserved, so margo runs its own graceful teardown
//!      (Wayland surface destruction, ext-session-lock cleanup,
//!      session.json snapshot) before exiting — start-hyprland only
//!      sends SIGTERM regardless of what it received.
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
//! Single binary, single source file, no glaze / hyprutils
//! dependency. Inherits the workspace's tracing stack — control
//! verbosity with `START_MARGO_LOG=debug` (falls back to `MARGO_LOG`
//! if start-specific filter isn't set).

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{error, info, warn};

/// Watchdog supervisor for the margo Wayland compositor.
#[derive(Parser, Debug)]
#[command(
    version,
    about = "Watchdog supervisor for the margo Wayland compositor",
    long_about = "Starts margo, waits for it to exit, and — if the exit was abnormal — \
                  restarts it within the crash budget. Forwards SIGTERM/SIGINT/SIGHUP \
                  to the child, emits sd_notify READY=1 only after margo signals real \
                  readiness, and pins the child to die with the supervisor via \
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

// Signal-handler globals. Atomics because they're touched from an
// async-signal context where allocator-unsafe operations would be UB.
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_SIGNAL: AtomicI32 = AtomicI32::new(0);
static CHILD_PID: AtomicI32 = AtomicI32::new(0);

/// Async-signal-safe handler: forward the received signal to the
/// child compositor, then flip the shutdown flag so the wait loop
/// stops restarting after the child exits.
extern "C" fn forward_signal(sig: libc::c_int) {
    let pid = CHILD_PID.load(Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: kill(2) is async-signal-safe.
        unsafe {
            libc::kill(pid, sig);
        }
    }
    SHUTDOWN_SIGNAL.store(sig, Ordering::SeqCst);
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() -> Result<()> {
    // SAFETY: sigaction(2) is async-signal-safe and we only set
    // process-wide handlers once during startup, before any threads
    // are spawned.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = forward_signal as *const () as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = libc::SA_RESTART;
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            if libc::sigaction(sig, &sa, std::ptr::null_mut()) != 0 {
                anyhow::bail!(
                    "sigaction({sig}) failed: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }
    Ok(())
}

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

fn set_fd_cloexec(fd: RawFd, enabled: bool) -> io::Result<()> {
    // SAFETY: fcntl(F_GETFD/F_SETFD) does not take ownership of fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let new_flags = if enabled {
        flags | libc::FD_CLOEXEC
    } else {
        flags & !libc::FD_CLOEXEC
    };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, new_flags) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_fd_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl(F_GETFL/F_SETFL) does not take ownership of fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct ReadyReader {
    file: File,
    buf: Vec<u8>,
}

impl ReadyReader {
    fn new(file: File) -> Self {
        Self {
            file,
            buf: Vec::new(),
        }
    }

    fn poll_ready(&mut self) -> io::Result<bool> {
        let mut chunk = [0u8; 64];
        loop {
            match self.file.read(&mut chunk) {
                Ok(0) => return Ok(false),
                Ok(n) => {
                    self.buf.extend_from_slice(&chunk[..n]);
                    if !self.buf.is_empty() {
                        return Ok(true);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

fn ready_pipe() -> io::Result<(ReadyReader, RawFd)> {
    let mut fds = [0; 2];
    // SAFETY: pipe fills both fd slots or returns -1 with errno set.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }

    if let Err(e) = set_fd_cloexec(fds[0], true)
        .and_then(|_| set_fd_cloexec(fds[1], false))
        .and_then(|_| set_fd_nonblocking(fds[0]))
    {
        // SAFETY: both fds came from pipe above.
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
        return Err(e);
    }

    // SAFETY: fds[0] is uniquely owned by this File from now on.
    let read_file = unsafe { File::from_raw_fd(fds[0]) };
    Ok((ReadyReader::new(read_file), fds[1]))
}

fn spawn_margo(
    path: &std::path::Path,
    args: &[OsString],
) -> Result<(std::process::Child, ReadyReader)> {
    let (ready_reader, ready_write_fd) = ready_pipe().context("create readiness pipe")?;
    let mut cmd = Command::new(path);
    cmd.args(args);
    cmd.env("MARGO_READY_FD", ready_write_fd.to_string());

    // SAFETY: `pre_exec` runs in the forked child between fork() and
    // execvp(). At that point only the calling thread exists, so
    // prctl()'s thread-scoped state is set for the (only) thread that
    // will become margo via execve. PR_SET_PDEATHSIG survives exec
    // (kernel keeps it on the task_struct).
    unsafe {
        cmd.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() == 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "start-margo died before child exec",
                ));
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
    CHILD_PID.store(child.id() as i32, Ordering::SeqCst);
    Ok((child, ready_reader))
}

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

/// First-run bootstrap. When there's no margo config at the default location
/// (`~/.config/margo/config.conf`), write a minimal-but-usable `config.conf` +
/// `binds.conf` so a brand-new login works instead of margo refusing to start.
/// Skipped when an explicit `--config` is passed (the user owns that path) or
/// when the file already exists. Best-effort: any failure is logged and ignored
/// — margo still falls back to built-in defaults and the relaxed preflight lets
/// it start regardless.
fn ensure_default_config(args: &Args) {
    // An explicit `--config <path>` means the user is managing config; don't
    // write into a path we were told to merely read.
    if config_arg(&args.margo_args).is_some() {
        return;
    }
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let dir = PathBuf::from(home).join(".config/margo");
    let config = dir.join("config.conf");
    if config.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!(error = %e, dir = %dir.display(), "first-run: could not create margo config dir");
        return;
    }
    match std::fs::write(&config, include_str!("../assets/default-config.conf")) {
        Ok(()) => info!(path = %config.display(), "first-run: wrote default margo config"),
        Err(e) => {
            warn!(error = %e, "first-run: could not write default config");
            return;
        }
    }
    // Placeholder for the matugen palette fragment the default config `source`s.
    // The config preflight rejects a `source` that doesn't resolve, and mshell
    // only writes this file on its first wallpaper apply — so seed an empty one
    // now (mshell overwrites it later). Create `conf.d/` first.
    let conf_d = dir.join("conf.d");
    if let Err(e) = std::fs::create_dir_all(&conf_d) {
        warn!(error = %e, "first-run: could not create conf.d");
    } else {
        let colors = conf_d.join("colors.conf");
        if !colors.exists() {
            let _ = std::fs::write(
                &colors,
                "# Auto-generated by mshell from the matugen palette. Placeholder\n\
                 # written on first run so config.conf's `source` resolves.\n",
            );
        }
    }
    // Starter binds (config.conf `source`s this). Only if absent so we never
    // clobber a binds.conf the user / Settings → Keybinds already manages.
    let binds = dir.join("binds.conf");
    if !binds.exists()
        && let Err(e) = std::fs::write(&binds, include_str!("../assets/default-binds.conf"))
    {
        warn!(error = %e, "first-run: could not write default binds");
    }
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
            Ok(Some(78))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            warn!("mctl not found; skipping config preflight");
            Ok(None)
        }
        Err(e) => Err(e).context("run config preflight"),
    }
}

fn exit_code_from_status(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    status.signal().map_or(1, |sig| 128 + sig)
}

fn crash_budget_exhausted(crash_count: usize, max_crashes: u32) -> bool {
    (crash_count as u32) >= max_crashes
}

fn send_signal(pid: u32, sig: libc::c_int) {
    // SAFETY: kill(2) is called with the child pid we got from std::process.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

fn wait_for_margo(
    child: &mut std::process::Child,
    ready_reader: &mut ReadyReader,
    args: &Args,
) -> Result<(ExitStatus, bool)> {
    let ready_timeout = if args.ready_timeout_secs == 0 {
        None
    } else {
        Some(Duration::from_secs(args.ready_timeout_secs))
    };
    let shutdown_timeout = Duration::from_secs(args.shutdown_timeout_secs);
    let child_pid = child.id();
    let started = Instant::now();
    let mut ready_notified = false;
    let mut shutdown_started: Option<Instant> = None;
    let mut killed = false;

    loop {
        if !ready_notified {
            match ready_reader.poll_ready() {
                Ok(true) => {
                    sd_notify("READY=1\nSTATUS=margo ready\n", args.no_notify);
                    ready_notified = true;
                }
                Ok(false) => {}
                Err(e) => warn!("readiness pipe read failed: {e}"),
            }
        }

        if let Some(status) = child.try_wait().context("wait for margo")? {
            return Ok((status, ready_notified));
        }

        let now = Instant::now();
        if SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
            if shutdown_started.is_none() {
                shutdown_started = Some(now);
                sd_notify("STOPPING=1\nSTATUS=stopping margo\n", args.no_notify);
            } else if let Some(started_at) = shutdown_started
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
        } else if !ready_notified
            && ready_timeout.is_some_and(|timeout| started.elapsed() >= timeout)
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

        std::thread::sleep(Duration::from_millis(50));
    }
}

fn run_loop(args: &Args) -> Result<i32> {
    let margo_path = args.path.clone().unwrap_or_else(|| PathBuf::from("margo"));

    // First-run bootstrap BEFORE the preflight: a fresh machine has no
    // `~/.config/margo/config.conf`, and margo's preflight refuses to start
    // without one. Write a minimal usable default so the very first login
    // comes up instead of bouncing back to the greeter.
    ensure_default_config(args);

    if let Some(code) = run_preflight(args)? {
        return Ok(code);
    }

    // Crash log — Vec of Instants inside the rolling window. We don't
    // need a ring buffer because `--max-crashes` caps the size at a
    // tiny number (default 3) and entries fall out of the window
    // naturally on every iteration.
    let mut crashes: Vec<Instant> = Vec::new();
    let window = Duration::from_secs(args.restart_window_secs);

    loop {
        info!(
            path = %margo_path.display(),
            args = ?args.margo_args,
            "spawning margo"
        );

        let (mut child, mut ready_reader) = match spawn_margo(&margo_path, &args.margo_args) {
            Ok(spawned) => spawned,
            Err(e) => {
                error!("spawn failed: {e:#}");
                // 127 mirrors POSIX shell convention for "command not
                // executable" — useful when start-margo is in a service
                // unit and `systemctl status` reads the exit code.
                return Ok(127);
            }
        };
        sd_notify("STATUS=starting margo\n", args.no_notify);

        let (status, _ready_notified) = wait_for_margo(&mut child, &mut ready_reader, args)?;
        CHILD_PID.store(0, Ordering::SeqCst);

        let signaled_shutdown = SHUTDOWN_REQUESTED.load(Ordering::SeqCst);
        if signaled_shutdown {
            info!(
                ?status,
                signal = SHUTDOWN_SIGNAL.load(Ordering::SeqCst),
                "shutdown requested via signal; margo exited"
            );
            sd_notify("STOPPING=1\n", args.no_notify);
            return Ok(exit_code_from_status(status));
        }

        if status.success() {
            info!("margo exited cleanly");
            return Ok(0);
        }

        if args.no_restart {
            error!(?status, "margo exited; --no-restart set, leaving");
            return Ok(exit_code_from_status(status));
        }

        // Slide the rolling crash window: drop entries older than
        // `window` before counting.
        let now = Instant::now();
        crashes.retain(|t| now.duration_since(*t) <= window);
        crashes.push(now);

        if crash_budget_exhausted(crashes.len(), args.max_crashes) {
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
            return Ok(2);
        }

        warn!(
            attempt = crashes.len(),
            max = args.max_crashes,
            ?status,
            "margo exited abnormally; restarting"
        );
        sd_notify(
            &format!("STATUS=restart attempt {}\n", crashes.len()),
            args.no_notify,
        );

        // Tiny breather so we don't hammer the GPU init path back-to-
        // back when a config is broken — also gives the user a moment
        // to read the previous log line in journalctl --follow.
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn main() -> ExitCode {
    let log_filter = std::env::var("START_MARGO_LOG")
        .or_else(|_| std::env::var("MARGO_LOG"))
        .unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(log_filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();

    let args = Args::parse();

    if let Err(e) = install_signal_handlers() {
        error!("signal handler setup failed: {e:#}");
        return ExitCode::from(1);
    }

    match run_loop(&args) {
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
}
