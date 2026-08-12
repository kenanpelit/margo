//! `mtm plugin ...` — TPM (tmux plugin manager) install/update/list. Rust
//! port of `tm.sh`'s PLUGIN MANAGEMENT section.

use crate::config::plugin_dir;
use anyhow::{Result, bail};
use clap::Subcommand;
use std::io::Write;
use std::process::Command;

#[derive(Subcommand, Debug)]
pub enum PluginCommands {
    /// Clone (or, on confirmation, update) a plugin
    #[command(aliases = ["i"])]
    Install { name: String, repo_url: String },
    /// List installed plugins with their last-update time
    #[command(aliases = ["l", "ls"])]
    List,
    /// Install the recommended plugin set (tpm, sensible, resurrect, …)
    #[command(aliases = ["a"])]
    All,
}

/// The recommended set — `tm.sh install_all_plugins`'s hardcoded list.
const RECOMMENDED: &[(&str, &str)] = &[
    ("tpm", "https://github.com/tmux-plugins/tpm"),
    (
        "tmux-sensible",
        "https://github.com/tmux-plugins/tmux-sensible",
    ),
    (
        "tmux-resurrect",
        "https://github.com/tmux-plugins/tmux-resurrect",
    ),
    (
        "tmux-continuum",
        "https://github.com/tmux-plugins/tmux-continuum",
    ),
    ("tmux-yank", "https://github.com/tmux-plugins/tmux-yank"),
    (
        "tmux-copycat",
        "https://github.com/tmux-plugins/tmux-copycat",
    ),
];

pub fn run(cmd: PluginCommands) -> Result<()> {
    match cmd {
        PluginCommands::Install { name, repo_url } => install(&name, &repo_url),
        PluginCommands::List => list(),
        PluginCommands::All => install_all(),
    }
}

fn install(name: &str, repo_url: &str) -> Result<()> {
    std::fs::create_dir_all(plugin_dir())?;
    let path = plugin_dir().join(name);

    if path.is_dir() {
        print!("Plugin '{name}' already installed. Update it? (y/N): ");
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).ok();
        if answer.trim().eq_ignore_ascii_case("y") {
            let status = Command::new("git")
                .args(["-C"])
                .arg(&path)
                .arg("pull")
                .status()?;
            if status.success() {
                println!("Updated: {name}");
            } else {
                bail!("failed to update plugin '{name}'");
            }
        }
        return Ok(());
    }

    println!("Installing plugin: {name}");
    let status = Command::new("git")
        .args(["clone", repo_url])
        .arg(&path)
        .status()?;
    if status.success() {
        println!("Installed: {name}");
        Ok(())
    } else {
        bail!("failed to install plugin '{name}'")
    }
}

fn list() -> Result<()> {
    let dir = plugin_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        println!("No plugins installed");
        return Ok(());
    };

    let mut names: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();

    if names.is_empty() {
        println!("No plugins installed");
        return Ok(());
    }

    println!("Installed plugins:");
    for name in names {
        let plugin_path = dir.join(&name);
        let last_update = if plugin_path.join(".git").is_dir() {
            Command::new("git")
                .args(["-C"])
                .arg(&plugin_path)
                .args(["log", "-1", "--format=%ar"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            "unknown".to_string()
        };
        println!("  • {name} (last update: {last_update})");
    }
    Ok(())
}

fn install_all() -> Result<()> {
    println!("Installing recommended plugins...");
    let mut failed = 0;
    for (name, url) in RECOMMENDED {
        if install(name, url).is_err() {
            failed += 1;
        }
    }
    if failed == 0 {
        println!("All plugins installed successfully");
    } else {
        println!("{failed} plugin(s) failed to install");
    }
    Ok(())
}
