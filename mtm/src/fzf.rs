//! Shared themed `fzf` spawn helper — `tm.sh`'s `fzf_themed`. `fzf` reads
//! its candidate list from stdin and paints its UI straight to the
//! controlling tty (not stdout), so piping a list in and capturing the
//! final selection from stdout works the same from Rust as it did from
//! bash — including `--bind execute(...)/reload(...)` hooks, which fzf
//! runs through `$SHELL` regardless of what spawned it.

use crate::config::FzfTheme;
use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Run `fzf` over `input` (one candidate per line) with the themed args
/// plus `extra_args`. Returns `None` if the user cancelled (Esc / no
/// stdout) rather than an error — cancelling a picker isn't a failure.
pub fn pick(
    theme: &FzfTheme,
    prompt: &str,
    header: &str,
    extra_args: &[&str],
    input: &str,
) -> Result<Option<String>> {
    let mut cmd = Command::new("fzf");
    cmd.args([
        "-e",
        "-i",
        "--info=inline",
        "--layout=reverse",
        "--border=rounded",
        "--margin=1",
        "--padding=1",
        "--ansi",
        "--pointer=▶",
        "--marker=✓",
        "--tiebreak=index",
    ]);
    cmd.arg(format!(
        "--color=bg+:{},bg:{},spinner:{},hl:{}",
        theme.bg_plus, theme.bg, theme.spinner, theme.hl
    ));
    cmd.arg(format!(
        "--color=fg:{},header:{},info:{},pointer:{}",
        theme.fg, theme.header, theme.info, theme.pointer
    ));
    cmd.arg(format!(
        "--color=marker:{},fg+:{},prompt:{},hl+:{}",
        theme.marker, theme.fg_plus, theme.prompt, theme.hl_plus
    ));
    cmd.arg(format!("--prompt={prompt} "));
    cmd.arg(format!("--header={header}"));
    cmd.args(extra_args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());

    let mut child = cmd.spawn().context("spawn fzf (is it installed?)")?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let out = child.wait_with_output().context("wait for fzf")?;
    if !out.status.success() {
        return Ok(None);
    }
    let selection = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
    if selection.is_empty() {
        Ok(None)
    } else {
        Ok(Some(selection))
    }
}
