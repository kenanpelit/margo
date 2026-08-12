//! `mtm session ...` — create/list/kill/attach/layout/term. Rust port of
//! `tm.sh`'s SESSION MANAGEMENT + LAYOUT FUNCTIONS sections.

use crate::config::Config;
use crate::tmux;
use anyhow::{Result, bail};
use clap::Subcommand;
use std::io::Write;
use std::process::Command;

#[derive(Subcommand, Debug)]
pub enum SessionCommands {
    /// Create a new session (or attach if it already exists)
    #[command(aliases = ["c"])]
    Create {
        /// Defaults to the current directory / git worktree name
        name: Option<String>,
        /// Apply a layout template (1-5) to a freshly created session
        layout: Option<u8>,
    },
    /// List every tmux session
    #[command(aliases = ["l", "ls"])]
    List,
    /// Terminate a session
    #[command(aliases = ["k"])]
    Kill { name: String },
    /// Attach to (or switch to) an existing session
    #[command(aliases = ["a"])]
    Attach { name: String },
    /// Apply a panel layout template (1-5) to an existing session
    #[command(aliases = ["lo"])]
    Layout { name: String, number: u8 },
    /// Open a session in a new terminal window (kitty/alacritty)
    #[command(aliases = ["t"])]
    Term {
        terminal: String,
        name: String,
        layout: Option<u8>,
    },
}

pub fn run(cmd: SessionCommands, cfg: &Config) -> Result<()> {
    tmux_required()?;
    match cmd {
        SessionCommands::Create { name, layout } => {
            let name = name.unwrap_or_else(tmux::session_name_from_cwd);
            create_session(&name, layout, cfg)
        }
        SessionCommands::List => list_sessions(),
        SessionCommands::Kill { name } => kill_session(&name),
        SessionCommands::Attach { name } => {
            if !tmux::has_session_exact(&name) {
                bail!("session '{name}' not found");
            }
            tmux::attach_or_switch(&name)
        }
        SessionCommands::Layout { name, number } => apply_layout(&name, number, cfg),
        SessionCommands::Term {
            terminal,
            name,
            layout,
        } => open_in_terminal(&terminal, &name, layout.unwrap_or(1)),
    }
}

fn tmux_required() -> Result<()> {
    if !tmux::installed() {
        bail!("tmux is not installed");
    }
    Ok(())
}

/// List every session with window count / attached count, mirroring
/// `tm.sh`'s `list_sessions` format string.
fn list_sessions() -> Result<()> {
    let out = tmux::run(&[
        "list-sessions",
        "-F",
        "#{session_name}: #{session_windows} window(s), #{session_attached} attached#{?session_grouped, (grouped),}",
    ]);
    match out {
        Ok(text) => {
            println!("Sessions:");
            for line in text.lines() {
                println!("  • {line}");
            }
        }
        Err(_) => println!("No active sessions"),
    }
    Ok(())
}

fn kill_session(name: &str) -> Result<()> {
    if !tmux::has_session_exact(name) {
        bail!("session '{name}' not found");
    }

    if tmux::is_in_tmux() {
        let current = tmux::run(&["display-message", "-p", "#S"])
            .unwrap_or_default()
            .trim()
            .to_string();
        if current == name {
            print!("You are currently inside this session. Kill it anyway? (y/N): ");
            std::io::stdout().flush().ok();
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer).ok();
            if !answer.trim().eq_ignore_ascii_case("y") {
                println!("Cancelled");
                return Ok(());
            }
        }
    }

    if tmux::ok(&["kill-session", "-t", name]) {
        println!("Session '{name}' killed");
        return Ok(());
    }

    // First attempt failed — try a socket cleanup + retry, matching
    // tm.sh's fallback.
    clean_sockets();
    if tmux::ok(&["kill-session", "-t", name]) {
        println!("Session '{name}' killed (after socket cleanup)");
        Ok(())
    } else {
        bail!("failed to kill session '{name}'")
    }
}

/// Create `name` (or attach if it already exists). Existing-but-detached
/// sessions attach straight away; existing sessions already attached
/// elsewhere get a fresh window first (matches `tm.sh create_session`'s
/// "someone else is using this session" case).
fn create_session(name: &str, layout: Option<u8>, cfg: &Config) -> Result<()> {
    tmux::validate_session_name(name)?;

    if tmux::has_session_exact(name) {
        println!("Session '{name}' already exists, attaching...");
        if !tmux::is_in_tmux() {
            let attached = tmux::run(&["list-sessions"])
                .unwrap_or_default()
                .lines()
                .any(|l| l.starts_with(&format!("{name}: ")) && l.contains("(attached)"));
            if attached {
                println!("Session is attached elsewhere, opening a new window...");
                let _ = tmux::ok(&["new-window", "-t", name]);
            }
        }
        return tmux::attach_or_switch(name);
    }

    println!("Creating session '{name}'...");
    if !tmux::ok(&["new-session", "-d", "-s", name]) {
        clean_sockets();
        if !tmux::ok(&["new-session", "-d", "-s", name]) {
            bail!("failed to create session '{name}' even after socket cleanup");
        }
    }

    if let Some(n) = layout {
        create_layout(name, n, cfg)?;
    }

    println!("Session created, attaching...");
    tmux::attach_or_switch(name)
}

fn apply_layout(name: &str, number: u8, cfg: &Config) -> Result<()> {
    if !tmux::has_session_exact(name) {
        bail!("session '{name}' not found");
    }
    create_layout(name, number, cfg)
}

/// Panel layout templates 1-5, ported verbatim from `tm.sh create_layout`.
fn create_layout(name: &str, number: u8, cfg: &Config) -> Result<()> {
    let cwd = if cfg.layout_cwd.is_empty() {
        std::env::var("HOME").unwrap_or_default()
    } else {
        cfg.layout_cwd.clone()
    };

    println!("Building layout {number} for session '{name}'...");
    let win = |c: &str| tmux::ok(&["new-window", "-t", name, "-n", "kenp", "-c", c]);
    let split =
        |axis: &str, pct: &str, c: &str| tmux::ok(&["split-window", axis, "-l", pct, "-c", c]);
    let select = |pane: &str| tmux::ok(&["select-pane", "-t", pane]);

    match number {
        1 => {
            win(&cwd);
            select("1");
        }
        2 => {
            win(&cwd);
            split("-v", "80%", &cwd);
            select("2");
        }
        3 => {
            win(&cwd);
            split("-h", "80%", &cwd);
            select("2");
            split("-v", "85%", &cwd);
            select("3");
        }
        4 => {
            win(&cwd);
            split("-h", "80%", &cwd);
            split("-v", "80%", &cwd);
            select("1");
            split("-v", "80%", &cwd);
            select("4");
        }
        5 => {
            win(&cwd);
            split("-h", "70%", &cwd);
            split("-h", "50%", &cwd);
            select("1");
            split("-v", "50%", &cwd);
            select("2");
            split("-v", "50%", &cwd);
            select("5");
        }
        _ => bail!("invalid layout number: {number} (must be 1-5)"),
    }

    println!("Layout {number} built");
    Ok(())
}

/// Open a session in a fresh terminal window, spawning `mtm session
/// create <name> <layout>` inside it — `tm.sh`'s `open_session_in_terminal`,
/// pointed at our own binary instead of re-invoking a shell script path.
fn open_in_terminal(terminal: &str, name: &str, layout: u8) -> Result<()> {
    let class = format!("tmux-{name}");
    let title = format!("Tmux: {name}");
    let cwd = std::env::current_dir().unwrap_or_default();
    let self_exe = std::env::current_exe().unwrap_or_else(|_| "mtm".into());

    let spawned = match terminal {
        "kitty" => Command::new("kitty")
            .args(["--class", &class, "--title", &title])
            .arg("--directory")
            .arg(&cwd)
            .arg("-e")
            .arg(&self_exe)
            .args(["session", "create", name, &layout.to_string()])
            .spawn(),
        "alacritty" => Command::new("alacritty")
            .args(["--class", &class, "--title", &title])
            .arg("--working-directory")
            .arg(&cwd)
            .arg("-e")
            .arg(&self_exe)
            .args(["session", "create", name, &layout.to_string()])
            .spawn(),
        other => bail!("unsupported terminal '{other}' (supported: kitty, alacritty)"),
    };

    spawned.map_err(|e| anyhow::anyhow!("failed to launch {terminal}: {e}"))?;
    println!("Terminal launched: '{name}'");
    Ok(())
}

/// Remove stale tmux sockets under `/tmp/tmux-$UID` and kill any lingering
/// server — `tm.sh`'s `clean_sockets`, the fallback path when a
/// create/kill fails the first time.
pub fn clean_sockets() {
    let uid = unsafe { libc::getuid() };
    let dir = std::path::PathBuf::from(format!("/tmp/tmux-{uid}"));
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let _ = Command::new("tmux").arg("kill-server").output();
    std::thread::sleep(std::time::Duration::from_secs(1));
}
