//! `mctl doctor` — one-shot health check for a margo install.
//!
//! Unifies the diagnostics that used to be scattered across `mctl
//! check-config`, `mctl config-errors`, the reload pre-flight, and "is
//! anything even running" guesswork. Each check prints a `✓ / ⚠ / ✗`
//! line; the command exits non-zero if any check is an outright error,
//! so it doubles as a scriptable smoke test. Everything degrades
//! gracefully — a missing optional tool is a warning, never a panic.

use std::io::IsTerminal;
use std::path::Path;
use std::process::Command;

use crate::ipc_client;

/// Outcome of a single check, in increasing severity.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Ok,
    Warn,
    Err,
}

struct Report {
    colored: bool,
    warns: u32,
    errs: u32,
}

impl Report {
    fn new() -> Self {
        Report {
            colored: std::io::stdout().is_terminal(),
            warns: 0,
            errs: 0,
        }
    }

    /// Print one check line: `  <sym> <label> — <detail>`.
    fn line(&mut self, level: Level, label: &str, detail: &str) {
        match level {
            Level::Ok => {}
            Level::Warn => self.warns += 1,
            Level::Err => self.errs += 1,
        }
        let (sym, color) = match level {
            Level::Ok => ('✓', "32"),   // green
            Level::Warn => ('⚠', "33"), // yellow
            Level::Err => ('✗', "31"),  // red
        };
        if self.colored {
            println!("  \x1b[{color}m{sym}\x1b[0m {label} — {detail}");
        } else {
            println!("  {sym} {label} — {detail}");
        }
    }

    fn section(&self, title: &str) {
        if self.colored {
            println!("\n\x1b[1m{title}\x1b[0m");
        } else {
            println!("\n{title}");
        }
    }
}

/// Run every check, print the report, and exit with a status code:
/// 0 = clean (warnings allowed), 1 = at least one error-level failure.
pub fn run() -> anyhow::Result<()> {
    let mut r = Report::new();

    r.section("Session");
    check_env(&mut r);

    r.section("Compositor (margo)");
    let snapshot = check_socket(&mut r);
    check_version_sync(&mut r, snapshot.as_ref());
    check_runtime_config_errors(&mut r, snapshot.as_ref());

    r.section("Config (on disk)");
    check_config_file(&mut r);

    r.section("Rendering & theming");
    check_render_nodes(&mut r);
    check_matugen(&mut r);

    r.section("Desktop services");
    check_dbus_service(&mut r, "com.mshell.Shell", "mshell (desktop shell)");
    check_dbus_service(&mut r, "org.freedesktop.portal.Desktop", "xdg-desktop-portal");

    // Summary.
    println!();
    if r.errs > 0 {
        eprintln!("doctor: {} error(s), {} warning(s) — see ✗ lines above.", r.errs, r.warns);
        std::process::exit(1);
    } else if r.warns > 0 {
        println!("doctor: no errors, {} warning(s).", r.warns);
    } else {
        println!("doctor: all checks passed.");
    }
    Ok(())
}

fn check_env(r: &mut Report) {
    match std::env::var("WAYLAND_DISPLAY") {
        Ok(v) if !v.is_empty() => r.line(Level::Ok, "WAYLAND_DISPLAY", &v),
        _ => r.line(Level::Warn, "WAYLAND_DISPLAY", "unset — not in a Wayland session?"),
    }
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(v) if !v.is_empty() => r.line(Level::Ok, "XDG_RUNTIME_DIR", &v),
        _ => r.line(Level::Warn, "XDG_RUNTIME_DIR", "unset"),
    }
    let sock = ipc_client::socket_path();
    let src = if std::env::var_os("MARGO_SOCKET").is_some() {
        "$MARGO_SOCKET"
    } else {
        "default"
    };
    r.line(Level::Ok, "control socket", &format!("{} ({src})", sock.display()));
}

/// Connect to margo's control socket and pull a `get state` snapshot.
/// Returns the parsed snapshot on success so later checks can reuse it.
fn check_socket(r: &mut Report) -> Option<serde_json::Value> {
    match ipc_client::request_once("get state") {
        Ok(v) if v.get("error").is_none() => {
            r.line(Level::Ok, "socket reachable", "margo is running");
            Some(v)
        }
        Ok(v) => {
            let msg = v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unexpected reply");
            r.line(Level::Err, "socket reachable", msg);
            None
        }
        Err(e) => {
            r.line(Level::Err, "socket reachable", &format!("cannot reach margo: {e}"));
            None
        }
    }
}

/// Compare the running compositor's version with mctl's own build version.
/// They share the workspace version, so a mismatch means a new margo was
/// installed but not re-logged into yet (the classic `just margo` gotcha).
fn check_version_sync(r: &mut Report, snapshot: Option<&serde_json::Value>) {
    let mine = env!("CARGO_PKG_VERSION");
    let Some(snap) = snapshot else {
        r.line(Level::Warn, "version sync", "skipped (margo unreachable)");
        return;
    };
    match snap.get("margo_version").and_then(|v| v.as_str()) {
        Some(running) if running == mine => r.line(
            Level::Ok,
            "version sync",
            &format!("margo {running} == mctl {mine}"),
        ),
        Some(running) => r.line(
            Level::Warn,
            "version sync",
            &format!("running margo {running} != mctl {mine} — re-login to load the new compositor"),
        ),
        None => r.line(
            Level::Warn,
            "version sync",
            "running margo predates version reporting — re-login to update",
        ),
    }
}

/// Whatever the *running* compositor rejected on its last (re)load.
/// Distinct from the on-disk validate below: this is the live truth.
fn check_runtime_config_errors(r: &mut Report, snapshot: Option<&serde_json::Value>) {
    let Some(snap) = snapshot else {
        r.line(Level::Warn, "runtime config", "skipped (margo unreachable)");
        return;
    };
    let n = snap
        .get("config_errors")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    if n == 0 {
        r.line(Level::Ok, "runtime config", "loaded clean");
    } else {
        r.line(
            Level::Warn,
            "runtime config",
            &format!("{n} line(s) rejected on last load — run `mctl config-errors`"),
        );
    }
}

/// Validate the on-disk config file the way `mctl reload` does pre-flight.
fn check_config_file(r: &mut Report) {
    match margo_config::validator::validate_config(None) {
        Ok(report) if report.has_errors() => r.line(
            Level::Err,
            "config validates",
            &format!(
                "{} error(s) — run `mctl check-config` for details",
                report.errors().count()
            ),
        ),
        Ok(report) if report.has_warnings() => r.line(
            Level::Warn,
            "config validates",
            &format!("{} warning(s) — run `mctl check-config`", report.warnings().count()),
        ),
        Ok(_) => r.line(Level::Ok, "config validates", "no errors or warnings"),
        Err(e) => r.line(Level::Warn, "config validates", &format!("could not read config: {e}")),
    }
}

/// DRM render nodes — without at least one, the GPU render path is dead.
fn check_render_nodes(r: &mut Report) {
    let dri = Path::new("/dev/dri");
    let nodes: Vec<String> = std::fs::read_dir(dri)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("renderD"))
        .collect();
    if nodes.is_empty() {
        r.line(Level::Warn, "GPU render node", "none found — software rendering only");
    } else {
        r.line(Level::Ok, "GPU render node", &nodes.join(", "));
    }
}

/// matugen powers Material-You theme-from-wallpaper. Optional, but the
/// `Wallpaper` colour scheme silently no-ops without it.
fn check_matugen(r: &mut Report) {
    if which("matugen") {
        r.line(Level::Ok, "matugen", "on PATH");
    } else {
        r.line(
            Level::Warn,
            "matugen",
            "not on PATH — the Wallpaper colour scheme won't generate a palette",
        );
    }
}

/// Best-effort session-bus name check via `busctl`. We don't depend on
/// zbus here (mctl is the compositor tool), so this shells out; a missing
/// busctl is reported as a skip rather than a failure.
fn check_dbus_service(r: &mut Report, name: &str, label: &str) {
    if !which("busctl") {
        r.line(Level::Warn, label, "skipped (busctl not found)");
        return;
    }
    let ok = Command::new("busctl")
        .args(["--user", "status", name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        r.line(Level::Ok, label, &format!("{name} is up"));
    } else {
        r.line(Level::Warn, label, &format!("{name} not on the session bus"));
    }
}

/// Is `bin` resolvable on `$PATH`?
fn which(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(bin).is_file())
}
