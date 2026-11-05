//! `mshellctl session …` — session/power actions over D-Bus (the same live
//! IPC as `mshellctl menu session`, promoted to a top-level command since
//! logout/reboot/shutdown are power actions, not menus).

use crate::bus::shell;
use clap::Subcommand;
use mshell_privileged::efiboot;

#[derive(Subcommand, Debug)]
pub enum SessionCommands {
    /// Open the session / power menu.
    Menu,
    /// Lock the session.
    Lock,
    /// Log out of the session.
    Logout,
    /// Suspend the system.
    Suspend,
    /// Reboot the system.
    Reboot,
    /// Power off the system.
    Shutdown,
    /// List the UEFI boot entries `reboot-into` takes (efibootmgr).
    BootEntries,
    /// Reboot into one UEFI boot entry for the next boot only (BootNext,
    /// through a polkit prompt). ENTRY is its number (`0003`) or exact
    /// label (`cachyos-nvme0`); see `mshellctl session boot-entries`.
    RebootInto {
        /// Entry number or label.
        entry: String,
    },
}

pub async fn execute(command: SessionCommands) -> anyhow::Result<()> {
    if let SessionCommands::BootEntries = command {
        return boot_entries();
    }
    let shell = shell().await?;
    match command {
        SessionCommands::Menu => shell.session().await?,
        SessionCommands::Lock => shell.session_lock().await?,
        SessionCommands::Logout => shell.session_logout().await?,
        SessionCommands::Suspend => shell.session_suspend().await?,
        SessionCommands::Reboot => shell.session_reboot().await?,
        SessionCommands::Shutdown => shell.session_shutdown().await?,
        SessionCommands::RebootInto { entry } => shell.session_reboot_into(&entry).await?,
        SessionCommands::BootEntries => {}
    }
    Ok(())
}

/// Print efibootmgr's entries locally (no shell needed): one per line,
/// `0003  cachyos-nvme0  (next boot)`.
fn boot_entries() -> anyhow::Result<()> {
    if !margo_fs::path::on_path("efibootmgr") {
        anyhow::bail!("efibootmgr isn't installed (package efibootmgr)");
    }
    let out = std::process::Command::new("efibootmgr")
        .stdin(std::process::Stdio::null())
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "efibootmgr failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let list = efiboot::parse(&String::from_utf8_lossy(&out.stdout));
    for e in &list.entries {
        println!("{}", efiboot::describe(&list, e));
    }
    Ok(())
}
