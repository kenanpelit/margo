//! `mshellctl doctor` — one-shot health check for the desktop shell.
//!
//! The shell-side counterpart to `mctl doctor` (which covers the
//! compositor). Confirms the session bus is reachable, that the shell
//! owns `com.mshell.Shell`, and that the running shell is the same
//! version as the `mshellctl` talking to it (a stale shell after an
//! upgrade is the classic "my new feature isn't there" cause). Prints a
//! ✓ / ⚠ / ✗ line per check and exits non-zero on any error.

use std::io::IsTerminal;

use zbus::connection;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;

const SHELL_NAME: &str = "com.mshell.Shell";

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

    fn line(&mut self, level: Level, label: &str, detail: &str) {
        match level {
            Level::Ok => {}
            Level::Warn => self.warns += 1,
            Level::Err => self.errs += 1,
        }
        let (sym, color) = match level {
            Level::Ok => ('✓', "32"),
            Level::Warn => ('⚠', "33"),
            Level::Err => ('✗', "31"),
        };
        if self.colored {
            println!("  \x1b[{color}m{sym}\x1b[0m {label} — {detail}");
        } else {
            println!("  {sym} {label} — {detail}");
        }
    }
}

pub async fn execute() -> anyhow::Result<()> {
    let mut r = Report::new();
    println!("Desktop shell (mshell)");

    // 1. Session bus reachable at all?
    let conn = match connection::Builder::session() {
        Ok(builder) => match builder.build().await {
            Ok(c) => {
                r.line(Level::Ok, "session bus", "connected");
                Some(c)
            }
            Err(e) => {
                r.line(Level::Err, "session bus", &format!("connect failed: {e}"));
                None
            }
        },
        Err(e) => {
            r.line(Level::Err, "session bus", &format!("unreachable: {e}"));
            None
        }
    };

    // 2. Does the shell own its well-known name?
    let mut name_up = false;
    if let Some(conn) = &conn {
        match (DBusProxy::new(conn).await, BusName::try_from(SHELL_NAME)) {
            (Ok(dbus), Ok(name)) => match dbus.name_has_owner(name).await {
                Ok(true) => {
                    name_up = true;
                    r.line(Level::Ok, "shell service", &format!("{SHELL_NAME} is up"));
                }
                Ok(false) => r.line(
                    Level::Err,
                    "shell service",
                    &format!("{SHELL_NAME} has no owner — is mshell running?"),
                ),
                Err(e) => {
                    r.line(
                        Level::Warn,
                        "shell service",
                        &format!("name query failed: {e}"),
                    );
                }
            },
            (_, Err(e)) => r.line(Level::Warn, "shell service", &format!("bad bus name: {e}")),
            (Err(e), _) => r.line(
                Level::Warn,
                "shell service",
                &format!("D-Bus proxy failed: {e}"),
            ),
        }
    }

    // 3. Version sync — only meaningful once the service is up.
    let mine = env!("CARGO_PKG_VERSION");
    if name_up {
        match crate::bus::bus_command_with_reply::<String>("Version").await {
            Ok(running) if running == mine => r.line(
                Level::Ok,
                "version sync",
                &format!("mshell {running} == mshellctl {mine}"),
            ),
            Ok(running) if running.is_empty() => r.line(
                Level::Warn,
                "version sync",
                "running shell predates version reporting — restart mshell to update",
            ),
            Ok(running) => r.line(
                Level::Warn,
                "version sync",
                &format!(
                    "running mshell {running} != mshellctl {mine} — restart the shell to load the new build"
                ),
            ),
            Err(e) => r.line(Level::Warn, "version sync", &format!("Version call failed: {e}")),
        }
    } else {
        r.line(Level::Warn, "version sync", "skipped (shell not up)");
    }

    println!();
    println!("Tip: run `mctl doctor` for the compositor-side health check.");
    println!();
    if r.errs > 0 {
        eprintln!(
            "doctor: {} error(s), {} warning(s) — see ✗ lines above.",
            r.errs, r.warns
        );
        std::process::exit(1);
    } else if r.warns > 0 {
        println!("doctor: no errors, {} warning(s).", r.warns);
    } else {
        println!("doctor: all checks passed.");
    }
    Ok(())
}
