//! start-margo — supervisor / watchdog launcher for the margo
//! Wayland compositor.
//!
//! Inspired by Hyprland's `start-hyprland` C++ launcher, with three
//! concrete improvements:
//!
//!   1. **Crash budget.** A rolling time window caps the number of
//!      automatic restarts; once exceeded, the supervisor exits and
//!      the session returns to the display-manager. Prevents a CPU-
//!      pinning respawn loop when a config tweak crashes margo on
//!      every startup (start-hyprland will respawn indefinitely
//!      in safe-mode).
//!   2. **systemd notification.** Emits `READY=1` once margo is
//!      spawned and `STOPPING=1` on shutdown, so a `Type=notify`
//!      systemd user service (e.g. uwsm's `wayland-wm@margo.service`
//!      template) knows the session is up without polling.
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

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
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
                  to the child, emits sd_notify READY=1 once spawned, and pins the \
                  child to die with the supervisor via PR_SET_PDEATHSIG."
)]
struct Args {
    /// Path to the margo binary. Defaults to `margo` resolved through PATH.
    #[arg(long, value_name = "PATH")]
    path: Option<PathBuf>,

    /// Hard cap on restart attempts inside the rolling crash window.
    /// After this many crashes in `--restart-window-secs`, the
    /// supervisor exits non-zero and the session returns to the DM.
    #[arg(long, value_name = "N", default_value = "3")]
    max_restarts: u32,

    /// Length of the rolling crash window (seconds) used by
    /// `--max-restarts`.
    #[arg(long, value_name = "SECONDS", default_value = "60")]
    restart_window_secs: u64,

    /// One-shot mode — disable automatic restart on crash. Useful
    /// when debugging an individual session.
    #[arg(long)]
    no_restart: bool,

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
    use std::os::unix::net::UnixDatagram;
    let Ok(sock) = UnixDatagram::unbound() else {
        return;
    };

    // Abstract namespace socket paths start with `@` in NOTIFY_SOCKET
    // and need a leading NUL byte on the wire. `sendto()` on a normal
    // path or a `\0`-prefixed path both work via SocketAddr::from_pathname,
    // but the abstract case requires SocketAddr::from_abstract_name,
    // which is nightly-only. Workaround: write the leading NUL into
    // a fresh PathBuf component. Almost no DMs use abstract sockets
    // in practice, so we fall back to a warning and skip the notify
    // rather than pull in tokio/nix for one syscall.
    let path_str = socket_path.to_string_lossy();
    if path_str.starts_with('@') {
        warn!(
            "NOTIFY_SOCKET is an abstract address ({path_str}); \
             stable-rust UnixDatagram can't reach it. Skipping sd_notify."
        );
        return;
    }

    if let Err(e) = sock.send_to(state.as_bytes(), &socket_path) {
        warn!(state, ?socket_path, "sd_notify failed: {e}");
    }
}

fn spawn_margo(path: &std::path::Path, args: &[OsString]) -> Result<std::process::Child> {
    let mut cmd = Command::new(path);
    cmd.args(args);

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
            Ok(())
        });
    }

    let child = cmd.spawn().context("spawn margo (check --path?)")?;
    CHILD_PID.store(child.id() as i32, Ordering::SeqCst);
    Ok(child)
}

fn run_loop(args: &Args) -> Result<i32> {
    let margo_path = args
        .path
        .clone()
        .unwrap_or_else(|| PathBuf::from("margo"));

    // Crash log — Vec of Instants inside the rolling window. We don't
    // need a ring buffer because `--max-restarts` caps the size at a
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

        let mut child = match spawn_margo(&margo_path, &args.margo_args) {
            Ok(c) => c,
            Err(e) => {
                error!("spawn failed: {e:#}");
                // 127 mirrors POSIX shell convention for "command not
                // executable" — useful when start-margo is in a service
                // unit and `systemctl status` reads the exit code.
                return Ok(127);
            }
        };
        sd_notify("READY=1\nSTATUS=margo running\n", args.no_notify);

        // `wait()` blocks until the child exits. Signals delivered to
        // the supervisor during the wait fire `forward_signal`, which
        // forwards them to the child; the child's eventual exit then
        // wakes the wait().
        let status = child.wait().context("wait for margo")?;
        CHILD_PID.store(0, Ordering::SeqCst);

        let signaled_shutdown = SHUTDOWN_REQUESTED.load(Ordering::SeqCst);
        if signaled_shutdown {
            info!(?status, "shutdown requested via signal; margo exited");
            sd_notify("STOPPING=1\n", args.no_notify);
            return Ok(status.code().unwrap_or(0));
        }

        if status.success() {
            info!("margo exited cleanly");
            return Ok(0);
        }

        if args.no_restart {
            error!(?status, "margo exited; --no-restart set, leaving");
            return Ok(status.code().unwrap_or(1));
        }

        // Slide the rolling crash window: drop entries older than
        // `window` before counting.
        let now = Instant::now();
        crashes.retain(|t| now.duration_since(*t) <= window);
        crashes.push(now);

        if crashes.len() as u32 >= args.max_restarts {
            error!(
                count = crashes.len(),
                max = args.max_restarts,
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
            max = args.max_restarts,
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
