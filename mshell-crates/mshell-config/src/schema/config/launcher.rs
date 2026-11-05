//! Launcher schema: `launcher:` (panel options, `>start` autostart
//! scripts, favourite websites, provider options) and `pass:` (the GNU
//! pass provider). Split out of `config.rs` to keep it under its size cap.

use super::default_true;
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
    /// Folder whose sub-directories the launcher's `nv` provider lists as
    /// Neovim projects next to the saved sessions. Empty = sessions only.
    pub nvim_projects_dir: String,
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
            nvim_projects_dir: String::new(),
        }
    }
}

/// GNU pass (password-store) launcher provider. Surfaced under
/// Settings → Launcher → Password store (pass).
///
/// Hand-written `Default` (not derived) for the same reason as
/// [`super::Valent`]: a profile without a `pass:` block takes the whole-struct
/// default, so the non-zero defaults below must live there too.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Pass {
    /// Password-store directory. Empty = follow `$PASSWORD_STORE_DIR`,
    /// else `~/.password-store` (pass's own resolution order).
    pub store_path: String,
    /// Seconds a copied password / field stays on the clipboard before it
    /// is cleared (if nothing else was copied since). pass's own default.
    #[serde(default = "default_pass_clip_timeout_secs")]
    pub clip_timeout_secs: u32,
    /// Milliseconds to wait after the launcher closes before typing, so
    /// focus is back in the field you were in.
    #[serde(default = "default_pass_type_delay_ms")]
    pub type_delay_ms: u32,
    /// Milliseconds between typed keystrokes (`wtype -d`). Raise it when
    /// a slow app drops characters. 0 = as fast as possible.
    pub key_delay_ms: u32,
    /// Key autotype presses between the username and the password.
    pub autotype_separator: PassAutotypeSeparator,
    /// Autotype presses Enter after the last value (the password, or the
    /// OTP code when the entry has one) to submit the form.
    #[serde(default = "default_true")]
    pub autotype_submit: bool,
}

impl Default for Pass {
    fn default() -> Self {
        Self {
            store_path: String::new(),
            clip_timeout_secs: default_pass_clip_timeout_secs(),
            type_delay_ms: default_pass_type_delay_ms(),
            key_delay_ms: 0,
            autotype_separator: PassAutotypeSeparator::default(),
            autotype_submit: true,
        }
    }
}

fn default_pass_clip_timeout_secs() -> u32 {
    45
}

fn default_pass_type_delay_ms() -> u32 {
    250
}

/// The key the pass provider's autotype presses between username and
/// password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, Store, JsonSchema)]
pub enum PassAutotypeSeparator {
    /// Tab — the usual next-field key.
    #[default]
    Tab,
    /// Enter — for two-step logins that show the password field only
    /// after the username is submitted.
    Enter,
}

impl PatchField for PassAutotypeSeparator {
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
