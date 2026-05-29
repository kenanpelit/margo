use crate::bus::bus_command_with_arg;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum PluginCommands {
    /// Force-reload an installed plugin's WASM panel — evicts the cached
    /// instance so the next open instantiates from disk. Wire your
    /// `cargo watch` to call this for a fast edit→pixels iteration loop
    /// without restarting mshell.
    Reload {
        /// The plugin key (its id, or a widget key).
        key: String,
    },
}

pub async fn execute(command: PluginCommands) -> anyhow::Result<()> {
    match command {
        PluginCommands::Reload { key } => {
            bus_command_with_arg("PluginReload", &key).await?;
        }
    }
    Ok(())
}
