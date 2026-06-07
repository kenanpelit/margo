//! mshell's logging front-end — a thin wrapper over the shared
//! [`margo_logging`] engine.
//!
//! Stands up the shell's file sink (`~/.local/state/margo/logs/mshell-*.log`,
//! last `keep_sessions` kept) plus stdout, and stashes the live [`LogHandle`]
//! in a process-global so the D-Bus IPC layer (`mshellctl log …`) and the
//! Settings page can retune the level without a restart.

use std::sync::OnceLock;

pub use margo_logging::{LEVELS, LogError, LogHandle, logs_dir, normalize_level};

static HANDLE: OnceLock<LogHandle> = OnceLock::new();

/// Parameters for [`init`], usually sourced from `Config::logging`.
pub struct Init {
    pub enabled: bool,
    pub level: String,
    pub keep_sessions: usize,
}

/// Bring up shell logging. Call once, right after the config is loaded.
/// `RUST_LOG` still overrides the level at startup.
pub fn init(opts: Init) {
    let handle = margo_logging::init(margo_logging::LogInit {
        app_name: "mshell".to_string(),
        dir: margo_logging::logs_dir(),
        level: opts.level,
        enabled: opts.enabled,
        keep_sessions: opts.keep_sessions.max(1),
        to_stdout: true,
        env_override: Some("RUST_LOG".to_string()),
    });
    let _ = HANDLE.set(handle);
}

/// The live handle, once [`init`] has run.
pub fn handle() -> Option<&'static LogHandle> {
    HANDLE.get()
}

/// Live: set the shell's file-log level. No-op (Ok) before [`init`].
pub fn set_level(level: &str) -> Result<(), LogError> {
    match HANDLE.get() {
        Some(h) => h.set_level(level),
        None => Ok(()),
    }
}

/// Live: enable/disable the shell's file logging. No-op (Ok) before [`init`].
pub fn set_enabled(enabled: bool) -> Result<(), LogError> {
    match HANDLE.get() {
        Some(h) => h.set_enabled(enabled),
        None => Ok(()),
    }
}
