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
mod render;
mod seat;
mod state;
mod surface;

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

    // Main loop. Each iteration:
    //   1. blocking_dispatch — wait for + consume one batch of events
    //      (keystrokes, configure, buffer release, etc.). Handlers
    //      flip `needs_redraw` on the affected surfaces.
    //   2. render_pending — flush any surfaces whose state changed
    //      since the last frame (typed character, fail message
    //      cleared, etc.). Without this the lock UI is frozen on
    //      the initial configure render.
    while !state.unlocked {
        event_queue
            .blocking_dispatch(&mut state)
            .context("event dispatch")?;
        if let Err(e) = state.render_pending(&qh) {
            tracing::warn!("render_pending failed: {e:#}");
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
