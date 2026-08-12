//! `mtm clip` — clipboard history picker. Rust port of `tm.sh`'s
//! CLIPBOARD MANAGEMENT section.
//!
//! Two backends, auto-detected (override with `$MTM_CLIPBOARD_BACKEND`):
//! - `cliphist` + `wl-copy`/`wl-paste` (preferred — fzf picker + preview)
//! - `clipse` (has its own TUI; mtm just launches it)

use crate::config::Config;
use crate::{fzf, tmux};
use anyhow::{Result, bail};
use std::io::Write;
use std::process::{Command, Stdio};

fn command_exists(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

enum Backend {
    Cliphist,
    Clipse,
}

fn resolve_backend() -> Result<Backend> {
    match std::env::var("MTM_CLIPBOARD_BACKEND").as_deref() {
        Ok("cliphist") => return Ok(Backend::Cliphist),
        Ok("clipse") => return Ok(Backend::Clipse),
        Ok(other) if !other.is_empty() && other != "auto" => {
            bail!("unknown clipboard backend: {other}")
        }
        _ => {}
    }
    if command_exists("cliphist") && command_exists("wl-copy") {
        Ok(Backend::Cliphist)
    } else if command_exists("clipse") {
        Ok(Backend::Clipse)
    } else {
        bail!(
            "no clipboard backend available — install cliphist + wl-clipboard (recommended), or clipse"
        )
    }
}

pub fn run(cfg: &Config) -> Result<()> {
    let _ = tmux::is_in_tmux(); // clip mode doesn't require tmux, unlike buffer
    match resolve_backend()? {
        Backend::Cliphist => show_cliphist(cfg),
        Backend::Clipse => {
            let status = Command::new("clipse")
                .status()
                .map_err(|e| anyhow::anyhow!("failed to launch clipse: {e}"))?;
            if status.success() {
                Ok(())
            } else {
                bail!("clipse exited with an error")
            }
        }
    }
}

fn show_cliphist(cfg: &Config) -> Result<()> {
    let listed = Command::new("cliphist")
        .arg("list")
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run cliphist list: {e}"))?;
    let listed = String::from_utf8_lossy(&listed.stdout).into_owned();
    if listed.trim().is_empty() {
        println!("Clipboard history is empty");
        return Ok(());
    }

    let selection = fzf::pick(
        &cfg.fzf_theme,
        "Clipboard",
        "ENTER: Paste | CTRL-D: Delete | ESC: Cancel",
        &[
            "--preview",
            "echo {} | cliphist decode",
            "--preview-window=up:70%:wrap",
            "--bind",
            "ctrl-d:execute(echo {} | cliphist delete)+reload(cliphist list)",
        ],
        &listed,
    )?;

    let Some(selection) = selection else {
        return Ok(());
    };

    let mut decode = Command::new("cliphist")
        .arg("decode")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn cliphist decode: {e}"))?;
    if let Some(mut stdin) = decode.stdin.take() {
        stdin.write_all(selection.as_bytes()).ok();
    }
    let decoded = decode.wait_with_output()?;
    if !decoded.status.success() {
        bail!("cliphist decode failed");
    }

    let mut copy = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn wl-copy: {e}"))?;
    if let Some(mut stdin) = copy.stdin.take() {
        stdin.write_all(&decoded.stdout).ok();
    }
    if copy.wait()?.success() {
        println!("Copied to clipboard");
        Ok(())
    } else {
        bail!("failed to copy to clipboard")
    }
}
