//! mlock — margo's screen locker.
//!
//! Uses `ext-session-lock-v1`, the Wayland protocol designed for session
//! locking. The compositor (margo) cooperates: while a lock is active it
//! hides *every* surface that isn't ours, and if our process dies the
//! session stays locked until margo's `force_unlock` is invoked. This
//! is strictly stronger than the wlr-layer-shell overlay approach mshell
//! shipped previously.
//!
//! Stack:
//!   • wayland-client + wayland-protocols (staging — ext-session-lock-v1)
//!   • cairo + pango for software rendering (no GPU dependency)
//!   • xkbcommon for keyboard
//!   • our own libpam FFI in `auth::pam` (shared with mshell)
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
mod battery;
mod power;
mod render;
mod seat;
mod state;
mod surface;
mod wallpaper;

use anyhow::{Context, Result};
use tracing::{error, info};
use wayland_client::Connection;

use crate::state::MlockState;

fn main() -> std::process::ExitCode {
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

fn init_logging() {
    let filter = std::env::var("MLOCK_LOG").unwrap_or_else(|_| "info".to_string());
    // Always tee to /tmp/mlock-debug.log so the user can post-mortem
    // from a TTY after a stuck lock (stderr is invisible when the
    // session is locked + no terminal attached).
    let log_path = std::env::var("MLOCK_LOG_FILE")
        .unwrap_or_else(|_| "/tmp/mlock-debug.log".to_string());
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

    info!(
        outputs = state.outputs.len(),
        "lock surfaces created"
    );

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

        let timeout_ms: i32 = if state.seat_state.is_shaking() { 16 } else { 500 };
        let fd = state.conn.backend().poll_fd().as_raw_fd();
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let r = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if r > 0 && pfd.revents & libc::POLLIN != 0
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
