//! `mshellctl dock …` — control the standalone mdock surface.

use crate::bus::{bus_command, bus_command_with_arg};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum DockCommands {
    /// Toggle the standalone dock surface.
    Toggle,
    /// Show it.
    Show,
    /// Hide it.
    Hide,
    /// Focus (or launch) the Nth pinned app — N is 1-based, in dock order.
    /// Bind to a hotkey, e.g. `bind=SUPER ALT,1,spawn,mshellctl dock activate 1`.
    Activate {
        /// 1-based index of the pinned app.
        index: u32,
    },
}

pub async fn execute(command: DockCommands) -> anyhow::Result<()> {
    match command {
        DockCommands::Toggle => bus_command("DockToggle").await?,
        DockCommands::Show => bus_command("DockShow").await?,
        DockCommands::Hide => bus_command("DockHide").await?,
        DockCommands::Activate { index } => bus_command_with_arg("DockActivate", &index).await?,
    }
    Ok(())
}
