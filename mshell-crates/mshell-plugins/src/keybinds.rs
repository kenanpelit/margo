//! Plugin-keybind registry + binds file writer.
//!
//! Each plugin's manifest can ship one or more `[[keybind]]` entries. We
//! resolve conflicts deterministically (alphabetical plugin id wins) so the
//! same plugin set always produces the same active bindings — load order
//! never matters. Losers are logged and surfaced via [`Resolution::conflict`].
//!
//! The shell writes the survivors to `~/.config/margo/binds.d/mshell-plugins.conf`
//! as `bind = <combo>, spawn, mshellctl plugin keybind <plugin-key> <id>` lines.
//! The user adds `source=binds.d/mshell-plugins.conf` to their `config.conf`
//! once; mshell calls `mctl reload` whenever the file changes.

use crate::{Keybind, PluginStore};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One plugin's bind plus how it fared against conflicts.
#[derive(Debug, Clone)]
pub struct Resolution {
    /// Composite plugin key (`"mullvad"`, `"a1b2:my-plugin"`, …).
    pub plugin_key: String,
    /// The effective binding (after any user override has been applied).
    pub keybind: Keybind,
    /// The manifest's original combo, kept for "reset to default" in the UI.
    pub default_combo: String,
    /// `Some(winner_key)` if this binding lost to another plugin claiming
    /// the same combo first; `None` when this is the active binding.
    pub conflict: Option<String>,
    /// `true` if the user explicitly cleared this binding (combo empty).
    pub disabled: bool,
}

impl Resolution {
    pub fn is_active(&self) -> bool {
        self.conflict.is_none() && !self.disabled
    }
}

/// Resolve every installed-and-enabled plugin's keybinds. Stable order:
/// plugins by composite key alphabetically, then bindings in manifest order.
pub fn resolve_all(store: &PluginStore) -> Vec<Resolution> {
    let state = store.load_state();
    // Alphabetical plugin order makes the resolution deterministic regardless
    // of install order on disk.
    let mut plugins = store.installed();
    plugins.sort_by(|a, b| a.key.cmp(&b.key));

    let mut owner: BTreeMap<String, String> = BTreeMap::new();
    let mut out: Vec<Resolution> = Vec::new();
    for plugin in plugins {
        if !state.is_enabled(&plugin.key) {
            continue;
        }
        for keybind in &plugin.manifest.keybinds {
            let default_combo = normalize_combo(&keybind.combo);
            if keybind.id.trim().is_empty() {
                continue;
            }
            // The user can override the combo (or disable it with "").
            let (combo, disabled) = match state.keybind_override(&plugin.key, &keybind.id) {
                Some(s) if s.trim().is_empty() => (String::new(), true),
                Some(s) => (normalize_combo(s), false),
                None => (default_combo.clone(), false),
            };
            // A binding with no combo (no manifest default + no override) is
            // skipped entirely; a *disabled* binding still surfaces in the UI
            // so the user can re-enable it.
            if combo.is_empty() && !disabled && default_combo.is_empty() {
                continue;
            }
            let conflict = if disabled || combo.is_empty() {
                None
            } else if let Some(winner) = owner.get(&combo) {
                Some(winner.clone())
            } else {
                owner.insert(combo.clone(), plugin.key.clone());
                None
            };
            out.push(Resolution {
                plugin_key: plugin.key.clone(),
                keybind: Keybind {
                    combo,
                    id: keybind.id.clone(),
                    description: keybind.description.clone(),
                },
                default_combo,
                conflict,
                disabled,
            });
        }
    }
    out
}

/// Lower-case the modifier part of a margo combo (`Super+a` → `super+a`) so
/// `Super+A` and `super+a` collide. Margo itself is case-insensitive on
/// modifiers; this just normalises the registry's view.
fn normalize_combo(combo: &str) -> String {
    let trimmed = combo.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Split on the comma that margo uses between modifier-chord and key —
    // keep the keysym case as is (margo cares about case for some keysyms).
    if let Some((mods, key)) = trimmed.rsplit_once(',') {
        format!("{},{}", mods.to_ascii_lowercase(), key)
    } else {
        // Single segment (just modifiers, no key) — uncommon but normalise.
        trimmed.to_ascii_lowercase()
    }
}

/// Path of the file mshell writes its plugin binds to. The user opts in by
/// adding `source=binds.d/mshell-plugins.conf` to their `config.conf` once.
pub fn binds_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("margo")
        .join("binds.d")
        .join("mshell-plugins.conf")
}

/// Write the active resolutions to [`binds_path`]. Returns `Ok(true)` if the
/// file's contents actually changed (so the caller knows to ask margo for a
/// reload). Creates the parent directory if missing.
pub fn write_binds_file(resolved: &[Resolution]) -> std::io::Result<bool> {
    let path = binds_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut body = String::new();
    body.push_str(
        "# Generated by mshell. Do not edit — re-rendered on every plugin\n\
         # enable / disable / install / uninstall.\n\
         #\n\
         # To activate, add this line to ~/.config/margo/config.conf:\n\
         #     source=binds.d/mshell-plugins.conf\n\
         # then run: mctl reload\n\n",
    );
    for r in resolved.iter().filter(|r| r.is_active()) {
        // Defence-in-depth against binds-file injection: manifest::validate
        // already rejects newlines/extra commas in combo+id at load, but skip
        // any resolution that still carries a newline in a field so a bypass
        // can never inject an arbitrary `bind = …, spawn, <cmd>` line.
        if [&r.keybind.combo, &r.plugin_key, &r.keybind.id]
            .iter()
            .any(|f| f.contains(['\n', '\r']))
        {
            continue;
        }
        body.push_str(&format!(
            "bind = {}, spawn, mshellctl plugin keybind {} {}\n",
            r.keybind.combo, r.plugin_key, r.keybind.id
        ));
    }
    // Trailing notes on conflicts so a curious user grepping the file sees
    // why a binding they expected isn't there.
    let conflicts: Vec<&Resolution> = resolved.iter().filter(|r| !r.is_active()).collect();
    if !conflicts.is_empty() {
        body.push_str("\n# Conflicts (combo already claimed by an earlier plugin):\n");
        for r in conflicts {
            let winner = r.conflict.as_deref().unwrap_or("?");
            body.push_str(&format!(
                "#   {} wanted {} -> {} but {} got it first\n",
                r.plugin_key, r.keybind.combo, r.keybind.id, winner
            ));
        }
    }

    let previous = std::fs::read_to_string(&path).unwrap_or_default();
    if previous == body {
        return Ok(false);
    }
    let mut f = std::fs::File::create(&path)?;
    f.write_all(body.as_bytes())?;
    Ok(true)
}

/// Convenience for callers (the settings UI, the shell startup) that just
/// want "write the binds file + ask margo to reload if anything changed".
/// Returns whether anything was actually written. `mctl reload` is only
/// attempted when the user already sources us (so we don't ping margo for
/// nothing).
pub fn sync_with_margo(store: &PluginStore) -> std::io::Result<bool> {
    let resolved = resolve_all(store);
    let changed = write_binds_file(&resolved)?;
    if changed {
        let config_conf = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("margo")
            .join("config.conf");
        if user_sources_us(&config_conf) {
            let _ = std::process::Command::new("mctl")
                .arg("reload")
                .arg("--force")
                .spawn();
        }
    }
    Ok(changed)
}

/// `true` if the user's `config.conf` already pulls in our binds file.
/// Lets the shell log a one-shot hint when it doesn't.
pub fn user_sources_us(config_conf: &Path) -> bool {
    std::fs::read_to_string(config_conf)
        .map(|text| text_sources_us(&text))
        .unwrap_or(false)
}

/// Whether the given config text `source`s our binds file. Whitespace-tolerant:
/// margo accepts `source=…` and `source = …` alike, so both match (users
/// naturally write the spaced form to match the rest of their config).
/// Commented-out lines don't count.
fn text_sources_us(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .any(|l| {
            l.strip_prefix("source")
                .map(str::trim_start)
                .and_then(|r| r.strip_prefix('='))
                .map(|v| v.contains("binds.d/mshell-plugins.conf"))
                .unwrap_or(false)
        })
}

#[cfg(test)]
mod tests {
    use super::text_sources_us;

    #[test]
    fn detects_sourced_in_any_whitespace_style() {
        assert!(text_sources_us("source=binds.d/mshell-plugins.conf"));
        assert!(text_sources_us("source = binds.d/mshell-plugins.conf"));
        assert!(text_sources_us(
            "  source   =   binds.d/mshell-plugins.conf  "
        ));
        // Among other lines.
        assert!(text_sources_us(
            "source = colors.conf\nsource = binds.d/mshell-plugins.conf\n"
        ));
    }

    #[test]
    fn ignores_absent_or_commented() {
        assert!(!text_sources_us(
            "source = colors.conf\nsource = mlayout.conf\n"
        ));
        assert!(!text_sources_us("# source = binds.d/mshell-plugins.conf"));
        assert!(!text_sources_us(""));
    }
}
