//! `mshellctl dock …` — control the standalone mdock surface.

use crate::bus::bus_command;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum DockCommands {
    /// Toggle the standalone dock surface.
    Toggle,
    /// Show it.
    Show,
    /// Hide it.
    Hide,
}

pub async fn execute(command: DockCommands) -> anyhow::Result<()> {
    match command {
        DockCommands::Toggle => bus_command("DockToggle").await?,
        DockCommands::Show => bus_command("DockShow").await?,
        DockCommands::Hide => bus_command("DockHide").await?,
    }
    Ok(())
}
