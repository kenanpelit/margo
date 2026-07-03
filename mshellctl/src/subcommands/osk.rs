//! `mshellctl osk …` — on-screen keyboard control, proxied to `mkeys`
//! (the zwp_virtual_keyboard layer-shell keyboard).

use crate::subcommands::proxy;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum OskCommands {
    /// Show the on-screen keyboard.
    Show,
    /// Hide the on-screen keyboard.
    Hide,
    /// Toggle the on-screen keyboard.
    Toggle,
}

pub async fn execute(command: OskCommands) -> anyhow::Result<()> {
    let verb = match command {
        OskCommands::Show => "show",
        OskCommands::Hide => "hide",
        OskCommands::Toggle => "toggle",
    };
    proxy::run("mkeys", [verb])
}
