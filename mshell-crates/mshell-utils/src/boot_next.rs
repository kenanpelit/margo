//! "Reboot into…": pick the UEFI entry the next boot starts, then reboot.
//!
//! The entries come from `efibootmgr`'s unprivileged listing (efivarfs
//! is world-readable), parsed by [`mshell_privileged::efiboot`] — the
//! same parser the root helper runs on its own listing before it sets
//! `BootNext`. Setting or clearing `BootNext` is the helper's fixed
//! `boot-next` verb behind its own polkit action; the reboot itself is
//! the session menu's Reboot ([`crate::session`]), so a configured
//! `session.reboot_command` and margo's clean window shutdown still apply.

pub use mshell_privileged::BootTarget;
use mshell_privileged::Request;
pub use mshell_privileged::efiboot::{self, BootEntry, BootList};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// How long the listing may take (efibootmgr reads a few dozen variables).
const LIST_TIMEOUT: Duration = Duration::from_secs(5);

/// Why the list couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListError {
    /// `efibootmgr` isn't installed.
    Missing,
    /// It ran and failed (legacy BIOS boot, no efivarfs, timeout).
    Failed(String),
}

impl ListError {
    pub fn message(&self) -> String {
        match self {
            ListError::Missing => "efibootmgr isn't installed (package efibootmgr)".to_string(),
            ListError::Failed(e) => e.clone(),
        }
    }
}

/// Read `efibootmgr`'s listing (off the GTK thread: it's a subprocess).
pub async fn read_list() -> Result<BootList, ListError> {
    if !margo_fs::path::on_path("efibootmgr") {
        return Err(ListError::Missing);
    }
    let run = Command::new("efibootmgr")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let out = match tokio::time::timeout(LIST_TIMEOUT, run).await {
        Err(_) => return Err(ListError::Failed("efibootmgr timed out".into())),
        Ok(Err(e)) => return Err(ListError::Failed(format!("efibootmgr: {e}"))),
        Ok(Ok(out)) => out,
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().map(str::trim).find(|l| !l.is_empty());
        return Err(ListError::Failed(
            line.unwrap_or("efibootmgr could not read the boot entries")
                .to_string(),
        ));
    }
    Ok(efiboot::parse(&String::from_utf8_lossy(&out.stdout)))
}

/// The pkexec argv for `target` (validated the way root will check it).
pub fn pkexec_argv(target: BootTarget) -> Result<Vec<String>, String> {
    let req = Request::BootNext { target }
        .validated()
        .map_err(|e| e.message().to_string())?;
    Ok(mshell_privileged::lossy(&req.argv()))
}

/// Set (or clear) `BootNext` through the helper; one polkit prompt.
pub async fn set(target: BootTarget) -> Result<(), String> {
    let argv = pkexec_argv(target)?;
    if !mshell_privileged::installed() {
        return Err(mshell_privileged::MISSING.to_string());
    }
    let out = Command::new("pkexec")
        .args(&argv)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("pkexec unavailable: {e}"))?;
    mshell_privileged::outcome(out.status.code(), &String::from_utf8_lossy(&out.stderr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_argv_is_the_helpers_fixed_verb() {
        assert_eq!(
            pkexec_argv(BootTarget::Entry(3)).unwrap(),
            [mshell_privileged::HELPER, "boot-next", "0003"]
        );
        assert_eq!(
            pkexec_argv(BootTarget::Clear).unwrap(),
            [mshell_privileged::HELPER, "boot-next", "none"]
        );
    }
}
