//! `mtm config ...` — tmux config backup/restore. Rust port of `tm.sh`'s
//! CONFIGURATION BACKUP/RESTORE section.

use anyhow::{Result, bail};
use clap::Subcommand;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Archive ~/.config/tmux (+ ~/.cache/tmux-manager) to ~/tmux_backup_<timestamp>.tar.gz
    #[command(aliases = ["b"])]
    Backup,
    /// Restore from a backup archive (defaults to the newest one in $HOME)
    #[command(aliases = ["r"])]
    Restore { path: Option<String> },
}

pub fn run(cmd: ConfigCommands) -> Result<()> {
    match cmd {
        ConfigCommands::Backup => backup(),
        ConfigCommands::Restore { path } => restore(path.as_deref()),
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn backup() -> Result<()> {
    let stamp = Command::new("date")
        .arg("+%Y%m%d_%H%M%S")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let backup_path = home().join(format!("tmux_backup_{stamp}.tar.gz"));

    println!("Backing up tmux configuration...");
    let status = Command::new("tar")
        .arg("czf")
        .arg(&backup_path)
        .arg("-C")
        .arg(home())
        .args([".config/tmux", ".cache/tmux-manager"])
        .status()?;

    if !status.success() {
        bail!("backup failed");
    }

    println!("Backup created: {}", backup_path.display());
    if let Ok(meta) = std::fs::metadata(&backup_path) {
        println!("Backup size: {} bytes", meta.len());
    }
    Ok(())
}

fn restore(path: Option<&str>) -> Result<()> {
    let backup_path = match path {
        Some(p) => PathBuf::from(p),
        None => match latest_backup()? {
            Some(p) => p,
            None => {
                bail!("no backup file found in {}", home().display());
            }
        },
    };

    if !backup_path.is_file() {
        bail!("backup file not found: {}", backup_path.display());
    }

    println!("This will overwrite your current tmux configuration!");
    print!("Continue? (y/N): ");
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).ok();
    if !answer.trim().eq_ignore_ascii_case("y") {
        println!("Cancelled");
        return Ok(());
    }

    println!("Restoring configuration...");
    let status = Command::new("tar")
        .arg("xzf")
        .arg(&backup_path)
        .arg("-C")
        .arg(home())
        .status()?;

    if status.success() {
        println!("Configuration restored");
        Ok(())
    } else {
        bail!("restore failed")
    }
}

/// Newest `tmux_backup_*.tar.gz` directly under `$HOME`, by mtime.
fn latest_backup() -> Result<Option<PathBuf>> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(home())?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("tmux_backup_") || !name.ends_with(".tar.gz") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(mtime) = meta.modified() else { continue };
        if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
            best = Some((mtime, entry.path()));
        }
    }
    Ok(best.map(|(_, p)| p))
}
