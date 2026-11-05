//! The host-config sections beside packages and modules: Flatpak scope,
//! hooks, backups, services, default apps and theming.

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FlatpakScope {
    User,
    System,
}

/// The Flatpak remote used when neither the package nor the host names one.
pub const DEFAULT_FLATPAK_REMOTE: &str = "flathub";

/// The remote a Flatpak is installed from: the package's own `remote:`, else the host's `flatpak_remote`, else flathub. A blank
/// value counts as unset.
pub fn resolve_flatpak_remote<'a>(
    package_remote: Option<&'a str>,
    host_remote: Option<&'a str>,
) -> &'a str {
    let set = |remote: Option<&'a str>| remote.map(str::trim).filter(|remote| !remote.is_empty());
    set(package_remote)
        .or(set(host_remote))
        .unwrap_or(DEFAULT_FLATPAK_REMOTE)
}

impl FlatpakScope {
    pub fn as_arg(&self) -> &'static str {
        match self {
            FlatpakScope::User => "--user",
            FlatpakScope::System => "--system",
        }
    }
}

/// Hook execution user configuration
/// Can be specified as a boolean (backward compat) or username string
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum RunHooksAsUser {
    /// Legacy boolean format: true = current user, false = root (with sudo)
    Bool(bool),
    /// New format: specific username to run as
    Username(String),
}

impl Default for RunHooksAsUser {
    fn default() -> Self {
        RunHooksAsUser::Bool(false)
    }
}

impl RunHooksAsUser {
    /// Get the username to run as, if any
    /// Returns None for root/sudo execution (false or empty string)
    /// Returns Some(username) for user execution
    pub fn username(&self) -> Option<String> {
        match self {
            RunHooksAsUser::Bool(false) => None,
            RunHooksAsUser::Bool(true) => {
                // Get current username
                std::env::var("USER").ok()
            }
            RunHooksAsUser::Username(s) if s.is_empty() => None,
            RunHooksAsUser::Username(s) => Some(s.clone()),
        }
    }
}

impl<'de> Deserialize<'de> for RunHooksAsUser {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let value = yaml_serde::Value::deserialize(deserializer)?;

        // Try boolean first
        if let Some(b) = value.as_bool() {
            return Ok(RunHooksAsUser::Bool(b));
        }

        // Try string
        if let Some(s) = value.as_str() {
            return Ok(RunHooksAsUser::Username(s.to_string()));
        }

        Err(D::Error::custom(
            "run_hooks_as_user must be a boolean or a username string",
        ))
    }
}

/// Configuration backup settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigBackupsSettings {
    /// Enable automatic backups on sync
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Maximum number of backups to keep (0 = unlimited)
    #[serde(default = "default_max_backups")]
    pub max_backups: u32,
}

impl Default for ConfigBackupsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_backups: 5,
        }
    }
}

/// System backup settings (timeshift/snapper)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBackupsSettings {
    /// Enable system backups globally
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Create backup during mdots sync
    #[serde(default = "default_true")]
    pub backup_on_sync: bool,

    /// Create backup during mdots update
    #[serde(default = "default_true")]
    pub backup_on_update: bool,

    /// Backup tool to use: "timeshift" or "snapper"
    #[serde(default)]
    pub tool: Option<String>,

    /// Snapper configuration name (default: "root")
    #[serde(default = "default_snapper_config")]
    pub snapper_config: String,

    /// Maximum number of backups to keep (0 = unlimited)
    #[serde(default = "default_max_backups")]
    pub max_backups: u32,
}

impl Default for SystemBackupsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            backup_on_sync: true,
            backup_on_update: true,
            tool: None,
            snapper_config: "root".to_string(),
            max_backups: 5,
        }
    }
}

pub(super) fn default_true() -> bool {
    true
}

pub(super) fn default_max_backups() -> u32 {
    5
}

/// Update hooks configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateHooksConfig {
    /// Pre-update hook script (runs before system update)
    #[serde(default)]
    pub pre_update: Option<String>,

    /// Post-update hook script (runs after system update)
    #[serde(default)]
    pub post_update: Option<String>,

    /// Hook behavior: "ask" (default), "always", "once", "skip"
    #[serde(default = "default_hook_behavior")]
    pub behavior: String,

    /// Enable --devel flag for AUR helper to update VCS packages (e.g., -git packages)
    #[serde(default)]
    pub devel: bool,

    /// Run hooks as current user instead of with sudo (default: false)
    #[serde(default)]
    pub run_as_user: bool,
}

impl Default for UpdateHooksConfig {
    fn default() -> Self {
        Self {
            pre_update: None,
            post_update: None,
            behavior: "ask".to_string(),
            devel: false,
            run_as_user: false,
        }
    }
}

/// Scope for systemd services (system or user)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ServiceScope {
    #[default]
    System,
    User,
}

impl ServiceScope {
    pub fn as_flag(&self) -> Option<&str> {
        match self {
            ServiceScope::System => None, // No flag needed for system scope
            ServiceScope::User => Some("--user"),
        }
    }
}

/// System services configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicesConfig {
    /// Services to enable (and start)
    #[serde(default)]
    pub enabled: Vec<String>,

    /// Services to disable (and stop)
    #[serde(default)]
    pub disabled: Vec<String>,

    /// Service scope: "system" (default) or "user"
    #[serde(default)]
    pub scope: ServiceScope,
}

impl Default for ServicesConfig {
    fn default() -> Self {
        Self {
            enabled: Vec::new(),
            disabled: Vec::new(),
            scope: ServiceScope::System,
        }
    }
}

/// Service profile structure (Lua-based service configuration)
/// Loaded from services/ directory
#[derive(Debug, Clone)]
pub struct ServiceProfile {
    /// Profile name (filename without extension)
    pub name: String,

    /// Path to the profile file
    pub path: std::path::PathBuf,

    /// Description of what this profile does
    pub description: String,

    /// Services to enable/disable
    pub services: ServicesConfig,

    /// Conflicting profiles (cannot be enabled together)
    pub conflicts: Vec<String>,
}

/// Scope for setting default applications
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DefaultsScope {
    User,
    System,
}

pub(super) fn default_defaults_scope() -> DefaultsScope {
    DefaultsScope::System
}

/// Module processing mode
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModuleProcessing {
    Parallel,
    Sequential,
}

pub(super) fn default_module_processing() -> ModuleProcessing {
    ModuleProcessing::Parallel
}

pub(super) fn default_strict_package_order() -> bool {
    false
}

/// Default applications configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultAppsConfig {
    /// Scope for setting defaults: "user" or "system" (default: system)
    #[serde(default = "default_defaults_scope")]
    pub scope: DefaultsScope,

    /// High-level app categories
    #[serde(default)]
    pub browser: Option<String>,

    #[serde(default)]
    pub text_editor: Option<String>,

    #[serde(default)]
    pub file_manager: Option<String>,

    #[serde(default)]
    pub terminal: Option<String>,

    #[serde(default)]
    pub video_player: Option<String>,

    #[serde(default)]
    pub audio_player: Option<String>,

    #[serde(default)]
    pub image_viewer: Option<String>,

    #[serde(default)]
    pub pdf_viewer: Option<String>,

    /// Custom MIME type mappings for fine-grained control
    #[serde(default)]
    pub mime_types: std::collections::HashMap<String, String>,
}

impl Default for DefaultAppsConfig {
    fn default() -> Self {
        Self {
            scope: DefaultsScope::System,
            browser: None,
            text_editor: None,
            file_manager: None,
            terminal: None,
            video_player: None,
            audio_player: None,
            image_viewer: None,
            pdf_viewer: None,
            mime_types: std::collections::HashMap::new(),
        }
    }
}

impl DefaultAppsConfig {
    /// Convert config to HashMap for processing (only non-None values)
    pub fn to_apps_map(&self) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();

        if let Some(ref app) = self.browser {
            map.insert("browser".to_string(), app.clone());
        }
        if let Some(ref app) = self.text_editor {
            map.insert("text_editor".to_string(), app.clone());
        }
        if let Some(ref app) = self.file_manager {
            map.insert("file_manager".to_string(), app.clone());
        }
        if let Some(ref app) = self.terminal {
            map.insert("terminal".to_string(), app.clone());
        }
        if let Some(ref app) = self.video_player {
            map.insert("video_player".to_string(), app.clone());
        }
        if let Some(ref app) = self.audio_player {
            map.insert("audio_player".to_string(), app.clone());
        }
        if let Some(ref app) = self.image_viewer {
            map.insert("image_viewer".to_string(), app.clone());
        }
        if let Some(ref app) = self.pdf_viewer {
            map.insert("pdf_viewer".to_string(), app.clone());
        }

        map
    }
}

/// Scope for theming configuration (user or system-wide)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemingScope {
    #[default]
    User,
    System,
}

/// Desktop theming configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemingConfig {
    /// Scope for theming: "user" (default) or "system"
    #[serde(default)]
    pub scope: ThemingScope,

    /// Cursor theme configuration
    #[serde(default)]
    pub cursor: Option<CursorConfig>,

    /// Icon theme name (e.g., "tela-purple-dark")
    #[serde(default)]
    pub icons: Option<String>,

    /// Main theme name (e.g., "catppuccin-mocha")
    #[serde(default)]
    pub theme: Option<String>,

    /// Dark mode preference: "dark", "light", or omit for auto
    #[serde(default)]
    pub dark_or_light: Option<String>,

    /// Global font configuration
    #[serde(default)]
    pub font: Option<FontConfig>,

    /// GTK-specific settings
    #[serde(default)]
    pub gtk: Option<GtkThemingConfig>,

    /// Qt-specific settings
    #[serde(default)]
    pub qt: Option<QtThemingConfig>,

    /// Additional environment variables to set
    #[serde(default)]
    pub env_vars: std::collections::HashMap<String, String>,
}

impl Default for ThemingConfig {
    fn default() -> Self {
        Self {
            scope: ThemingScope::User,
            cursor: None,
            icons: None,
            theme: None,
            dark_or_light: None,
            font: None,
            gtk: None,
            qt: None,
            env_vars: std::collections::HashMap::new(),
        }
    }
}

/// Cursor theme configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorConfig {
    /// Cursor theme name (e.g., "bibata-modern-ice")
    pub theme: String,

    /// Cursor size in pixels
    #[serde(default)]
    pub size: Option<u32>,
}

/// Font configuration with separate family and size
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontConfig {
    /// Font family name (e.g., "JetBrainsMono Nerd Font")
    #[serde(default)]
    pub family: Option<String>,

    /// Font size in points
    #[serde(default)]
    pub size: Option<f32>,
}

/// GTK-specific theming configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GtkThemingConfig {
    /// Enable client-side window decorations
    #[serde(default)]
    pub decorations: Option<bool>,

    /// Primary mouse button: "left" or "right"
    #[serde(default)]
    pub primary_button: Option<String>,

    /// Enable animations
    #[serde(default)]
    pub enable_animations: Option<bool>,
}

/// Qt backend selection
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum QtBackend {
    #[default]
    Auto,
    Qt5ct,
    Kde,
}

/// Qt-specific theming configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QtThemingConfig {
    /// Qt backend to use: "auto" (default), "kde", or "qt5ct"
    #[serde(default)]
    pub backend: QtBackend,

    /// Qt style name (e.g., "kvantum", "fusion", "breeze")
    #[serde(default)]
    pub style: Option<String>,

    /// Qt-specific icon theme override
    #[serde(default)]
    pub icon_theme: Option<String>,

    /// Qt-specific font configuration
    #[serde(default)]
    pub font: Option<FontConfig>,
}

impl Default for QtThemingConfig {
    fn default() -> Self {
        Self {
            backend: QtBackend::Auto,
            style: None,
            icon_theme: None,
            font: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatpak_remote_package_beats_host_beats_flathub() {
        assert_eq!(resolve_flatpak_remote(None, None), "flathub");
        assert_eq!(resolve_flatpak_remote(None, Some("fedora")), "fedora");
        assert_eq!(
            resolve_flatpak_remote(Some("gnome-nightly"), Some("fedora")),
            "gnome-nightly"
        );
        // A blank value is unset, not an empty remote name.
        assert_eq!(resolve_flatpak_remote(Some("  "), Some("")), "flathub");
    }
}
