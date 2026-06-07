//! Shared file-logging engine for **margo** and **mshell**.
//!
//! One job: stand up `tracing` with the existing stdout layer *plus* a durable
//! per-session file sink, and hand back a [`LogHandle`] whose level can be
//! changed live (from Settings or a CLI command) without a restart.
//!
//! Design notes:
//! * **Location** — all logs live flat in [`logs_dir`]
//!   (`$XDG_STATE_HOME/margo/logs`, default `~/.local/state/margo/logs`),
//!   namespaced by an `{app}-` filename prefix (`margo-*.log`, `mshell-*.log`).
//! * **Session = one process start** — [`init`] timestamps a fresh file
//!   `{app}-YYYYMMDD-HHMMSS.log`, prunes older ones so at most `keep_sessions`
//!   survive, and refreshes a convenience `{app}-latest.log` symlink.
//! * **Durable writes** — the file layer uses `tracing_appender::rolling::never`
//!   (synchronous); each event hits the kernel immediately, so a crash still
//!   leaves the tail on disk (the whole point — catching bugs after the fact).
//! * **Live level** — the file layer's `EnvFilter` is wrapped in a
//!   `reload::Layer`; [`LogHandle::set_level`] / [`LogHandle::set_enabled`]
//!   swap it. `enabled = false` maps the filter to `"off"`, so toggling file
//!   logging is live too.
//! * **Never panics** — if the directory or file can't be opened we fall back
//!   to stdout-only and log a warning (this runs on the login-critical path).

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt as fmt_layer;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{Registry, reload};

/// The five log levels we expose, lowest to highest verbosity.
pub const LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

/// Parameters for [`init`].
pub struct LogInit {
    /// `"margo"` or `"mshell"` — the filter target and the file prefix.
    pub app_name: String,
    /// The shared log directory (both apps pass the same one). Usually
    /// [`logs_dir`].
    pub dir: PathBuf,
    /// Initial level, one of [`LEVELS`].
    pub level: String,
    /// Whether file logging is on (false ⇒ the file filter starts `"off"`).
    pub enabled: bool,
    /// How many session files to keep on disk.
    pub keep_sessions: usize,
    /// Also mirror to stdout (preserves the pre-existing console behaviour).
    pub to_stdout: bool,
    /// Optional env var whose value overrides the filter at startup
    /// (e.g. `"MARGO_LOG"` for margo, `"RUST_LOG"` for mshell). Honoured for
    /// both the file and the stdout layers when set + non-empty.
    pub env_override: Option<String>,
}

/// Live control over the running logger. Returned by [`init`]; store it for the
/// lifetime of the process (e.g. a `OnceLock`) so the file sink keeps writing.
pub struct LogHandle {
    dir: PathBuf,
    current_file: PathBuf,
    reload: Option<reload::Handle<EnvFilter, Registry>>,
    state: Mutex<FilterState>,
}

struct FilterState {
    app: String,
    level: String,
    enabled: bool,
}

/// Error from a live logger adjustment.
#[derive(Debug)]
pub enum LogError {
    /// The level string was not one of [`LEVELS`].
    InvalidLevel(String),
    /// The reload handle rejected the new filter.
    Reload(String),
}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogError::InvalidLevel(s) => {
                write!(f, "invalid log level '{s}' (expected one of {LEVELS:?})")
            }
            LogError::Reload(e) => write!(f, "could not apply log level: {e}"),
        }
    }
}

impl std::error::Error for LogError {}

/// `$XDG_STATE_HOME/margo/logs`, falling back to `~/.local/state/margo/logs`.
pub fn logs_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".local/state")
        });
    base.join("margo").join("logs")
}

/// Validate + normalise a level string to lowercase, or `None` if unknown.
pub fn normalize_level(level: &str) -> Option<String> {
    let l = level.trim().to_ascii_lowercase();
    LEVELS.contains(&l.as_str()).then_some(l)
}

/// The `EnvFilter` directive string for an app at a level: a `warn` baseline
/// for noisy dependencies plus the app's own target at the chosen level.
/// `enabled = false` silences everything (`"off"`).
fn filter_string(app: &str, level: &str, enabled: bool) -> String {
    if !enabled {
        return "off".to_string();
    }
    // The crate target uses underscores (`margo`, `mshell`); callers pass the
    // binary name which already matches.
    format!("warn,{app}={level}")
}

/// The filter to start with: an env override (if set + non-empty) wins,
/// otherwise the configured app/level directive.
fn initial_filter(env_override: Option<&str>, app: &str, level: &str, enabled: bool) -> String {
    if let Some(var) = env_override
        && let Ok(val) = std::env::var(var)
        && !val.trim().is_empty()
    {
        return val;
    }
    filter_string(app, level, enabled)
}

/// Session file name for "now": `{app}-YYYYMMDD-HHMMSS.log`.
fn session_file_name(app: &str) -> String {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    format!("{app}-{ts}.log")
}

/// Delete the oldest `{app}-*.log` session files until at most `max_remaining`
/// remain. Excludes the `{app}-latest.log` symlink. Best-effort: a failed
/// delete logs nothing here (caller decides) and never aborts.
fn prune_sessions(dir: &Path, app: &str, max_remaining: usize) -> std::io::Result<()> {
    let latest = format!("{app}-latest.log");
    let prefix = format!("{app}-");
    let mut sessions: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            name != latest && name.starts_with(&prefix) && name.ends_with(".log")
        })
        .collect();
    // Timestamped names sort chronologically as plain strings.
    sessions.sort();
    if sessions.len() > max_remaining {
        let cut = sessions.len() - max_remaining;
        for old in sessions.into_iter().take(cut) {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(())
}

/// Point `{app}-latest.log` at `target_name` (best-effort; symlink is a
/// convenience, never required).
fn update_latest_symlink(dir: &Path, app: &str, target_name: &str) {
    let link = dir.join(format!("{app}-latest.log"));
    let _ = std::fs::remove_file(&link);
    let _ = std::os::unix::fs::symlink(target_name, &link);
}

/// Stand up logging. Call once near process start. Returns a [`LogHandle`] —
/// keep it alive for the whole process.
pub fn init(opts: LogInit) -> LogHandle {
    let LogInit {
        app_name,
        dir,
        level,
        enabled,
        keep_sessions,
        to_stdout,
        env_override,
    } = opts;

    let level = normalize_level(&level).unwrap_or_else(|| "info".to_string());
    let env_ref = env_override.as_deref();
    let stdout_initial = initial_filter(env_ref, &app_name, &level, true);

    // Try to set the file sink up; on any filesystem failure, fall back to
    // stdout-only so we never block startup.
    let session_name = session_file_name(&app_name);
    let current_file = dir.join(&session_name);

    let file_ready = std::fs::create_dir_all(&dir).is_ok();
    if file_ready {
        // Keep room for the about-to-be-created session file.
        let _ = prune_sessions(&dir, &app_name, keep_sessions.saturating_sub(1));
        update_latest_symlink(&dir, &app_name, &session_name);
    }

    if file_ready {
        let file_initial = initial_filter(env_ref, &app_name, &level, enabled);
        let (filter, handle) = reload::Layer::new(EnvFilter::new(file_initial));
        let appender = tracing_appender::rolling::never(&dir, &session_name);
        let file_layer = fmt_layer::layer()
            .with_writer(appender)
            .with_ansi(false)
            .with_target(true)
            .with_filter(filter);

        let stdout = to_stdout.then(|| {
            fmt_layer::layer()
                .with_target(true)
                .with_filter(EnvFilter::new(&stdout_initial))
        });

        let _ = tracing_subscriber::registry()
            .with(file_layer)
            .with(stdout)
            .try_init();

        return LogHandle {
            dir,
            current_file,
            reload: Some(handle),
            state: Mutex::new(FilterState {
                app: app_name,
                level,
                enabled,
            }),
        };
    }

    // Fallback: stdout only.
    let stdout = to_stdout.then(|| {
        fmt_layer::layer()
            .with_target(true)
            .with_filter(EnvFilter::new(&stdout_initial))
    });
    let _ = tracing_subscriber::registry().with(stdout).try_init();
    tracing::warn!(
        "margo-logging: could not open log dir {} — file logging disabled, stdout only",
        dir.display()
    );

    LogHandle {
        dir,
        current_file,
        reload: None,
        state: Mutex::new(FilterState {
            app: app_name,
            level,
            enabled,
        }),
    }
}

impl LogHandle {
    /// Live: change the file layer's verbosity. Errors if `level` is unknown.
    pub fn set_level(&self, level: &str) -> Result<(), LogError> {
        let level =
            normalize_level(level).ok_or_else(|| LogError::InvalidLevel(level.to_string()))?;
        let mut st = self.state.lock().unwrap();
        st.level = level;
        self.apply(&st)
    }

    /// Live: enable or disable file logging.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), LogError> {
        let mut st = self.state.lock().unwrap();
        st.enabled = enabled;
        self.apply(&st)
    }

    fn apply(&self, st: &FilterState) -> Result<(), LogError> {
        let Some(handle) = &self.reload else {
            return Ok(()); // stdout-only fallback: nothing to retune.
        };
        let directive = filter_string(&st.app, &st.level, st.enabled);
        handle
            .reload(EnvFilter::new(directive))
            .map_err(|e| LogError::Reload(e.to_string()))
    }

    /// Path of this process's current session log file.
    pub fn current_file(&self) -> PathBuf {
        self.current_file.clone()
    }

    /// The shared log directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_string_off_when_disabled() {
        assert_eq!(filter_string("margo", "debug", false), "off");
    }

    #[test]
    fn filter_string_has_warn_baseline_and_app_level() {
        assert_eq!(filter_string("mshell", "trace", true), "warn,mshell=trace");
        assert_eq!(filter_string("margo", "info", true), "warn,margo=info");
    }

    #[test]
    fn normalize_level_accepts_ladder_rejects_junk() {
        for l in LEVELS {
            assert_eq!(normalize_level(l).as_deref(), Some(l));
        }
        assert_eq!(normalize_level("DEBUG").as_deref(), Some("debug"));
        assert_eq!(normalize_level(" warn ").as_deref(), Some("warn"));
        assert_eq!(normalize_level("verbose"), None);
        assert_eq!(normalize_level(""), None);
    }

    #[test]
    fn initial_filter_prefers_env_override() {
        // Use a unique var name so parallel tests don't collide.
        let var = "MARGO_LOGGING_TEST_OVERRIDE_A";
        unsafe { std::env::set_var(var, "trace,foo=debug") };
        assert_eq!(
            initial_filter(Some(var), "margo", "info", true),
            "trace,foo=debug"
        );
        unsafe { std::env::remove_var(var) };
        assert_eq!(
            initial_filter(Some(var), "margo", "info", true),
            "warn,margo=info"
        );
    }

    #[test]
    fn prune_keeps_newest_and_ignores_latest_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        // Five session files with sortable timestamps + the latest symlink name.
        for ts in [
            "20260101-000001",
            "20260101-000002",
            "20260101-000003",
            "20260101-000004",
            "20260101-000005",
        ] {
            std::fs::write(p.join(format!("margo-{ts}.log")), b"x").unwrap();
        }
        std::fs::write(p.join("margo-latest.log"), b"link-ish").unwrap();
        // A foreign app's file must be left alone.
        std::fs::write(p.join("mshell-20260101-000009.log"), b"x").unwrap();

        prune_sessions(p, "margo", 2).unwrap();

        let mut left: Vec<String> = std::fs::read_dir(p)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("margo-") && n.ends_with(".log") && n != "margo-latest.log")
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "margo-20260101-000004.log".to_string(),
                "margo-20260101-000005.log".to_string()
            ]
        );
        // Untouched neighbours.
        assert!(p.join("margo-latest.log").exists());
        assert!(p.join("mshell-20260101-000009.log").exists());
    }

    #[test]
    fn session_file_name_shape() {
        let name = session_file_name("margo");
        assert!(name.starts_with("margo-"));
        assert!(name.ends_with(".log"));
        // margo-YYYYMMDD-HHMMSS.log
        assert_eq!(name.len(), "margo-".len() + 15 + ".log".len());
    }
}
