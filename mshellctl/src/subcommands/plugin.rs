use crate::bus::bus_command_with_arg;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum PluginCommands {
    /// List installed plugins with their version, enabled state, keybinds.
    /// `--names` prints just the composite keys, one per line — useful for
    /// shell completions.
    List {
        /// Print only the composite keys (machine-friendly).
        #[arg(long)]
        names: bool,
        /// Only show enabled plugins.
        #[arg(long)]
        enabled: bool,
    },
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
        PluginCommands::List { names, enabled } => list_plugins(names, enabled),
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

fn list_plugins(names_only: bool, enabled_only: bool) {
    let store = mshell_plugins::PluginStore::new();
    let state = store.load_state();
    let installed = store.installed();
    let resolved = mshell_plugins::keybinds::resolve_all(&store);

    // Build a `key -> Vec<binding-summary>` map so the human view can show
    // every binding (winning *or* losing) inline with the plugin row.
    let mut binds_by_plugin: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for r in &resolved {
        let combo = if r.disabled {
            "disabled".to_string()
        } else if r.keybind.combo.is_empty() {
            "—".to_string()
        } else {
            r.keybind.combo.clone()
        };
        let marker = if let Some(winner) = &r.conflict {
            format!(" (conflict with {winner})")
        } else {
            String::new()
        };
        binds_by_plugin
            .entry(r.plugin_key.clone())
            .or_default()
            .push(format!("{combo} → {}{marker}", r.keybind.id));
    }

    let rows: Vec<_> = installed
        .iter()
        .filter(|p| !enabled_only || state.is_enabled(&p.key))
        .collect();

    if names_only {
        for p in rows {
            println!("{}", p.key);
        }
        return;
    }
    if rows.is_empty() {
        println!("No plugins installed.");
        return;
    }

    // Header.
    println!("{:<22} {:<9} {:<9} KEYBINDS", "KEY", "VERSION", "STATUS");
    for p in rows {
        let status = if state.is_enabled(&p.key) {
            "enabled"
        } else {
            "disabled"
        };
        let binds = binds_by_plugin
            .get(&p.key)
            .map(|v| v.join("; "))
            .unwrap_or_else(|| "—".to_string());
        println!(
            "{:<22} {:<9} {:<9} {}",
            p.key, p.manifest.version, status, binds
        );
    }
}
