//! `mshellctl layout …` — saved tiling-layout snapshots, proxied to `mlayout`
//! (the layout file parser/applier).

use crate::subcommands::proxy;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum LayoutCommands {
    /// List saved layouts.
    List,
    /// Print the current layout.
    Current,
    /// Apply a saved layout by name.
    Set {
        /// Layout name.
        name: String,
    },
    /// Apply the next saved layout.
    Next,
    /// Apply the previous saved layout.
    Prev,
    /// Preview a layout without applying it.
    Preview {
        /// Layout name.
        name: String,
    },
    /// Interactive layout picker.
    Pick,
    /// Any other `mlayout` subcommand passes through — e.g.
    /// `layout new`, `layout init`, `layout suggest`, `layout outputs`.
    #[command(external_subcommand)]
    Exec(Vec<String>),
}

pub async fn execute(command: LayoutCommands) -> anyhow::Result<()> {
    match command {
        LayoutCommands::List => proxy::run("mlayout", ["list"]),
        LayoutCommands::Current => proxy::run("mlayout", ["current"]),
        LayoutCommands::Set { name } => proxy::run("mlayout", ["set", &name]),
        LayoutCommands::Next => proxy::run("mlayout", ["next"]),
        LayoutCommands::Prev => proxy::run("mlayout", ["prev"]),
        LayoutCommands::Preview { name } => proxy::run("mlayout", ["preview", &name]),
        LayoutCommands::Pick => proxy::run("mlayout", ["pick"]),
        LayoutCommands::Exec(args) => proxy::run("mlayout", &args),
    }
}
