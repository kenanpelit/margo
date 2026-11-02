//! `mshellctl session …` — session/power actions over D-Bus (the same live
//! IPC as `mshellctl menu session`, promoted to a top-level command since
//! logout/reboot/shutdown are power actions, not menus).

use crate::bus::shell;
use clap::Subcommand;

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
}

pub async fn execute(command: SessionCommands) -> anyhow::Result<()> {
    let shell = shell().await?;
    match command {
        SessionCommands::Menu => shell.session().await?,
        SessionCommands::Lock => shell.session_lock().await?,
        SessionCommands::Logout => shell.session_logout().await?,
        SessionCommands::Suspend => shell.session_suspend().await?,
        SessionCommands::Reboot => shell.session_reboot().await?,
        SessionCommands::Shutdown => shell.session_shutdown().await?,
    }
    Ok(())
}
