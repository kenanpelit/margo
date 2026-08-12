//! `mtm buffer ...` — tmux paste-buffer management. Rust port of `tm.sh`'s
//! BUFFER MANAGEMENT section.

use crate::config::Config;
use crate::{fzf, tmux};
use anyhow::{Result, bail};
use clap::Subcommand;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Subcommand, Debug)]
pub enum BufferCommands {
    /// Raw `tmux list-buffers` passthrough
    #[command(aliases = ["l", "ls"])]
    List,
    /// Pick a buffer (fzf) and copy it to the system clipboard
    #[command(aliases = ["s"])]
    Show,
}

pub fn run(cmd: Option<BufferCommands>, cfg: &Config) -> Result<()> {
    if !tmux::is_in_tmux() {
        bail!("not inside a tmux session");
    }
    match cmd.unwrap_or(BufferCommands::Show) {
        BufferCommands::List => {
            print!("{}", tmux::run(&["list-buffers"])?);
            Ok(())
        }
        BufferCommands::Show => show(cfg),
    }
}

fn show(cfg: &Config) -> Result<()> {
    let listed =
        tmux::run(&["list-buffers", "-F", "#{buffer_name}: #{buffer_sample}"]).unwrap_or_default();
    if listed.trim().is_empty() {
        println!("No buffers");
        return Ok(());
    }

    let selection = fzf::pick(
        &cfg.fzf_theme,
        "Buffer",
        "ENTER: Copy | CTRL-D: Delete | ESC: Cancel",
        &[
            "--delimiter=: ",
            "--preview",
            "tmux show-buffer -b {1}",
            "--preview-window=up:70%:wrap",
            "--bind",
            "ctrl-d:execute(tmux delete-buffer -b {1})+reload(tmux list-buffers -F \"#{buffer_name}: #{buffer_sample}\")",
        ],
        &listed,
    )?;

    let Some(selection) = selection else {
        return Ok(());
    };
    let Some(buffer_name) = selection.split(':').next() else {
        return Ok(());
    };

    let contents = tmux::run(&["show-buffer", "-b", buffer_name])?;
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn wl-copy: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(contents.as_bytes()).ok();
    }
    if child.wait()?.success() {
        println!("Buffer copied: {buffer_name}");
        Ok(())
    } else {
        bail!("failed to copy buffer to clipboard")
    }
}
