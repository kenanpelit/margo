//! `mshellctl session …` — session/power actions over D-Bus (the same live
//! IPC as `mshellctl menu session`, promoted to a top-level command since
//! logout/reboot/shutdown are power actions, not menus).

use crate::bus::bus_command;
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
    let method = match command {
        SessionCommands::Menu => "Session",
        SessionCommands::Lock => "SessionLock",
        SessionCommands::Logout => "SessionLogout",
        SessionCommands::Suspend => "SessionSuspend",
        SessionCommands::Reboot => "SessionReboot",
        SessionCommands::Shutdown => "SessionShutdown",
    };
    bus_command(method).await?;
    Ok(())
}
