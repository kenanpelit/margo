//! Shared `tmux` subprocess helpers — the Rust equivalent of `tm.sh`'s
//! `tmux_cmd`/`has_session_exact`/`attach_or_switch`/`validate_session_name`
//! family.

use anyhow::{Context, Result, bail};
use std::process::{Command, Stdio};

/// True if `tmux` resolves on `$PATH`.
pub fn installed() -> bool {
    Command::new("tmux")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// True if we're already running inside a tmux client.
pub fn is_in_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}

/// Run a `tmux` subcommand, returning its captured stdout on success.
pub fn run(args: &[&str]) -> Result<String> {
    let out = Command::new("tmux")
        .args(args)
        .output()
        .with_context(|| format!("spawn tmux {args:?}"))?;
    if !out.status.success() {
        bail!(
            "tmux {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run a `tmux` subcommand for its exit status only (stdout/stderr
/// discarded) — used for probes like `has-session` where a non-zero exit
/// is an expected, silent "no".
pub fn ok(args: &[&str]) -> bool {
    Command::new("tmux")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run a `tmux` subcommand attached to the real terminal (interactive:
/// `attach-session`, `switch-client`, …) — inherits stdio so the tmux
/// client actually takes over the screen.
pub fn run_interactive(args: &[&str]) -> Result<bool> {
    let status = Command::new("tmux")
        .args(args)
        .status()
        .with_context(|| format!("spawn tmux {args:?}"))?;
    Ok(status.success())
}

/// Exact-match session lookup (`tmux has-session -t "=<name>"`) — `=name`
/// anchors to an exact match so `KENP` doesn't also match `KENP-old`.
pub fn has_session_exact(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    ok(&["has-session", "-t", &format!("={name}")])
}

/// Session name rule from `tm.sh`: non-empty, ≤ 50 chars, and only
/// `[a-zA-Z0-9_.-]` (tmux's own `-t` target syntax reserves `:` `.` `$`
/// for session/window/pane addressing, so anything else risks a target
/// that doesn't mean what the user typed).
pub fn validate_session_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("session name cannot be empty");
    }
    if name.len() > 50 {
        bail!("session name too long (max 50 characters)");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        bail!("invalid session name: '{name}' — only letters, digits, '-', '_', '.' are allowed");
    }
    Ok(())
}

/// A session name derived from the current directory: the git worktree's
/// top-level directory name if we're inside one, else the plain cwd's
/// basename.
pub fn session_name_from_cwd() -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    if let Ok(out) = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        && out.status.success()
    {
        let top = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if let Some(name) = std::path::Path::new(&top).file_name() {
            return name.to_string_lossy().into_owned();
        }
    }
    cwd.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "session".to_string())
}

/// Attach to `name` if outside tmux, or `switch-client` if already inside
/// one (attaching from inside a session just nests it) — `tm.sh`'s
/// `attach_or_switch`.
pub fn attach_or_switch(name: &str) -> Result<()> {
    if !has_session_exact(name) {
        bail!("session '{name}' not found");
    }
    let verb = if is_in_tmux() {
        "switch-client"
    } else {
        "attach-session"
    };
    if !run_interactive(&[verb, "-t", name])? {
        bail!("failed to {verb} to '{name}'");
    }
    Ok(())
}
