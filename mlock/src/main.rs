//! mlock — margo's screen locker.
//!
//! Uses `ext-session-lock-v1`, the Wayland protocol designed for session
//! locking. The compositor (margo) cooperates: while a lock is active it
//! hides *every* surface that isn't ours, and if our process dies the
//! session stays locked until margo's `force_unlock` is invoked.
//!
//! Stack:
//!   • wayland-client + wayland-protocols (staging — ext-session-lock-v1)
//!   • cairo + pango for software rendering (no GPU dependency)
//!   • xkbcommon for keyboard
//!   • our own libpam FFI in `auth::pam`
//!
//! Lock flow:
//!   1. Connect to Wayland, bind globals
//!   2. ExtSessionLockManagerV1::lock() → SessionLock
//!   3. For each wl_output: SessionLock::get_lock_surface() and submit
//!      an initial buffer
//!   4. Spin the event queue, handle keystrokes
//!   5. Enter → PAM auth → on success SessionLock::unlock_and_destroy()
//!   6. Drain the queue, exit cleanly

#![allow(clippy::too_many_arguments)]

mod auth;
mod background;
mod battery;
mod config;
mod icons;
mod power;
mod render;
mod seat;
mod sidecar;
mod state;
mod surface;
mod wallpaper;

use anyhow::{Context, Result};
use tracing::{error, info};
use wayland_client::Connection;

use crate::state::MlockState;

/// What the first CLI argument (if any) asks `main` to do instead of
/// locking. mlock never had any argument parsing at all -- `mlock --help`
/// (or any other flag) fell straight through to `run()` and genuinely
/// locked the session, same as a bare `mlock`. That's surprising and
/// dangerous CLI behavior on its own, and it's also what let margo's
/// Settings -> Guide page crash the compositor: its Tools tab runs
/// `<binary> --help` on every companion tool assuming that's always a
/// quick, side-effect-free no-op, which was false here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EarlyExit {
    Help,
    Version,
}

fn early_exit_action(args: &[String]) -> Option<EarlyExit> {
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => Some(EarlyExit::Help),
        Some("--version" | "-V") => Some(EarlyExit::Version),
        _ => None,
    }
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match early_exit_action(&args) {
        Some(EarlyExit::Help) => {
            print_usage();
            return std::process::ExitCode::SUCCESS;
        }
        Some(EarlyExit::Version) => {
            println!("mlock {}", env!("CARGO_PKG_VERSION"));
            return std::process::ExitCode::SUCCESS;
        }
        None => {}
    }

    init_logging();

    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            error!("mlock fatal: {e:#}");
            // The compositor keeps the session locked if we exit
            // without calling unlock_and_destroy — that's the right
            // failure mode for a locker.
            std::process::ExitCode::from(1)
        }
    }
}

fn print_usage() {
    println!(
        "mlock {}\nmargo's screen locker.\n\nUSAGE:\n    mlock\n\nTakes no arguments -- running it (with or without flags) locks the\ncurrent session via ext-session-lock-v1. Configuration is via config\nfiles under $XDG_CONFIG_HOME/mlock, not CLI flags.\n\nOPTIONS:\n    -h, --help       Print this message and exit\n    -V, --version    Print version information and exit\n\nENVIRONMENT:\n    MLOCK_LOG        Log filter (default: info)\n    MLOCK_LOG_FILE   Log file path (default: $XDG_RUNTIME_DIR/mlock-debug.log)",
        env!("CARGO_PKG_VERSION")
    );
}

fn init_logging() {
    let filter = std::env::var("MLOCK_LOG").unwrap_or_else(|_| "info".to_string());
    // Always tee to a debug log so the user can post-mortem
    // from a TTY after a stuck lock (stderr is invisible when the
    // session is locked + no terminal attached).
    //
    // Default to `$XDG_RUNTIME_DIR/mlock-debug.log` so the file is
    // per-user (no `/tmp/mlock-debug.log` collisions on shared
    // machines, no symlink races in world-writable /tmp). Fall
    // back to `/tmp/` only when XDG_RUNTIME_DIR is unset — that
    // shouldn't happen on systemd-managed sessions, but mlock can
    // run on weird setups (TTY-only, recovery shells) so we keep
    // the fallback rather than refusing to log.
    let log_path = std::env::var("MLOCK_LOG_FILE").unwrap_or_else(|_| {
        match std::env::var("XDG_RUNTIME_DIR") {
            Ok(dir) if !dir.is_empty() => format!("{dir}/mlock-debug.log"),
            _ => "/tmp/mlock-debug.log".to_string(),
        }
    });
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();
    if let Some(file) = file {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init();
    }
}

fn run() -> Result<()> {
    info!("mlock starting");
    let conn = Connection::connect_to_env().context("connect to Wayland")?;
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut state = MlockState::new(&conn, &qh)?;

    // Initial roundtrip: registry → globals → outputs.
    event_queue
        .roundtrip(&mut state)
        .context("initial roundtrip")?;
    state.assert_globals()?;

    // Bind the session_lock and request lock surfaces for every output.
    state.lock_session(&qh)?;
    event_queue
        .roundtrip(&mut state)
        .context("post-lock roundtrip")?;

    info!(outputs = state.outputs.len(), "lock surfaces created");

    // Main loop — poll(2)-based so we get periodic ticks even when
    // no Wayland events arrive (live clock, shake animation, etc.).
    //
    // Per-iteration:
    //   1. dispatch any events already in the queue
    //   2. tick state (clock minute, shake decay) → flag dirty
    //   3. render_pending if any surface is dirty
    //   4. flush outgoing writes
    //   5. poll(wayland_fd, timeout) — short during animations,
    //      longer otherwise (cheaper on idle battery)
    //   6. prepare_read + read events into the queue if fd was ready
    use std::os::fd::AsRawFd;
    while !state.unlocked {
        event_queue
            .dispatch_pending(&mut state)
            .context("dispatch_pending")?;

        state.tick();

        if let Err(e) = state.render_pending(&qh) {
            tracing::warn!("render_pending failed: {e:#}");
        }

        state.conn.flush().context("flush")?;

        let timeout_ms: i32 = if state.seat_state.is_shaking() {
            16
        } else {
            500
        };
        let fd = state.conn.backend().poll_fd().as_raw_fd();
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let r = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if r > 0
            && pfd.revents & libc::POLLIN != 0
            && let Some(guard) = event_queue.prepare_read()
        {
            let _ = guard.read();
        }
    }

    // CRITICAL: `unlock_and_destroy` is a Wayland *request* queued
    // on the connection's outbound buffer. Per ext-session-lock-v1
    // spec, if the client disconnects *without* this request having
    // reached the compositor, the session stays locked — and there
    // is no way to recover except via the compositor's emergency
    // keybind (margo's `force_unlock`).
    //
    // We must therefore roundtrip to the compositor before exiting
    // so the unlock takes effect. Without this the user gets a
    // black/locked screen forever after auth succeeds.
    if let Err(e) = event_queue.roundtrip(&mut state) {
        tracing::warn!("final roundtrip failed: {e:#}");
    } else {
        info!("unlock request flushed to compositor");
    }

    info!("mlock unlocked, exiting");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_args_does_not_early_exit() {
        assert_eq!(early_exit_action(&args(&[])), None);
    }

    #[test]
    fn help_flags_trigger_help() {
        assert_eq!(early_exit_action(&args(&["--help"])), Some(EarlyExit::Help));
        assert_eq!(early_exit_action(&args(&["-h"])), Some(EarlyExit::Help));
    }

    #[test]
    fn version_flags_trigger_version() {
        assert_eq!(
            early_exit_action(&args(&["--version"])),
            Some(EarlyExit::Version)
        );
        assert_eq!(early_exit_action(&args(&["-V"])), Some(EarlyExit::Version));
    }

    #[test]
    fn unrecognized_arg_does_not_early_exit() {
        // Anything else falls through to the real lock flow -- mlock has no
        // other flags, so an unknown one is silently ignored rather than
        // treated as an error (matches the pre-fix behavior for non-help
        // args, which is the smallest possible change).
        assert_eq!(early_exit_action(&args(&["--bogus"])), None);
    }

    #[test]
    fn only_the_first_argument_is_checked() {
        // `--help` in a later position doesn't match -- mirrors clap's own
        // "only recognized in argument position" behavior for a flag this
        // simple, and keeps the check trivial to reason about.
        assert_eq!(early_exit_action(&args(&["--foo", "--help"])), None);
    }
}
