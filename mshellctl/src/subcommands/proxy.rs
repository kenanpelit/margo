//! Shared helper for `mshellctl` subcommands that shell out to a sibling margo
//! binary (mcal, mvpn, mkeys, mplay, mpower, mlayout, mpicker, mscreenshot).
//!
//! These tools own their own control surfaces; mshellctl re-exposes the common
//! verbs so the shell has one control CLI. Stdio is inherited (so interactive
//! flows and normal output work) and the child's exit code is propagated.

use std::ffi::OsStr;
use std::process::Command;

/// Run `bin <args>` with inherited stdio; propagate a non-zero exit.
pub fn run<I, S>(bin: &str, args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(bin)
        .args(args)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to launch {bin} (is it installed?): {e}"))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
