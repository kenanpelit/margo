//! Launcher schema: `launcher:` (panel options, `>start` autostart
//! scripts, favourite websites, provider options) and `pass:` (the GNU
//! pass provider). Split out of `config.rs` to keep it under its size cap.

use reactive_stores::{KeyMap, Patch, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Launcher-wide settings (currently the `>start` script autostart
/// list). Each entry is keyed by the script's short name.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Launcher {
    /// Scripts the user opted into running at shell startup, with a
    /// per-script delay. Names match `ScriptsProvider` short names
    /// (e.g. `start-brave-ai`).
    pub autostart_scripts: Vec<ScriptAutostart>,
    /// Show the detail/preview pane beside the result list. When off,
    /// the result list always fills the launcher's full width.
    pub show_preview: bool,
    /// Compact rows (tighter padding) instead of the default
    /// comfortable density — fits more results per screen.
    pub compact_rows: bool,
    /// Lead app / window rows with a larger icon.
    pub large_app_icons: bool,
    /// Favourite websites for the launcher's `web` prefix, one
    /// `Name|https://url` entry each (a bare URL or domain works too).
    /// Listed most-recently-opened first.
    pub web_sites: Vec<String>,
    /// Fetch favicons for `web_sites` and for web apps made with `webapp`
    /// (the domain is sent to Google's favicon service; icons are cached
    /// in ~/.cache/margo/favicons and ~/.local/share/margo/webapps).
    pub web_site_favicons: bool,
}

/// Default `launcher.web_sites` — mirrors
/// `mshell_launcher::providers::websearch::DEFAULT_SITES` (this crate
/// can't depend on the launcher).
pub fn default_web_sites() -> Vec<String> {
    [
        "GitHub|https://github.com",
        "YouTube|https://www.youtube.com",
        "Gmail|https://mail.google.com",
        "Arch Wiki|https://wiki.archlinux.org",
        "AUR|https://aur.archlinux.org",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            autostart_scripts: Vec::new(),
            show_preview: false,
            compact_rows: false,
            large_app_icons: true,
            web_sites: default_web_sites(),
            web_site_favicons: true,
        }
    }
}

/// GNU pass (password-store) launcher provider. Surfaced under
/// Settings → Launcher → Storage paths.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct Pass {
    /// Password-store directory. Empty = follow `$PASSWORD_STORE_DIR`,
    /// else `~/.password-store` (pass's own resolution order).
    pub store_path: String,
}

/// One `>start` script's autostart configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct ScriptAutostart {
    /// Script short name (e.g. `start-brave-ai`).
    pub name: String,
    /// Run this script at shell startup.
    pub enabled: bool,
    /// Seconds to wait after startup before running it.
    pub delay_secs: u32,
    /// When the script fires within a login session. Defaults to
    /// `EveryStart` so configs written before this field existed keep
    /// their original "runs on every restart" behaviour.
    pub trigger: AutostartTrigger,
    /// Extra arguments appended to the command (whitespace-separated).
    pub args: String,
    /// Working directory to run the script in (empty = inherit the
    /// session's). `~` is expanded at launch.
    pub working_dir: String,
}

/// How often an autostart script fires across a login session.
///
/// A *login session* is the graphical seat session — it survives
/// `systemctl --user restart mshell` but ends at logout. We tell the
/// two apart with a marker file under `$XDG_RUNTIME_DIR`, which systemd
/// tears down at logout, so `LoginOnce` runs exactly once per login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, Store, JsonSchema)]
pub enum AutostartTrigger {
    /// Run on every shell start, including in-session mshell restarts.
    /// The pre-existing behaviour, and the serde default for old configs.
    #[default]
    EveryStart,
    /// Run only on the first shell start of a login session; skipped on
    /// `systemctl --user restart mshell` until the next login.
    LoginOnce,
}

impl PatchField for AutostartTrigger {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

impl AutostartTrigger {
    pub fn to_index(self) -> u32 {
        match self {
            AutostartTrigger::EveryStart => 0,
            AutostartTrigger::LoginOnce => 1,
        }
    }

    pub fn from_index(index: u32) -> Self {
        match index {
            1 => AutostartTrigger::LoginOnce,
            _ => AutostartTrigger::EveryStart,
        }
    }
}
