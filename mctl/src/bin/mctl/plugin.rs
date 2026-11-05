//! `mctl plugin`: list plugins and flip a manifest's `enabled` line.

use anyhow::{Result, bail};

use super::cli::PluginCmd;

/// `~/.config/margo/plugins` if it exists.
fn mctl_plugins_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(|h| std::path::PathBuf::from(h).join("margo/plugins"))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".config/margo/plugins"))
        })
        .filter(|p| p.is_dir())
}

/// Read a bare `key = value` field from a plugin.toml manifest (same flat
/// grammar margo's loader uses — no nested tables, and everything after a
/// `#` is a comment, inline ones included).
fn manifest_field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.split('#').next().unwrap_or("").trim();
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').trim_matches('\'').to_string())
    })
}

fn manifest_enabled(text: &str) -> bool {
    // Default true (matches the loader): absent / unparseable ⇒ enabled.
    manifest_field(text, "enabled")
        .map(|v| matches!(v.as_str(), "true" | "yes" | "1" | "on"))
        .unwrap_or(true)
}

pub(super) fn cmd_plugin(action: &PluginCmd) -> Result<()> {
    let Some(dir) = mctl_plugins_dir() else {
        // No `~/.config/margo/plugins/` — the common case. Not an error;
        // explain the three separate plugin/scripting worlds so the user
        // isn't left thinking something is broken.
        match action {
            PluginCmd::List => {
                println!("No compositor plugins.");
                println!();
                println!(
                    "  `mctl plugin` manages PACKAGED Rhai plugins in\n  \
                     ~/.config/margo/plugins/<name>/ (plugin.toml + init.rhai).\n\n  \
                     Personal scripting lives in ~/.config/margo/init.rhai and loads\n  \
                     automatically — no packaging needed.\n  \
                     Shell (WASM) plugins are a separate system: Settings → Plugins\n  \
                     (or `mshellctl`)."
                );
            }
            PluginCmd::Enable { .. } | PluginCmd::Disable { .. } => {
                println!(
                    "No ~/.config/margo/plugins/ directory yet — nothing to enable/disable.\n\
                     Create ~/.config/margo/plugins/<name>/ with a plugin.toml + init.rhai first."
                );
            }
        }
        return Ok(());
    };
    match action {
        PluginCmd::List => {
            let mut found = false;
            let mut entries: Vec<_> = std::fs::read_dir(&dir)?.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let manifest = entry.path().join("plugin.toml");
                let Ok(text) = std::fs::read_to_string(&manifest) else {
                    continue;
                };
                found = true;
                let dirname = entry.file_name().to_string_lossy().to_string();
                let name = manifest_field(&text, "name").unwrap_or_else(|| dirname.clone());
                let version = manifest_field(&text, "version").unwrap_or_default();
                let mark = if manifest_enabled(&text) {
                    "●"
                } else {
                    "○"
                };
                println!("{mark}  {name:<24} {version}");
            }
            if !found {
                println!("(no plugins in {})", dir.display());
            }
        }
        PluginCmd::Enable { name } => set_plugin_enabled(&dir, name, true)?,
        PluginCmd::Disable { name } => set_plugin_enabled(&dir, name, false)?,
    }
    Ok(())
}

/// Flip the `enabled` key of the plugin matching `name` (by directory name or
/// manifest `name`). Rewrites the `enabled = ...` line, or appends it.
fn set_plugin_enabled(dir: &std::path::Path, name: &str, enabled: bool) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let manifest = entry.path().join("plugin.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let dirname = entry.file_name().to_string_lossy().to_string();
        let mname = manifest_field(&text, "name").unwrap_or_else(|| dirname.clone());
        if dirname == name || mname == name {
            std::fs::write(&manifest, with_enabled_line(&text, enabled))?;
            println!(
                "{} plugin '{name}'. Takes effect on the next margo start.",
                if enabled { "Enabled" } else { "Disabled" }
            );
            return Ok(());
        }
    }
    bail!("no plugin named '{name}' in {}", dir.display());
}

/// `text` (a plugin.toml) with its `enabled = …` line set to `enabled`,
/// appended when there is none.
fn with_enabled_line(text: &str, enabled: bool) -> String {
    let val = if enabled { "true" } else { "false" };
    let mut out = String::new();
    let mut rewrote = false;
    for line in text.lines() {
        let is_enabled = line
            .split_once('=')
            .map(|(k, _)| k.trim() == "enabled")
            .unwrap_or(false);
        if is_enabled {
            out.push_str(&format!("enabled = {val}\n"));
            rewrote = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !rewrote {
        out.push_str(&format!("enabled = {val}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_fields() {
        let text = "# a plugin\nname = \"clock\"\nversion='1.2'\ndescription = a=b\n";
        assert_eq!(manifest_field(text, "name").as_deref(), Some("clock"));
        assert_eq!(manifest_field(text, "version").as_deref(), Some("1.2"));
        assert_eq!(manifest_field(text, "description").as_deref(), Some("a=b"));
        assert_eq!(manifest_field(text, "enabled"), None);
        assert_eq!(
            manifest_field("# name = x\n", "name"),
            None,
            "commented out"
        );
    }

    #[test]
    fn manifest_inline_comments_are_stripped_like_the_loader() {
        // margo's plugin loader drops everything after `#`; mctl read
        // `true # …` as a non-true value and listed the plugin disabled
        // (the loader's own doc example has such a comment).
        assert!(manifest_enabled(
            "name = clock\nenabled = true                # default true; flip to disable\n"
        ));
        assert!(!manifest_enabled("enabled = false # off for now\n"));
        assert_eq!(
            manifest_field("name = \"clock\"  # shown in lists\n", "name").as_deref(),
            Some("clock")
        );
        assert_eq!(
            manifest_field("   # name = x\n", "name"),
            None,
            "indented comment"
        );
    }

    #[test]
    fn manifest_enabled_defaults_to_true() {
        assert!(manifest_enabled("name = x\n"));
        assert!(manifest_enabled("enabled = yes\n"));
        assert!(manifest_enabled("enabled = on"));
        assert!(!manifest_enabled("enabled = false\n"));
        assert!(!manifest_enabled("enabled = 0\n"));
        assert!(
            !manifest_enabled("enabled = nah\n"),
            "unknown values disable"
        );
    }

    #[test]
    fn enabled_line_is_rewritten_or_appended() {
        assert_eq!(
            with_enabled_line("name = x\nenabled = true\nversion = 1\n", false),
            "name = x\nenabled = false\nversion = 1\n"
        );
        assert_eq!(
            with_enabled_line("name = x\n", true),
            "name = x\nenabled = true\n"
        );
        assert_eq!(
            with_enabled_line("name = x\n  enabled=false", true),
            "name = x\nenabled = true\n"
        );
        assert_eq!(
            with_enabled_line("# enabled = false\nname = x", false),
            "# enabled = false\nname = x\nenabled = false\n",
            "a commented-out key is left alone"
        );
    }
}
