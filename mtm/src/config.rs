//! mtm config — `~/.config/margo/mtm.toml`.
//!
//! `mtm` is a standalone tool (like `mpower`/`mvpn`), not part of either
//! `margo-config` or `mshell-config` (docs/config-conventions.md §1) — its
//! own small TOML file, hand-edited, read once at startup.
//!
//! `#[serde(default)]` on the struct means a missing or partial file is
//! filled from [`Config::default`], so a bare `mtm.toml` with one overridden
//! key stays valid.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Every mtm knob. Defaults mirror `tm.sh`'s hardcoded constants and the
/// Catppuccin Mocha fzf theme it shipped with, so an unconfigured `mtm`
/// behaves exactly like the bash tool did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Session `mtm` attaches to (creating it if needed) when run with no
    /// arguments — `tm.sh`'s `DEFAULT_SESSION`.
    pub default_session: String,
    /// Wait for `anka` (a snapshot-restore tool) to repopulate the default
    /// session on session-server cold start before creating it ourselves.
    /// Off entirely if `anka`'s tmux options / snapshot file aren't found,
    /// regardless of this flag — see `kenp::anka_restore_pending`.
    pub anka_integration: bool,
    /// Starting directory for new layout windows/panes. Empty = `$HOME`.
    pub layout_cwd: String,
    /// fzf `--color` theme, passed through verbatim.
    pub fzf_theme: FzfTheme,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_session: "KENP".to_string(),
            anka_integration: true,
            layout_cwd: String::new(),
            fzf_theme: FzfTheme::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FzfTheme {
    pub bg_plus: String,
    pub bg: String,
    pub fg: String,
    pub fg_plus: String,
    pub hl: String,
    pub hl_plus: String,
    pub info: String,
    pub prompt: String,
    pub pointer: String,
    pub marker: String,
    pub spinner: String,
    pub header: String,
}

impl Default for FzfTheme {
    fn default() -> Self {
        // Catppuccin Mocha — tm.sh's `_TM_FZF_THEME`.
        Self {
            bg_plus: "#313244".into(),
            bg: "#1e1e2e".into(),
            fg: "#cdd6f4".into(),
            fg_plus: "#cdd6f4".into(),
            hl: "#f38ba8".into(),
            hl_plus: "#f38ba8".into(),
            info: "#cba6f7".into(),
            prompt: "#cba6f7".into(),
            pointer: "#f5e0dc".into(),
            marker: "#a6e3a1".into(),
            spinner: "#f5e0dc".into(),
            header: "#89b4fa".into(),
        }
    }
}

impl Config {
    /// Read `mtm.toml`, falling back to [`Config::default`] when the file
    /// is missing or unparseable (never panics — a bad edit must not brick
    /// the tool).
    pub fn load() -> Self {
        match std::fs::read_to_string(config_path()) {
            Ok(s) => toml::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}

/// Absolute path to `mtm.toml` under the margo config dir, honouring
/// `XDG_CONFIG_HOME` then `HOME`.
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".config")
        });
    base.join("margo").join("mtm.toml")
}

/// `~/.config/tmux` — same constant `tm.sh` used for `CONFIG_DIR`, home to
/// the plugin dir, the fzf speed-command dir, and what `mtm config backup`
/// archives.
pub fn tmux_config_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".config").join("tmux")
}

pub fn plugin_dir() -> PathBuf {
    tmux_config_dir().join("plugins")
}

pub fn fzf_dir() -> PathBuf {
    tmux_config_dir().join("fzf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_falls_back_to_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_override_keeps_other_defaults() {
        let cfg: Config = toml::from_str("default_session = \"WORK\"\n").unwrap();
        assert_eq!(cfg.default_session, "WORK");
        assert!(cfg.anka_integration);
    }

    #[test]
    fn round_trips() {
        let cfg = Config::default();
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }
}
