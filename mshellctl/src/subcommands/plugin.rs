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
    /// Fire a registered plugin keybind — opens the plugin's panel and
    /// delivers a `Keybind` event with the given `id`. Invoked from the
    /// `bind = …, spawn, mshellctl plugin keybind …` line mshell writes
    /// to `~/.config/margo/binds.d/mshell-plugins.conf`.
    Keybind {
        /// Composite plugin key.
        key: String,
        /// Binding id from the plugin's manifest `[[keybind]]`.
        id: String,
    },
}

pub async fn execute(command: PluginCommands) -> anyhow::Result<()> {
    match command {
        PluginCommands::Reload { key } => {
            bus_command_with_arg("PluginReload", &key).await?;
        }
        PluginCommands::Keybind { key, id } => {
            // zbus expects each method argument as a separate body item; the
            // simplest cross-IPC path with our existing `_with_arg` helper is
            // to pack the two into one delimited string and split on the
            // shell side. `|` doesn't appear in plugin keys or bind ids.
            let arg = format!("{key}|{id}");
            bus_command_with_arg("PluginKeybind", &arg).await?;
        }
    }
    Ok(())
}
