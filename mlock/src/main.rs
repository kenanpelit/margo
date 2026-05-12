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
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
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

    // Main loop: blocking dispatch until auth succeeds.
    while !state.unlocked {
        event_queue
            .blocking_dispatch(&mut state)
            .context("event dispatch")?;
    }

    info!("mlock unlocked, exiting");
    Ok(())
}
