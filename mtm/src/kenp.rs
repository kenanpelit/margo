//! Default-session mode — `mtm` with no arguments. Rust port of `tm.sh`'s
//! "KENP SESSION MODE" section: a single-owner dev session that coordinates
//! with `anka` (an external snapshot-restore tool) instead of racing it.
//!
//! - autorestore ON + a snapshot already contains this session → anka owns
//!   creating it; we just wait (bounded) and attach.
//! - autorestore OFF / no snapshot / session missing from it → we own it,
//!   no wait.

use crate::config::Config;
use crate::tmux;
use anyhow::{Result, bail};
use std::process::Command;
use std::time::Duration;

/// True if `anka` is about to (re)create `session_name` from a snapshot on
/// this same tmux server start, so we should wait for it instead of racing
/// a `new-session` against its restore. Every check here fails *open*
/// (returns `false`, i.e. "don't wait") on anything unexpected — a missing
/// option, a missing snapshot file, anka not installed at all — so an
/// `anka`-less system behaves exactly as if this function didn't exist.
fn anka_restore_pending(session_name: &str) -> bool {
    let restore_on_start =
        tmux::run(&["show-options", "-gqv", "@anka-restore-on-start"]).unwrap_or_default();
    let restore_on_start = restore_on_start.trim();
    let enabled = matches!(restore_on_start, "" | "on" | "1" | "true" | "yes");
    if !enabled {
        return false;
    }

    let dir = tmux::run(&["show-options", "-gqv", "@anka-dir"])
        .unwrap_or_default()
        .trim()
        .to_string();
    let dir = if dir.is_empty() {
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_default()
                    .join(".local")
                    .join("share")
            });
        data_home.join("tmux").join("anka")
    } else if let Some(rest) = dir.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join(rest)
    } else {
        std::path::PathBuf::from(dir)
    };

    let snapshot = dir.join("snapshots").join("last").join("snapshot.json");
    let Ok(contents) = std::fs::read_to_string(&snapshot) else {
        return false;
    };
    contents.contains(&format!("\"name\": \"{session_name}\""))
}

/// `mtm` with no arguments, or `mtm kenp [name]` — attach to the default
/// session, coordinating with `anka` if it looks like it's about to
/// restore this exact session on its own.
pub fn default_session(name: Option<&str>, cfg: &Config) -> Result<()> {
    if !tmux::installed() {
        bail!("tmux is not installed");
    }
    let name = name.unwrap_or(&cfg.default_session);
    tmux::validate_session_name(name)?;
    println!("Starting session '{name}'...");

    // 1) Already there — just attach.
    if tmux::has_session_exact(name) {
        return tmux::attach_or_switch(name);
    }

    // 2) Idempotent start — also what triggers anka's restore-on-start.
    let _ = Command::new("tmux").arg("start-server").output();

    // 3) If anka is about to restore this exact session, wait briefly
    // instead of racing it with our own new-session.
    if cfg.anka_integration && anka_restore_pending(name) {
        println!("Waiting for anka to restore '{name}' from a snapshot...");
        for _ in 0..50 {
            if tmux::has_session_exact(name) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // 4) Still missing (restore off / no snapshot / timed out) — ours to
    // create. `-A` makes this idempotent even if anka just barely won the
    // race: attaches to what's there instead of erroring "duplicate session".
    if !tmux::has_session_exact(name) {
        let created = Command::new("tmux")
            .args(["new-session", "-A", "-d", "-s", name, "-n", "terminal"])
            .output();
        if !created.map(|o| o.status.success()).unwrap_or(false) {
            bail!("failed to create session '{name}'");
        }
        println!("Session '{name}' ready");
    }

    // 5) A freshly anka-restored pane should never have unsent text in it;
    // if it does, log it (forensics) and clear the line before attaching,
    // without the user ever seeing it.
    if let Ok(stray) = tmux::run(&["capture-pane", "-p", "-t", name])
        && !stray.trim().is_empty()
    {
        log_stray_pane(name, &stray);
    }
    let _ = tmux::ok(&["send-keys", "-t", name, "C-u"]);

    tmux::attach_or_switch(name)
}

fn log_stray_pane(session_name: &str, contents: &str) {
    let cache_dir = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_default()
                .join(".cache")
        });
    let log_path = cache_dir.join("tm-stray-pane.log");
    let now = std::process::Command::new("date")
        .arg("+%F %T")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let entry = format!("=== {now} session={session_name} ===\n{contents}\n---\n");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = f.write_all(entry.as_bytes());
    }
}
