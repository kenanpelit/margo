//! midle — margo's idle manager.
//!
//! Listens on `ext-idle-notify-v1` for sequential idle thresholds and
//! runs configured shell commands at each step. Resumes (user
//! activity) trigger the matching `resume_command`s in reverse.
//!
//! Architecture:
//!   • Daemon thread: tokio runtime + Wayland event queue. Each
//!     config'd `[[step]]` becomes one `ext_idle_notification_v1`.
//!   • IPC: a unix socket at `$XDG_RUNTIME_DIR/midle.sock` — CLI
//!     subcommands (pause/resume/info/reload/stop) connect briefly.
//!   • State machine: Active | StepFired(n) | Paused — manager owns
//!     it; the Wayland and IPC tasks just push events at it.

#![allow(clippy::too_many_arguments)]

mod actions;
mod config;
mod daemon;
mod ipc;
mod state;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::error;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Idle manager for the margo Wayland compositor",
)]
struct Args {
    /// Path to the config file. Defaults to
    /// `$XDG_CONFIG_HOME/midle/config.toml` (or `~/.config/midle/...`).
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print the daemon's current state as JSON.
    Info,
    /// Suspend the idle timer for a duration, or until resumed.
    Pause {
        /// Optional duration suffix: `30s`, `5m`, `1h`. Omit for
        /// indefinite.
        duration: Option<String>,
    },
    /// Resume idle timing.
    Resume,
    /// Flip the manual inhibitor flag (used by status bars).
    ToggleInhibit,
    /// Re-read the config file.
    Reload,
    /// Tell the daemon to exit cleanly.
    Stop,
}

fn main() -> std::process::ExitCode {
    init_logging();

    let args = Args::parse();

    let result: Result<()> = match args.command {
        Some(cmd) => ipc::run_client(cmd).context("CLI command"),
        None => daemon::run(args.config.clone()).context("daemon"),
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            error!("midle: {e:#}");
            std::process::ExitCode::from(1)
        }
    }
}

fn init_logging() {
    let filter = std::env::var("MIDLE_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
