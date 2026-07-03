//! `mshellctl notification …` — notification-centre actions over D-Bus (the
//! same live IPC as `mshellctl menu notifications`, promoted to a top-level
//! command for scripts and keybinds).

use crate::bus::{bus_command, bus_command_with_reply};
use clap::Subcommand;

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum DndState {
    On,
    Off,
}

#[derive(Subcommand, Debug)]
pub enum NotificationCommands {
    /// Open the notifications menu.
    Open,
    /// Clear every notification — history and on-screen popups.
    Clear,
    /// Dismiss the on-screen popups, keeping them in history.
    Read,
    /// Toggle Do Not Disturb, or set it explicitly with `on` / `off`.
    Dnd {
        /// on | off (omit to toggle).
        #[arg(value_enum)]
        state: Option<DndState>,
    },
    /// Print the number of notifications in history.
    Count,
}

pub async fn execute(command: NotificationCommands) -> anyhow::Result<()> {
    match command {
        NotificationCommands::Open => bus_command("Notifications").await?,
        NotificationCommands::Clear => bus_command("NotificationsClearAll").await?,
        NotificationCommands::Read => bus_command("NotificationsReadPopups").await?,
        NotificationCommands::Dnd { state } => match state {
            Some(DndState::On) => bus_command("NotificationDndOn").await?,
            Some(DndState::Off) => bus_command("NotificationDndOff").await?,
            None => bus_command("NotificationDndToggle").await?,
        },
        NotificationCommands::Count => {
            let count: u32 = bus_command_with_reply("NotificationCount").await?;
            println!("{count}");
        }
    }
    Ok(())
}
