//! `mshellctl notification …` — notification-centre actions over D-Bus (the
//! same live IPC as `mshellctl menu notifications`, promoted to a top-level
//! command for scripts and keybinds).

use crate::bus::shell;
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
        NotificationCommands::Open => shell().await?.notifications().await?,
        NotificationCommands::Clear => shell().await?.notifications_clear_all().await?,
        NotificationCommands::Read => shell().await?.notifications_read_popups().await?,
        NotificationCommands::Dnd { state } => match state {
            Some(DndState::On) => shell().await?.notification_dnd_on().await?,
            Some(DndState::Off) => shell().await?.notification_dnd_off().await?,
            None => shell().await?.notification_dnd_toggle().await?,
        },
        NotificationCommands::Count => {
            let count: u32 = shell().await?.notification_count().await?;
            println!("{count}");
        }
    }
    Ok(())
}
