//! `mshellctl dock …` — control the standalone mdock surface.

use crate::bus::shell;
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
        DockCommands::Toggle => shell().await?.dock_toggle().await?,
        DockCommands::Show => shell().await?.dock_show().await?,
        DockCommands::Hide => shell().await?.dock_hide().await?,
        DockCommands::Activate { index } => shell().await?.dock_activate(index).await?,
    }
    Ok(())
}
