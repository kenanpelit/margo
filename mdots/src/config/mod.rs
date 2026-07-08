use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Main configuration structure for mdots
/// Can be used as both config.yaml (pointer) and host file (full config)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Hostname of the current machine
    pub host: String,

    /// Optional path to SOPS/Age key file.
    /// When set, exported as `SOPS_AGE_KEY_FILE` for `sops --decrypt`.
    #[serde(default)]
    pub sops_key_path: Option<String>,

    /// SOPS-encrypted secrets to decrypt into place during sync.
    #[serde(default)]
    pub secrets: Vec<SecretEntry>,

    /// Optional description of this host
    #[serde(default)]
    pub description: String,

    /// Import additional config files (relative to arch-config root)
    /// Example: ["hosts/shared/laptop-common.yaml"]
    #[serde(default)]
    pub import: Vec<String>,

    /// List of enabled module names (can include paths like "window-managers/hyprland")
    #[serde(default)]
    pub enabled_modules: Vec<String>,

    /// Host-specific packages to install
    #[serde(default)]
    pub packages: Vec<PackageEntry>,

    /// Packages to exclude from base or modules
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Additional packages to install (backwards compatibility)
    #[serde(default)]
    pub additional_packages: Vec<PackageEntry>,

    /// Backup tool to use: "timeshift" or "snapper"
    /// DEPRECATED: Use system_backups.tool instead
    #[serde(default, skip_serializing)]
    #[deprecated(note = "Use system_backups.tool instead")]
    pub backup_tool: Option<String>,

    /// Snapper configuration name (default: "root")
    /// DEPRECATED: Use system_backups.snapper_config instead
    #[serde(default = "default_snapper_config", skip_serializing)]
    #[deprecated(note = "Use system_backups.snapper_config instead")]
    pub snapper_config: String,

    /// Flatpak installation scope: "user" or "system"
    #[serde(default = "default_flatpak_scope")]
    pub flatpak_scope: FlatpakScope,

    /// Automatically prune packages during sync (default: false)
    #[serde(default)]
    pub auto_prune: bool,

    /// Module processing mode: "parallel" (default) or "sequential"
    #[serde(default = "default_module_processing")]
    pub module_processing: ModuleProcessing,

    /// Install packages one-at-a-time in strict order (default: false)
    /// Only applies when module_processing is "sequential"
    #[serde(default = "default_strict_package_order")]
    pub strict_package_order: bool,

    /// Configuration backup settings
    #[serde(default)]
    pub config_backups: ConfigBackupsSettings,

    /// System backup settings
    #[serde(default)]
    pub system_backups: SystemBackupsSettings,

    /// System services configuration
    #[serde(default)]
    pub services: ServicesConfig,

    /// List of enabled service profile names (from services/ directory)
    #[serde(default)]
    pub enabled_service_profiles: Vec<String>,

    /// Update hooks configuration
    #[serde(default)]
    pub update_hooks: UpdateHooksConfig,

    /// Default applications configuration
    #[serde(default)]
    pub default_apps: DefaultAppsConfig,

    /// Desktop theming configuration (GTK, Qt, cursor, icons, fonts)
    #[serde(default)]
    pub theming: ThemingConfig,

    /// Editor to use for config file editing (falls back to $EDITOR env var)
    #[serde(default)]
    pub editor: Option<String>,

    /// Package manager type: pacman (Arch and Arch-based distros)
    /// Auto-detected during `mdots init`, can be overridden
    #[serde(default)]
    pub package_manager: Option<PackageManagerType>,

    /// AUR helper to use for package management (paru, yay, etc.)
    /// Only applicable when package_manager is "pacman"
    #[serde(default)]
    pub aur_helper: Option<String>,

    /// Run sync operations with sudo (default: false)
    #[serde(default)]
    pub sync_sudo: bool,

    /// Automatically commit changes to git after successful sync (default: false)
    #[serde(default)]
    pub auto_commit: bool,

    /// Nix package manager and home-manager integration
    #[serde(default)]
    pub nix: NixConfig,
}

fn default_snapper_config() -> String {
    "root".to_string()
}

fn default_flatpak_scope() -> FlatpakScope {
    FlatpakScope::User
}

fn default_nixpkgs_channel() -> String {
    "nixpkgs-unstable".to_string()
}

fn default_hm_channel() -> String {
    "release-25.05".to_string()
}

/// Nix package manager and home-manager configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NixConfig {
    /// Nix is installed on this system
    #[serde(default)]
    pub enabled: bool,

    /// Run home-manager switch during mdots sync
    #[serde(default)]
    pub home_manager_enabled: bool,

    /// Use flakes instead of channels
    #[serde(default)]
    pub flake_enabled: bool,

    /// Nixpkgs channel URL/name (default: nixpkgs-unstable)
    #[serde(default = "default_nixpkgs_channel")]
    pub nixpkgs_channel: String,

    /// Home-manager channel URL/name (default: release-25.05)
    #[serde(default = "default_hm_channel")]
    pub home_manager_channel: String,
}

impl Default for NixConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            home_manager_enabled: false,
            flake_enabled: false,
            nixpkgs_channel: default_nixpkgs_channel(),
            home_manager_channel: default_hm_channel(),
        }
    }
}

/// Append every item from `incoming` into `target`, skipping values already
/// present. Order-preserving: existing entries keep their position and new
/// values are added in their original order. Used to merge string lists when an
/// imported config is folded into the main one.
fn extend_dedup(target: &mut Vec<String>, incoming: Vec<String>) {
    for item in incoming {
        if !target.contains(&item) {
            target.push(item);
        }
    }
}

impl Config {
    /// Merge another config into this one
    /// Main file values take precedence over imported values for scalar fields
    /// Lists are merged (deduplicated for enabled_modules)
    pub fn merge(&mut self, other: Config) {
        // Collections: order-preserving dedup (main's entries keep their order,
        // new ones from the import are appended).
        extend_dedup(&mut self.enabled_modules, other.enabled_modules);
        extend_dedup(&mut self.exclude, other.exclude);
        extend_dedup(&mut self.services.enabled, other.services.enabled);
        extend_dedup(&mut self.services.disabled, other.services.disabled);
        extend_dedup(
            &mut self.enabled_service_profiles,
            other.enabled_service_profiles,
        );

        // Packages: keep duplicates — the package manager collapses them.
        self.packages.extend(other.packages);
        self.additional_packages.extend(other.additional_packages);

        // For scalar values: main file wins (only use import if main is None/default)
        #[allow(deprecated)]
        if self.backup_tool.is_none() && other.backup_tool.is_some() {
            self.backup_tool = other.backup_tool;
        }

        // Merge system_backups settings
        // If main file has old-style settings, migrate them to new structure
        #[allow(deprecated)]
        if self.system_backups.tool.is_none() && self.backup_tool.is_some() {
            self.system_backups.tool = self.backup_tool.clone();
        }

        // For other system_backups fields, main file wins unless it's default
        if other.system_backups.tool.is_some() && self.system_backups.tool.is_none() {
            self.system_backups.tool = other.system_backups.tool;
        }

        // If description is empty, use imported description
        if self.description.is_empty() && !other.description.is_empty() {
            self.description = other.description;
        }

        // If package_manager not set, use imported value
        if self.package_manager.is_none() && other.package_manager.is_some() {
            self.package_manager = other.package_manager;
        }

        // Note: host, snapper_config, flatpak_scope, auto_prune from main file always win
        // This prevents imports from accidentally changing core settings
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FlatpakScope {
    User,
    System,
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

        let value = serde_yaml::Value::deserialize(deserializer)?;

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

fn default_true() -> bool {
    true
}

fn default_max_backups() -> u32 {
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

fn default_defaults_scope() -> DefaultsScope {
    DefaultsScope::System
}

/// Module processing mode
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModuleProcessing {
    Parallel,
    Sequential,
}

fn default_module_processing() -> ModuleProcessing {
    ModuleProcessing::Parallel
}

fn default_strict_package_order() -> bool {
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

/// Package entry that can be either a simple string or an object with type specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PackageEntry {
    /// Simple package name (e.g., "vim")
    Simple(String),

    /// Package with explicit type
    WithType {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        r#type: Option<PackageType>,
    },
}

impl PackageEntry {
    pub fn name(&self) -> &str {
        match self {
            PackageEntry::Simple(name) => {
                // Handle flatpak: and nix: prefix formats
                if let Some(stripped) = name.strip_prefix("flatpak:") {
                    stripped
                } else if let Some(stripped) = name.strip_prefix("nix:") {
                    stripped
                } else {
                    name
                }
            }
            PackageEntry::WithType { name, .. } => name,
        }
    }

    pub fn package_type(&self) -> PackageType {
        match self {
            PackageEntry::Simple(name) => {
                if name.starts_with("flatpak:") {
                    PackageType::Flatpak
                } else if name.starts_with("nix:") {
                    PackageType::Nix
                } else {
                    PackageType::Native
                }
            }
            PackageEntry::WithType { r#type, .. } => r#type.clone().unwrap_or(PackageType::Native),
        }
    }
}

/// The type of a package entry (native system package, flatpak, or nix)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PackageType {
    /// Native system package (pacman on Arch)
    #[serde(alias = "pacman", alias = "native", rename = "native")]
    Native,
    #[serde(rename = "flatpak")]
    Flatpak,
    /// Nix package (managed via home-manager)
    #[serde(rename = "nix")]
    Nix,
}

/// The system package manager to use for native package operations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageManagerType {
    /// Arch Linux pacman (with AUR helper support)
    Pacman,
}

/// Package list file (base.yaml, host files, legacy modules)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageList {
    /// Description of the package list
    #[serde(default)]
    pub description: String,

    /// List of packages
    #[serde(default)]
    pub packages: Vec<PackageEntry>,

    /// Packages to exclude (only in host files)
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Conflicting modules (only in modules)
    #[serde(default)]
    pub conflicts: Vec<String>,

    /// Pre-install hook script path (only in modules)
    #[serde(default)]
    pub pre_install_hook: Option<String>,

    /// Post-install hook script path (only in modules)
    #[serde(default)]
    pub post_install_hook: Option<String>,

    /// Hook behavior: "ask" (default), "always", "skip", "once" (only in modules)
    /// DEPRECATED: Use pre_hook_behavior and post_hook_behavior for independent control
    #[serde(default = "default_hook_behavior", alias = "hooks_behavior")]
    pub hook_behavior: String,

    /// Pre-install hook behavior (overrides hook_behavior for pre-install hook)
    #[serde(default)]
    pub pre_hook_behavior: Option<String>,

    /// Post-install hook behavior (overrides hook_behavior for post-install hook)
    #[serde(default)]
    pub post_hook_behavior: Option<String>,

    /// Run hooks as specified user instead of with sudo (default: false)
    /// Can be: false (default, use sudo), true (current user), or "username"
    #[serde(default)]
    pub run_hooks_as_user: RunHooksAsUser,

    /// Post-disable hook script path (only in modules)
    /// Runs after a module is disabled during sync
    #[serde(default)]
    pub post_disable_hook: Option<String>,

    /// Post-disable hook behavior: "ask" (default), "always", "skip", "once"
    #[serde(default)]
    pub post_disable_behavior: Option<String>,
}

/// Module structure - can be legacy (single file), directory-based, Lua, or Nix
#[derive(Debug, Clone)]
pub enum ModuleStructure {
    /// Legacy format: single .yaml file with all content
    Legacy { path: PathBuf, content: PackageList },
    /// Directory format: module/ directory with module.yaml manifest
    Directory(DirectoryModule),
    /// Lua format: single .lua file with dynamic configuration
    Lua(crate::lua::LuaModule),
    /// Nix format: single .nix file with dynamic configuration
    Nix(DynamicModule),
}

/// Dynamic module structure for Nix-based modules
#[allow(dead_code)] // kept: fields parsed from Nix module config; retained for round-trip fidelity
#[derive(Debug, Clone)]
pub struct DynamicModule {
    /// Path to the .nix file
    pub path: PathBuf,
    pub description: String,
    pub packages: Vec<PackageEntry>,
    pub services: ServicesConfig,
    pub conflicts: Vec<String>,
    pub pre_install_hook: Option<String>,
    pub post_install_hook: Option<String>,
    pub hook_behavior: String,
    pub pre_hook_behavior: Option<String>,
    pub post_hook_behavior: Option<String>,
    pub post_disable_hook: Option<String>,
    pub post_disable_behavior: Option<String>,
    pub run_hooks_as_user: RunHooksAsUser,
    /// reserved for future use
    #[allow(dead_code)]
    pub metadata: Option<serde_json::Value>,
    // Informational metadata fields (kept for module manifests, not actively read)
    #[allow(dead_code)]
    pub author: Option<String>,
    #[allow(dead_code)]
    pub version: Option<String>,
    #[allow(dead_code)]
    pub category: Option<String>,
    #[allow(dead_code)]
    pub tags: Vec<String>,
    #[allow(dead_code)]
    pub license: Option<String>,
    #[allow(dead_code)]
    pub upstream_url: Option<String>,
}

impl ModuleStructure {
    /// Get the description from any format
    pub fn description(&self) -> &str {
        match self {
            ModuleStructure::Legacy { content, .. } => &content.description,
            ModuleStructure::Directory(dir) => &dir.manifest.description,
            ModuleStructure::Lua(lua) => &lua.description,
            ModuleStructure::Nix(dyn_mod) => &dyn_mod.description,
        }
    }

    /// Get conflicts from any format
    pub fn conflicts(&self) -> &[String] {
        match self {
            ModuleStructure::Legacy { content, .. } => &content.conflicts,
            ModuleStructure::Directory(dir) => &dir.manifest.conflicts,
            ModuleStructure::Lua(lua) => &lua.conflicts,
            ModuleStructure::Nix(dyn_mod) => &dyn_mod.conflicts,
        }
    }

    /// Get pre-install hook from any format
    pub fn pre_install_hook(&self) -> Option<&str> {
        match self {
            ModuleStructure::Legacy { content, .. } => content.pre_install_hook.as_deref(),
            ModuleStructure::Directory(dir) => dir.manifest.pre_install_hook.as_deref(),
            ModuleStructure::Lua(lua) => lua.pre_install_hook.as_deref(),
            ModuleStructure::Nix(dyn_mod) => dyn_mod.pre_install_hook.as_deref(),
        }
    }

    /// Get post-install hook from any format
    pub fn post_install_hook(&self) -> Option<&str> {
        match self {
            ModuleStructure::Legacy { content, .. } => content.post_install_hook.as_deref(),
            ModuleStructure::Directory(dir) => dir.manifest.post_install_hook.as_deref(),
            ModuleStructure::Lua(lua) => lua.post_install_hook.as_deref(),
            ModuleStructure::Nix(dyn_mod) => dyn_mod.post_install_hook.as_deref(),
        }
    }

    /// Get post-disable hook from any format
    pub fn post_disable_hook(&self) -> Option<&str> {
        match self {
            ModuleStructure::Legacy { content, .. } => content.post_disable_hook.as_deref(),
            ModuleStructure::Directory(dir) => dir.manifest.post_disable_hook.as_deref(),
            ModuleStructure::Lua(lua) => lua.post_disable_hook.as_deref(),
            ModuleStructure::Nix(dyn_mod) => dyn_mod.post_disable_hook.as_deref(),
        }
    }

    /// Get pre-install hook behavior (with fallback to hook_behavior)
    pub fn pre_hook_behavior(&self) -> &str {
        match self {
            ModuleStructure::Legacy { content, .. } => content
                .pre_hook_behavior
                .as_deref()
                .unwrap_or(&content.hook_behavior),
            ModuleStructure::Directory(dir) => dir
                .manifest
                .pre_hook_behavior
                .as_deref()
                .unwrap_or(&dir.manifest.hook_behavior),
            ModuleStructure::Lua(lua) => lua
                .pre_hook_behavior
                .as_deref()
                .unwrap_or(&lua.hook_behavior),
            ModuleStructure::Nix(dyn_mod) => dyn_mod
                .pre_hook_behavior
                .as_deref()
                .unwrap_or(&dyn_mod.hook_behavior),
        }
    }

    /// Get post-install hook behavior (with fallback to hook_behavior)
    pub fn post_hook_behavior(&self) -> &str {
        match self {
            ModuleStructure::Legacy { content, .. } => content
                .post_hook_behavior
                .as_deref()
                .unwrap_or(&content.hook_behavior),
            ModuleStructure::Directory(dir) => dir
                .manifest
                .post_hook_behavior
                .as_deref()
                .unwrap_or(&dir.manifest.hook_behavior),
            ModuleStructure::Lua(lua) => lua
                .post_hook_behavior
                .as_deref()
                .unwrap_or(&lua.hook_behavior),
            ModuleStructure::Nix(dyn_mod) => dyn_mod
                .post_hook_behavior
                .as_deref()
                .unwrap_or(&dyn_mod.hook_behavior),
        }
    }

    /// Get all packages from any format
    pub fn packages(&self) -> Vec<PackageEntry> {
        match self {
            ModuleStructure::Legacy { content, .. } => content.packages.clone(),
            ModuleStructure::Directory(dir) => {
                let mut all_packages = Vec::new();
                for pkg_list in &dir.package_lists {
                    all_packages.extend(pkg_list.packages.clone());
                }
                all_packages
            }
            ModuleStructure::Lua(lua) => lua.packages.clone(),
            ModuleStructure::Nix(dyn_mod) => dyn_mod.packages.clone(),
        }
    }

    /// Get the root directory for this module (for resolving relative paths)
    pub fn root_dir(&self) -> PathBuf {
        match self {
            ModuleStructure::Legacy { path, .. } => path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
            ModuleStructure::Directory(dir) => dir.root.clone(),
            ModuleStructure::Lua(lua) => lua
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
            ModuleStructure::Nix(dyn_mod) => dyn_mod
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
        }
    }

    /// Check if this is a directory module
    pub fn is_directory(&self) -> bool {
        matches!(self, ModuleStructure::Directory(_))
    }

    /// Check if this is a Lua module
    pub fn is_lua(&self) -> bool {
        matches!(self, ModuleStructure::Lua(_))
    }

    /// Check if this is a Nix/dynamic module
    pub fn is_nix(&self) -> bool {
        matches!(self, ModuleStructure::Nix(_))
    }

    /// Get the username to run hooks as (None = use sudo/root)
    pub fn run_hooks_as_user(&self) -> Option<String> {
        match self {
            ModuleStructure::Legacy { content, .. } => content.run_hooks_as_user.username(),
            ModuleStructure::Directory(dir) => dir.manifest.run_hooks_as_user.username(),
            ModuleStructure::Lua(lua) => lua.run_hooks_as_user.username(),
            ModuleStructure::Nix(dyn_mod) => dyn_mod.run_hooks_as_user.username(),
        }
    }
}

/// Directory-based module structure
#[derive(Debug, Clone)]
pub struct DirectoryModule {
    /// Root directory of the module
    pub root: PathBuf,

    /// Module manifest (module.yaml)
    pub manifest: ModuleManifest,

    /// Loaded package lists from all package YAML files
    pub package_lists: Vec<PackageList>,

    /// Package file paths (for reference)
    pub package_file_paths: Vec<PathBuf>,

    /// Scripts directory (if exists)
    pub scripts_dir: Option<PathBuf>,
}

/// Module manifest (module.yaml in directory-based modules)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleManifest {
    /// Description of the module
    #[serde(default)]
    pub description: String,

    /// Conflicting modules
    #[serde(default)]
    pub conflicts: Vec<String>,

    /// Pre-install hook script path (relative to module directory)
    #[serde(default)]
    pub pre_install_hook: Option<String>,

    /// Post-install hook script path (relative to module directory)
    #[serde(default)]
    pub post_install_hook: Option<String>,

    /// Hook behavior: "ask" (default), "always", "skip", "once"
    /// DEPRECATED: Use pre_hook_behavior and post_hook_behavior for independent control
    #[serde(default = "default_hook_behavior", alias = "hooks_behavior")]
    pub hook_behavior: String,

    /// Pre-install hook behavior (overrides hook_behavior for pre-install hook)
    #[serde(default)]
    pub pre_hook_behavior: Option<String>,

    /// Post-install hook behavior (overrides hook_behavior for post-install hook)
    #[serde(default)]
    pub post_hook_behavior: Option<String>,

    /// Run hooks as specified user instead of with sudo (default: false)
    /// Can be: false (default, use sudo), true (current user), or "username"
    #[serde(default)]
    pub run_hooks_as_user: RunHooksAsUser,

    /// Post-disable hook script path (relative to module directory)
    /// Runs after a module is disabled during sync
    #[serde(default)]
    pub post_disable_hook: Option<String>,

    /// Post-disable hook behavior: "ask" (default), "always", "skip", "once"
    #[serde(default)]
    pub post_disable_behavior: Option<String>,

    /// Explicit list of package files to load (empty = auto-discover)
    #[serde(default)]
    pub package_files: Vec<String>,

    /// Auto-sync dotfiles/ directories to ~/.config/
    #[serde(default)]
    pub dotfiles_sync: Option<bool>,

    /// Explicit dotfiles list with custom source/target
    #[serde(default)]
    pub dotfiles: Vec<DotfileEntry>,

    // === Informational metadata fields (kept on the manifest) ===
    /// Module author (e.g. a username)
    #[serde(default)]
    pub author: Option<String>,

    /// Module version (semver format: X.Y.Z)
    #[serde(default)]
    pub version: Option<String>,

    /// Category for organization (defaults to "other" if not specified)
    #[serde(default)]
    pub category: Option<String>,

    /// Tags for search/filtering
    #[serde(default)]
    pub tags: Vec<String>,

    /// License identifier (e.g., "MIT", "GPL-3.0")
    #[serde(default)]
    pub license: Option<String>,

    /// URL to upstream project/documentation
    #[serde(default)]
    pub upstream_url: Option<String>,
}

fn default_hook_behavior() -> String {
    "ask".to_string()
}

/// Dotfile entry with explicit source and target paths
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotfileEntry {
    /// Source path relative to module root
    pub source: String,

    /// Target path (supports ~ expansion)
    pub target: String,
}

/// A SOPS-encrypted secret: an encrypted source in the config repo that is
/// decrypted into a plaintext target (copied, never symlinked) during sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEntry {
    /// Encrypted source path, relative to the config repo root.
    pub source: String,

    /// Plaintext target path (supports `~` expansion). Must not be inside the
    /// config repo — that guard prevents leaking plaintext into git.
    pub target: String,

    /// Octal file mode for the decrypted target, e.g. "0600" (the default).
    #[serde(default)]
    pub mode: Option<String>,

    /// Stable identifier for `mdots secrets edit/status/list`.
    /// Defaults to the target's file name when absent.
    #[serde(default)]
    pub name: Option<String>,
}

/// Configuration paths
#[derive(Debug, Clone)]
pub struct ConfigPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub packages_dir: PathBuf,
    pub state_dir: PathBuf,
    pub state_file: PathBuf,
    pub hooks_state_file: PathBuf,
    pub services_state_file: PathBuf,
    pub defaults_state_file: PathBuf,
    pub theming_state_file: PathBuf,
    pub config_backups_dir: PathBuf,
}

impl ConfigPaths {
    /// Create configuration paths from environment or defaults.
    /// Checks for config directories in this order:
    /// 1. MDOTS_CONFIG_DIR env var (if set)
    /// 2. ARCH_CONFIG_DIR env var (legacy, if set)
    /// 3. ~/.config/mdots/ (new default)
    /// 4. ~/.config/arch-config/ (legacy fallback)
    pub fn new() -> Result<Self> {
        let config_dir = if let Ok(custom) = std::env::var("MDOTS_CONFIG_DIR") {
            PathBuf::from(custom)
        } else if let Ok(custom) = std::env::var("ARCH_CONFIG_DIR") {
            PathBuf::from(custom)
        } else {
            let home = std::env::var("HOME").context("HOME environment variable not set")?;
            let new_path = PathBuf::from(&home).join(".config/mdots");
            let legacy_path = PathBuf::from(&home).join(".config/arch-config");

            if new_path.exists() {
                new_path
            } else if legacy_path.exists() {
                legacy_path
            } else {
                // Default to new path for fresh installs
                new_path
            }
        };

        let config_file = config_dir.join("config.yaml");
        let packages_dir = config_dir.join("packages");
        let state_dir = config_dir.join("state");
        let state_file = state_dir.join("installed.yaml");
        let hooks_state_file = state_dir.join("hooks-executed.yaml");
        let services_state_file = state_dir.join("services-state.yaml");
        let defaults_state_file = state_dir.join("defaults-state.yaml");
        let theming_state_file = state_dir.join("theming-state.yaml");
        let config_backups_dir = state_dir.join("config-backups");

        Ok(Self {
            config_dir,
            config_file,
            packages_dir,
            state_dir,
            state_file,
            hooks_state_file,
            services_state_file,
            defaults_state_file,
            theming_state_file,
            config_backups_dir,
        })
    }

    /// Get the hosts directory (supports both old and new structure)
    pub fn hosts_dir(&self) -> PathBuf {
        let new_path = self.config_dir.join("hosts");
        if new_path.exists() {
            new_path
        } else {
            // Fallback to old location
            self.packages_dir.join("hosts")
        }
    }

    /// Get the modules directory (supports both old and new structure)
    pub fn modules_dir(&self) -> PathBuf {
        let new_path = self.config_dir.join("modules");
        if new_path.exists() {
            new_path
        } else {
            // Fallback to old location
            self.packages_dir.join("modules")
        }
    }

    /// Get the services profiles directory
    pub fn services_dir(&self) -> PathBuf {
        self.config_dir.join("services")
    }

    /// Get the sources directory (build-from-source configs)
    pub fn sources_dir(&self) -> PathBuf {
        self.config_dir.join("sources")
    }

    /// Get the home-manager directory
    pub fn home_manager_dir(&self) -> PathBuf {
        self.config_dir.join("home-manager")
    }

    /// Get the base packages file (supports both old and new structure)
    pub fn base_packages_file(&self) -> PathBuf {
        let new_nix = self.config_dir.join("modules").join("base.nix");
        let new_lua = self.config_dir.join("modules").join("base.lua");
        let new_yaml = self.config_dir.join("modules").join("base.yaml");

        if new_nix.exists() {
            new_nix
        } else if new_lua.exists() {
            new_lua
        } else if new_yaml.exists() {
            new_yaml
        } else {
            self.packages_dir.join("base.yaml")
        }
    }

    /// Get a host-specific configuration file (supports both old and new structure)
    /// Now supports .yaml, .lua, and .nix files, preferring lua > nix > yaml.
    /// Also supports directory-based structure: hosts/{hostname}/host.lua or host.nix
    pub fn host_packages_file(&self, hostname: &str) -> PathBuf {
        let host_dir = self.config_dir.join("hosts").join(hostname);

        // Check for directory structure
        if host_dir.is_dir() {
            let nix_config = self.config_dir.join("config.nix");
            let lua_config = self.config_dir.join("config.lua");

            // In Nix mode, prefer host.nix; in Lua mode, prefer host.lua
            if nix_config.exists() && !lua_config.exists() {
                let host_nix = host_dir.join("host.nix");
                if host_nix.exists() {
                    return host_nix;
                }
            }
            return host_dir.join("host.lua");
        }

        let lua_filename = format!("{}.lua", hostname);
        let nix_filename = format!("{}.nix", hostname);
        let yaml_filename = format!("{}.yaml", hostname);

        let new_lua_path = self.config_dir.join("hosts").join(&lua_filename);
        let new_nix_path = self.config_dir.join("hosts").join(&nix_filename);
        let new_yaml_path = self.config_dir.join("hosts").join(&yaml_filename);

        if new_lua_path.exists() {
            return new_lua_path;
        }
        if new_nix_path.exists() {
            return new_nix_path;
        }
        if new_yaml_path.exists() {
            return new_yaml_path;
        }

        let old_lua_path = self.packages_dir.join("hosts").join(&lua_filename);
        let old_nix_path = self.packages_dir.join("hosts").join(&nix_filename);
        let old_yaml_path = self.packages_dir.join("hosts").join(&yaml_filename);

        if old_lua_path.exists() {
            return old_lua_path;
        }
        if old_nix_path.exists() {
            return old_nix_path;
        }
        if old_yaml_path.exists() {
            return old_yaml_path;
        }

        new_yaml_path
    }
}

impl Default for ConfigPaths {
    fn default() -> Self {
        Self::new().expect("Failed to create default config paths")
    }
}

/// Resolve the effective config file path (config.lua, config.nix, host file, or config.yaml).
pub fn resolve_config_path(paths: &ConfigPaths) -> Result<PathBuf> {
    // Check for config.lua first
    let lua_config_file = paths.config_dir.join("config.lua");
    if lua_config_file.exists() {
        // Check that config.nix doesn't also exist
        let nix_config_file = paths.config_dir.join("config.nix");
        if nix_config_file.exists() {
            anyhow::bail!(
                "Cannot have both config.lua and config.nix. Please remove one.\n\
                 Run 'mdots init --lua' for Lua or 'mdots init --nix' for Nix."
            );
        }

        if let Some(hostname) = crate::lua::detect_pointer_lua_config(&lua_config_file)? {
            let host_file = paths.host_packages_file(&hostname);
            if !host_file.exists() {
                anyhow::bail!(
                    "Host file not found: {:?}\nconfig.lua is a pointer but host file doesn't exist",
                    host_file
                );
            }
            return Ok(host_file);
        }
        return Ok(lua_config_file);
    }

    // Check for config.nix next
    let nix_config_file = paths.config_dir.join("config.nix");
    if nix_config_file.exists() {
        if crate::nix_eval::is_nix_installed() {
            if let Some(hostname) =
                crate::nix_eval::detect_pointer_nix_config_file(&nix_config_file)?
            {
                let host_file = paths.host_packages_file(&hostname);
                if !host_file.exists() {
                    anyhow::bail!(
                        "Host file not found: {:?}\nconfig.nix is a pointer but host file doesn't exist",
                        host_file
                    );
                }
                return Ok(host_file);
            }
            return Ok(nix_config_file);
        } else {
            eprintln!("config.nix found but nix is not installed. Falling back to config.yaml.");
        }
    }

    // Fall back to config.yaml
    let content = std::fs::read_to_string(&paths.config_file).context(format!(
        "Failed to read config file: {:?}",
        paths.config_file
    ))?;

    if is_pointer_config_raw(&content) {
        let config: Config =
            serde_yaml::from_str(&content).context("Failed to parse config.yaml")?;

        let host_file = paths.host_packages_file(&config.host);
        if !host_file.exists() {
            anyhow::bail!(
                "Host file not found: {:?}\nConfig.yaml is a pointer but host file doesn't exist",
                host_file
            );
        }
        return Ok(host_file);
    }

    Ok(paths.config_file.clone())
}

/// Check if the active config file is Lua-based.
pub fn is_lua_config(paths: &ConfigPaths) -> Result<bool> {
    let config_path = resolve_config_path(paths)?;
    Ok(config_path.extension().and_then(|e| e.to_str()) == Some("lua"))
}

/// Check if the active config file is Nix-based.
pub fn is_nix_config(paths: &ConfigPaths) -> Result<bool> {
    let config_path = resolve_config_path(paths)?;
    Ok(config_path.extension().and_then(|e| e.to_str()) == Some("nix"))
}

/// Get preferred and fallback declared-packages paths based on config format.
/// Prefers host directory structure (hosts/{hostname}/) when the config uses
/// directory-based hosts. Falls back to modules/ for file-based hosts or legacy configs.
pub fn declared_packages_paths(paths: &ConfigPaths) -> Result<(PathBuf, PathBuf)> {
    let prefer_lua = is_lua_config(paths).unwrap_or(false);
    let prefer_nix = is_nix_config(paths).unwrap_or(false);

    let (preferred_ext, fallback_ext) = if prefer_nix {
        ("nix", "yaml")
    } else if prefer_lua {
        ("lua", "yaml")
    } else {
        ("yaml", "lua")
    };

    let config_path = resolve_config_path(paths)?;
    let host_filename = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let dir_based_host = config_path
        .parent()
        .map(|p| {
            p.is_dir()
                && (host_filename == "host.lua" || host_filename == "host.nix")
                && config_path.starts_with(paths.hosts_dir())
        })
        .unwrap_or(false);

    let base_dir = if dir_based_host {
        config_path.parent().unwrap().to_path_buf()
    } else {
        paths.modules_dir()
    };

    Ok((
        base_dir.join(format!("declared-packages.{}", preferred_ext)),
        base_dir.join(format!("declared-packages.{}", fallback_ext)),
    ))
}

/// Load main configuration file
/// Supports both pointer configs (just host field) and full configs (legacy)
/// Also handles import: directive for loading additional config files
/// Now supports both YAML and Lua config files
pub fn load_config(paths: &ConfigPaths) -> Result<Config> {
    // Read the pointer config to extract package_manager before resolving
    let pointer_pkg_manager = read_pointer_package_manager(paths);

    let config_path = resolve_config_path(paths)?;
    let mut config = load_config_from_file(paths, &config_path)?;

    // If the pointer config specified a package_manager, apply it to the loaded config
    // (the pointer config is the authoritative source for this field)
    if let Some(pm) = pointer_pkg_manager {
        config.package_manager = Some(pm);
    }

    // If the loaded config is from a host directory, check for companion packages file
    if let Some(parent) = config_path.parent() {
        let packages_nix = parent.join("packages.nix");
        let packages_lua = parent.join("packages.lua");
        let packages_yaml = parent.join("packages.yaml");
        if packages_nix.exists() {
            if let Ok(pkg_list) = load_package_list_any(&packages_nix) {
                config.packages.extend(pkg_list.packages);
            }
        } else if packages_lua.exists() {
            if let Ok(pkg_list) = load_package_list_any(&packages_lua) {
                config.packages.extend(pkg_list.packages);
            }
        } else if packages_yaml.exists() {
            if let Ok(pkg_list) = load_package_list_any(&packages_yaml) {
                config.packages.extend(pkg_list.packages);
            }
        }
    }

    Ok(config)
}

/// Read the package_manager field from the pointer config.yaml (if it's a pointer)
fn read_pointer_package_manager(paths: &ConfigPaths) -> Option<PackageManagerType> {
    let content = std::fs::read_to_string(&paths.config_file).ok()?;
    if !is_pointer_config_raw(&content) {
        return None;
    }
    let config: Config = serde_yaml::from_str(&content).ok()?;
    config.package_manager
}

/// Check if a config file is a "pointer" config by examining raw YAML content
/// A pointer config only has the "host" field (and optionally "package_manager")
fn is_pointer_config_raw(yaml_content: &str) -> bool {
    // Parse as a raw YAML value to check field count
    let value: serde_yaml::Value = match serde_yaml::from_str(yaml_content) {
        Ok(v) => v,
        Err(_) => return false,
    };

    // Check if it's a mapping with only pointer fields (host, and optionally package_manager)
    if let serde_yaml::Value::Mapping(map) = value {
        let has_host = map.contains_key(serde_yaml::Value::String("host".to_string()));
        let has_pkg_mgr =
            map.contains_key(serde_yaml::Value::String("package_manager".to_string()));

        // A pointer config has "host" and optionally "package_manager", nothing else
        if has_host && (map.len() == 1 || (map.len() == 2 && has_pkg_mgr)) {
            return true;
        }
    }

    false
}

/// Load a config file from a specific path (used for host files)
/// Supports YAML, Lua, and Nix config files based on extension
fn load_config_from_file(paths: &ConfigPaths, file_path: &Path) -> Result<Config> {
    let extension = file_path.extension().and_then(|e| e.to_str());

    let config = match extension {
        Some("nix") => crate::nix_eval::load_nix_config(file_path)?,
        Some("lua") => {
            let content = std::fs::read_to_string(file_path)
                .context(format!("Failed to read Lua config: {:?}", file_path))?;

            if content.trim_start().starts_with("host:") || content.trim_start().starts_with("---")
            {
                anyhow::bail!(
                    "Failed to execute Lua config {:?}: syntax error\n\n\
                     The file appears to contain YAML content but has a .lua extension.\n\
                     This can happen if the config was previously overwritten.\n\n\
                     To fix this, rename the file to use the correct extension:\n\
                     mv {} {}",
                    file_path,
                    file_path.display(),
                    file_path.with_extension("yaml").display()
                );
            }

            crate::lua::load_lua_config(file_path)?
        }
        Some("yaml") | Some("yml") | None => {
            let content = std::fs::read_to_string(file_path)
                .context(format!("Failed to read config file: {:?}", file_path))?;

            serde_yaml::from_str(&content)
                .context(format!("Failed to parse config: {:?}", file_path))?
        }
        _ => {
            anyhow::bail!("Unsupported config file type: {:?}", file_path)
        }
    };

    if !config.import.is_empty() {
        return load_config_with_imports(paths, file_path, &mut std::collections::HashSet::new());
    }

    Ok(config)
}

/// Load a config with imports, handling circular import detection
/// Supports both YAML and Lua config files
fn load_config_with_imports(
    paths: &ConfigPaths,
    config_path: &Path,
    visited: &mut std::collections::HashSet<PathBuf>,
) -> Result<Config> {
    // Canonicalize the path to detect circular imports
    let canonical = config_path
        .canonicalize()
        .context(format!("Failed to canonicalize path: {:?}", config_path))?;

    // Check for circular imports
    if visited.contains(&canonical) {
        anyhow::bail!("Circular import detected: {:?}", config_path);
    }

    visited.insert(canonical.clone());

    // Load the main config based on file extension
    let extension = config_path.extension().and_then(|e| e.to_str());
    let mut config: Config = match extension {
        Some("nix") => crate::nix_eval::load_nix_config(config_path)?,
        Some("lua") => crate::lua::load_lua_config(config_path)?,
        Some("yaml") | Some("yml") | None => {
            let content = std::fs::read_to_string(config_path)
                .context(format!("Failed to read config file: {:?}", config_path))?;
            serde_yaml::from_str(&content)
                .context(format!("Failed to parse config: {:?}", config_path))?
        }
        _ => anyhow::bail!("Unsupported config file type: {:?}", config_path),
    };

    // Clone the import list to avoid borrow issues
    let import_list = config.import.clone();

    // Load and merge each import
    for import_path in import_list {
        let full_path = if Path::new(&import_path).is_absolute() {
            PathBuf::from(&import_path)
        } else {
            // Resolve relative to arch-config root
            paths.config_dir.join(&import_path)
        };

        if !full_path.exists() {
            eprintln!(
                "⚠️  Warning: Import file not found, skipping: {:?}",
                import_path
            );
            continue;
        }

        // Validate that import path is within arch-config directory
        let full_canonical = full_path.canonicalize().context(format!(
            "Failed to canonicalize import path: {:?}",
            full_path
        ))?;

        let config_canonical = paths
            .config_dir
            .canonicalize()
            .context("Failed to canonicalize arch-config directory")?;

        if !full_canonical.starts_with(&config_canonical) {
            anyhow::bail!(
                "Security error: Import path outside arch-config directory: {:?}",
                import_path
            );
        }

        // Recursively load imported config
        let imported = load_config_with_imports(paths, &full_path, visited)?;

        // Merge into main config
        config.merge(imported);
    }

    Ok(config)
}

/// Load a package list file (base, host, or legacy module)
pub fn load_package_list<P: AsRef<Path>>(path: P) -> Result<PackageList> {
    let content = std::fs::read_to_string(path.as_ref())
        .context(format!("Failed to read package list: {:?}", path.as_ref()))?;

    serde_yaml::from_str(&content).context("Failed to parse package list YAML")
}

/// Load a package list from YAML, Lua, or Nix based on file extension.
pub fn load_package_list_any<P: AsRef<Path>>(path: P) -> Result<PackageList> {
    let path = path.as_ref();
    let extension = path.extension().and_then(|e| e.to_str());

    match extension {
        Some("lua") => load_package_list_lua(path),
        Some("nix") => load_package_list_nix(path),
        Some("yaml") | Some("yml") | None => load_package_list(path),
        _ => anyhow::bail!("Unsupported package list type: {:?}", path),
    }
}

fn load_package_list_nix(path: &Path) -> Result<PackageList> {
    let nix_module = crate::nix_eval::load_nix_module(path)?;

    Ok(PackageList {
        description: nix_module.description,
        packages: nix_module.packages,
        exclude: Vec::new(),
        conflicts: nix_module.conflicts,
        pre_install_hook: nix_module.pre_install_hook,
        post_install_hook: nix_module.post_install_hook,
        hook_behavior: nix_module.hook_behavior,
        pre_hook_behavior: nix_module.pre_hook_behavior,
        post_hook_behavior: nix_module.post_hook_behavior,
        run_hooks_as_user: nix_module.run_hooks_as_user,
        post_disable_hook: nix_module.post_disable_hook,
        post_disable_behavior: nix_module.post_disable_behavior,
    })
}

/// Write a package list to YAML, Lua, or Nix based on file extension.
pub fn write_package_list_any<P: AsRef<Path>>(path: P, list: &PackageList) -> Result<()> {
    let path = path.as_ref();
    let extension = path.extension().and_then(|e| e.to_str());

    let content = match extension {
        Some("nix") => package_list_to_nix(list),
        Some("lua") => package_list_to_lua(list),
        Some("yaml") | Some("yml") | None => {
            serde_yaml::to_string(list).context("Failed to serialize package list")?
        }
        _ => anyhow::bail!("Unsupported package list type: {:?}", path),
    };

    // Atomic write (temp sibling + rename) so an interrupt mid-write can't
    // corrupt the package manifest inside the user's tracked dotfiles repo.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".mdots-tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, content).context(format!("Failed to write package list: {:?}", tmp))?;
    std::fs::rename(&tmp, path).context(format!("Failed to write package list: {:?}", path))?;

    Ok(())
}

fn load_package_list_lua(path: &Path) -> Result<PackageList> {
    let lua_module = crate::lua::load_lua_module(path)?;

    Ok(PackageList {
        description: lua_module.description,
        packages: lua_module.packages,
        exclude: Vec::new(),
        conflicts: lua_module.conflicts,
        pre_install_hook: lua_module.pre_install_hook,
        post_install_hook: lua_module.post_install_hook,
        hook_behavior: lua_module.hook_behavior,
        pre_hook_behavior: lua_module.pre_hook_behavior,
        post_hook_behavior: lua_module.post_hook_behavior,
        run_hooks_as_user: lua_module.run_hooks_as_user,
        post_disable_hook: None,
        post_disable_behavior: None,
    })
}

fn package_list_to_lua(list: &PackageList) -> String {
    let mut out = String::new();
    out.push_str("return {\n");

    if !list.description.is_empty() {
        out.push_str("    description = ");
        out.push_str(&lua_string(&list.description));
        out.push_str(",\n");
    }

    if list.packages.is_empty() {
        out.push_str("    packages = {},\n");
    } else {
        out.push_str("    packages = {\n");
        for entry in &list.packages {
            out.push_str("        ");
            out.push_str(&format_package_entry_lua(entry));
            out.push_str(",\n");
        }
        out.push_str("    },\n");
    }

    if !list.conflicts.is_empty() {
        out.push_str("    conflicts = {\n");
        for conflict in &list.conflicts {
            out.push_str("        ");
            out.push_str(&lua_string(conflict));
            out.push_str(",\n");
        }
        out.push_str("    },\n");
    }

    if let Some(ref hook) = list.pre_install_hook {
        out.push_str("    pre_install_hook = ");
        out.push_str(&lua_string(hook));
        out.push_str(",\n");
    }

    if let Some(ref hook) = list.post_install_hook {
        out.push_str("    post_install_hook = ");
        out.push_str(&lua_string(hook));
        out.push_str(",\n");
    }

    if !list.hook_behavior.is_empty() && list.hook_behavior != "ask" {
        out.push_str("    hook_behavior = ");
        out.push_str(&lua_string(&list.hook_behavior));
        out.push_str(",\n");
    }

    if let Some(ref behavior) = list.pre_hook_behavior {
        out.push_str("    pre_hook_behavior = ");
        out.push_str(&lua_string(behavior));
        out.push_str(",\n");
    }

    if let Some(ref behavior) = list.post_hook_behavior {
        out.push_str("    post_hook_behavior = ");
        out.push_str(&lua_string(behavior));
        out.push_str(",\n");
    }

    out.push_str("}\n");
    out
}

fn format_package_entry_lua(entry: &PackageEntry) -> String {
    match entry {
        PackageEntry::Simple(name) => lua_string(name),
        PackageEntry::WithType { name, r#type } => match r#type {
            Some(PackageType::Flatpak) => {
                format!("{{ name = {}, type = \"flatpak\" }}", lua_string(name))
            }
            _ => lua_string(name),
        },
    }
}

fn lua_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('"');
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Load a module (detects legacy vs directory format automatically)
pub fn load_module<P: AsRef<Path>>(path: P) -> Result<ModuleStructure> {
    let path = path.as_ref();

    if path.is_file() {
        let extension = path.extension().and_then(|e| e.to_str());

        match extension {
            Some("nix") => {
                let nix_module = crate::nix_eval::load_nix_module_as_module_structure(path)?;
                Ok(nix_module)
            }
            Some("lua") => {
                let lua_module = crate::lua::load_lua_module(path)?;
                Ok(ModuleStructure::Lua(lua_module))
            }
            Some("yaml") | Some("yml") | None => {
                let content = load_package_list(path)?;
                Ok(ModuleStructure::Legacy {
                    path: path.to_path_buf(),
                    content,
                })
            }
            _ => {
                anyhow::bail!("Unsupported module file type: {:?}", path)
            }
        }
    } else if path.is_dir() {
        let nix_manifest_path = path.join("module.nix");
        let lua_manifest_path = path.join("module.lua");
        let yaml_manifest_path = path.join("module.yaml");
        let mut inline_packages: Vec<PackageEntry> = Vec::new();

        let manifest: ModuleManifest = if nix_manifest_path.exists() {
            log::debug!("Loading Nix manifest: {:?}", nix_manifest_path);
            let (manifest, pkgs) = crate::nix_eval::load_nix_directory_module(&nix_manifest_path)?;
            inline_packages = pkgs;
            manifest
        } else if lua_manifest_path.exists() {
            log::debug!("Loading Lua manifest: {:?}", lua_manifest_path);
            let lua_dir = crate::lua::load_lua_directory_module(&lua_manifest_path)?;
            inline_packages = lua_dir.packages;
            lua_dir.manifest
        } else if yaml_manifest_path.exists() {
            let manifest_content = std::fs::read_to_string(&yaml_manifest_path).context(
                format!("Failed to read module.yaml: {:?}", yaml_manifest_path),
            )?;
            serde_yaml::from_str(&manifest_content).context("Failed to parse module.yaml")?
        } else {
            let discovered_files = discover_package_files(path)?;
            if discovered_files.is_empty() {
                anyhow::bail!(
                    "Directory module must contain 'module.nix', 'module.lua', 'module.yaml', or package files: {:?}",
                    path
                );
            }

            let module_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            log::debug!(
                "Creating default manifest for package-only directory module: {:?}",
                path
            );
            ModuleManifest {
                description: format!("Module: {}", module_name),
                package_files: Vec::new(),
                dotfiles: Vec::new(),
                dotfiles_sync: None,
                conflicts: Vec::new(),
                pre_install_hook: None,
                post_install_hook: None,
                hook_behavior: "ask".to_string(),
                pre_hook_behavior: None,
                post_hook_behavior: None,
                run_hooks_as_user: RunHooksAsUser::Bool(false),
                post_disable_hook: None,
                post_disable_behavior: None,
                author: None,
                version: None,
                category: None,
                tags: Vec::new(),
                license: None,
                upstream_url: None,
            }
        };

        // Discover or load package files
        let mut package_file_paths = if manifest.package_files.is_empty() {
            discover_package_files(path)?
        } else {
            manifest
                .package_files
                .iter()
                .map(|f| path.join(f))
                .collect()
        };

        // Load all package lists
        let mut package_lists = Vec::new();
        for pkg_path in &package_file_paths {
            if !pkg_path.exists() {
                anyhow::bail!("Package file not found: {:?}", pkg_path);
            }
            let pkg_list = load_package_list_any(pkg_path)?;
            package_lists.push(pkg_list);
        }

        if !inline_packages.is_empty() {
            package_file_paths.push(lua_manifest_path.clone());
            package_lists.push(PackageList {
                description: "Packages defined in module.lua".to_string(),
                packages: inline_packages,
                exclude: Vec::new(),
                conflicts: Vec::new(),
                pre_install_hook: None,
                post_install_hook: None,
                hook_behavior: "ask".to_string(),
                pre_hook_behavior: None,
                post_hook_behavior: None,
                run_hooks_as_user: RunHooksAsUser::Bool(false),
                post_disable_hook: None,
                post_disable_behavior: None,
            });
        }

        // Check for scripts and dotfiles directories
        let scripts_dir = path.join("scripts");
        let scripts_dir = if scripts_dir.exists() && scripts_dir.is_dir() {
            Some(scripts_dir)
        } else {
            None
        };

        Ok(ModuleStructure::Directory(DirectoryModule {
            root: path.to_path_buf(),
            manifest,
            package_lists,
            package_file_paths,
            scripts_dir,
        }))
    } else {
        anyhow::bail!("Module path is neither a file nor directory: {:?}", path)
    }
}

/// Discover package files in a module directory (excludes module.yaml/module.nix/module.lua)
fn discover_package_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let excluded = [
        std::ffi::OsStr::new("module.yaml"),
        std::ffi::OsStr::new("module.nix"),
        std::ffi::OsStr::new("module.lua"),
    ];

    for entry in std::fs::read_dir(dir).context(format!("Failed to read directory: {:?}", dir))? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() && !excluded.contains(&path.file_name().unwrap_or_default()) {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext == "yaml" || ext == "nix" || ext == "lua" {
                    files.push(path);
                }
            }
        }
    }

    files.sort();
    Ok(files)
}

/// Validation result for modules
#[derive(Debug, Clone)]
pub struct ModuleValidationResult {
    #[allow(dead_code)] // kept: populated for diagnostics; not yet surfaced to the user
    pub module_name: String,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ModuleValidationResult {
    pub fn new(module_name: String) -> Self {
        Self {
            module_name,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        self.errors.is_empty() && self.warnings.is_empty()
    }
}

/// Validate a module structure
pub fn validate_module(module: &ModuleStructure, module_name: &str) -> ModuleValidationResult {
    let mut result = ModuleValidationResult::new(module_name.to_string());

    match module {
        ModuleStructure::Legacy { content, .. } => {
            // Validate legacy module
            if content.description.is_empty() {
                result
                    .warnings
                    .push("Module has no description".to_string());
            }

            // Empty pre_install_hook is fine - just ignore it
            // Only validate if hook is actually specified with a non-empty value
            if let Some(hook) = &content.pre_install_hook {
                if !hook.is_empty() {
                    // Hook is specified - for legacy modules, we can't validate the path
                    // since it's resolved relative to config_dir at runtime
                    // Just check that it's not obviously invalid
                }
            }

            // Empty post_install_hook is fine - just ignore it
            // Only validate if hook is actually specified with a non-empty value
            if let Some(hook) = &content.post_install_hook {
                if !hook.is_empty() {
                    // Hook is specified - for legacy modules, we can't validate the path
                    // since it's resolved relative to config_dir at runtime
                    // Just check that it's not obviously invalid
                }
            }

            // Validate no duplicate packages
            let mut seen = std::collections::HashSet::new();
            for pkg in &content.packages {
                let name = pkg.name();
                if !seen.insert(name) {
                    result.errors.push(format!("Duplicate package: {}", name));
                }
            }
        }
        ModuleStructure::Directory(dir) => {
            // Validate directory module
            if dir.manifest.description.is_empty() {
                result
                    .warnings
                    .push("Module has no description".to_string());
            }

            // Validate package files exist and are non-empty
            if dir.package_lists.is_empty() {
                result
                    .warnings
                    .push("No package files found (module has no packages)".to_string());
            }

            // Check for empty package files
            for (idx, pkg_list) in dir.package_lists.iter().enumerate() {
                if pkg_list.packages.is_empty() {
                    let file_name = dir
                        .package_file_paths
                        .get(idx)
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");
                    result
                        .warnings
                        .push(format!("Package file '{}' is empty", file_name));
                }
            }

            // Validate pre-install hook exists if specified (non-empty)
            if let Some(hook) = &dir.manifest.pre_install_hook {
                if !hook.is_empty() {
                    // Hook is specified - validate it exists
                    let hook_path = if Path::new(hook).is_absolute() {
                        PathBuf::from(hook)
                    } else {
                        dir.root.join(hook)
                    };

                    if !hook_path.exists() {
                        result.errors.push(format!(
                            "pre_install_hook script not found: {} (resolved to {:?})",
                            hook, hook_path
                        ));
                    } else if !hook_path.is_file() {
                        result
                            .errors
                            .push(format!("pre_install_hook is not a file: {}", hook));
                    }

                    // If hook is in scripts/ subdirectory, ensure it's in our scripts dir
                    if hook.starts_with("scripts/") && dir.scripts_dir.is_none() {
                        result.errors.push(
                            "pre_install_hook references 'scripts/' but scripts directory doesn't exist".to_string()
                        );
                    }
                }
            }

            // Validate post-install hook exists if specified (non-empty)
            if let Some(hook) = &dir.manifest.post_install_hook {
                if !hook.is_empty() {
                    // Hook is specified - validate it exists
                    let hook_path = if Path::new(hook).is_absolute() {
                        PathBuf::from(hook)
                    } else {
                        dir.root.join(hook)
                    };

                    if !hook_path.exists() {
                        result.errors.push(format!(
                            "post_install_hook script not found: {} (resolved to {:?})",
                            hook, hook_path
                        ));
                    } else if !hook_path.is_file() {
                        result
                            .errors
                            .push(format!("post_install_hook is not a file: {}", hook));
                    }

                    // If hook is in scripts/ subdirectory, ensure it's in our scripts dir
                    if hook.starts_with("scripts/") && dir.scripts_dir.is_none() {
                        result.errors.push(
                            "post_install_hook references 'scripts/' but scripts directory doesn't exist".to_string()
                        );
                    }
                }
            }

            // Warn if scripts directory exists but no hook
            if dir.scripts_dir.is_some() && dir.manifest.post_install_hook.is_none() {
                result.warnings.push(
                    "scripts/ directory exists but no post_install_hook is configured".to_string(),
                );
            }

            // Note: dotfiles/ directory is allowed - users can handle dotfiles in their post-install hooks

            // Note: We don't check for duplicate packages across files
            // because the same package may legitimately appear in multiple modules
            // (user may enable one module but not another)

            // Validate package files in manifest actually exist
            for specified_file in &dir.manifest.package_files {
                let file_path = dir.root.join(specified_file);
                if !file_path.exists() {
                    result.errors.push(format!(
                        "Package file specified in manifest not found: {}",
                        specified_file
                    ));
                }
            }
        }
        ModuleStructure::Lua(lua) => {
            // Validate Lua module
            if lua.description.is_empty() {
                result
                    .warnings
                    .push("Module has no description".to_string());
            }

            // Validate no duplicate packages
            let mut seen = std::collections::HashSet::new();
            for pkg in &lua.packages {
                let name = pkg.name();
                if !seen.insert(name) {
                    result.errors.push(format!("Duplicate package: {}", name));
                }
            }

            // Validate hook paths if specified
            if let Some(hook) = &lua.pre_install_hook {
                if !hook.is_empty() {
                    let hook_path = lua
                        .path
                        .parent()
                        .map(|p| p.join(hook))
                        .unwrap_or_else(|| std::path::PathBuf::from(hook));
                    if !hook_path.exists() {
                        result
                            .errors
                            .push(format!("pre_install_hook script not found: {}", hook));
                    }
                }
            }

            if let Some(hook) = &lua.post_install_hook {
                if !hook.is_empty() {
                    let hook_path = lua
                        .path
                        .parent()
                        .map(|p| p.join(hook))
                        .unwrap_or_else(|| std::path::PathBuf::from(hook));
                    if !hook_path.exists() {
                        result
                            .errors
                            .push(format!("post_install_hook script not found: {}", hook));
                    }
                }
            }
        }
        ModuleStructure::Nix(dyn_mod) => {
            if dyn_mod.description.is_empty() {
                result
                    .warnings
                    .push("Module has no description".to_string());
            }

            let mut seen = std::collections::HashSet::new();
            for pkg in &dyn_mod.packages {
                let name = pkg.name();
                if !seen.insert(name) {
                    result.errors.push(format!("Duplicate package: {}", name));
                }
            }

            if let Some(hook) = &dyn_mod.pre_install_hook {
                if !hook.is_empty() {
                    let hook_path = dyn_mod
                        .path
                        .parent()
                        .map(|p| p.join(hook))
                        .unwrap_or_else(|| std::path::PathBuf::from(hook));
                    if !hook_path.exists() {
                        result
                            .errors
                            .push(format!("pre_install_hook script not found: {}", hook));
                    }
                }
            }

            if let Some(hook) = &dyn_mod.post_install_hook {
                if !hook.is_empty() {
                    let hook_path = dyn_mod
                        .path
                        .parent()
                        .map(|p| p.join(hook))
                        .unwrap_or_else(|| std::path::PathBuf::from(hook));
                    if !hook_path.exists() {
                        result
                            .errors
                            .push(format!("post_install_hook script not found: {}", hook));
                    }
                }
            }
        }
    }

    result
}

/// Resolve the editor to use for editing config files
/// Resolution order: 1) config.editor, 2) $EDITOR env var, 3) fallback (nano or vim)
pub fn resolve_editor(config: &Config) -> Result<String> {
    // Priority 1: Config file setting
    if let Some(ref editor) = config.editor {
        return Ok(editor.clone());
    }

    // Priority 2: $EDITOR environment variable
    if let Ok(editor) = std::env::var("EDITOR") {
        if !editor.trim().is_empty() {
            return Ok(editor);
        }
    }

    // Priority 3: Fallback to commonly available editors
    if which::which("nano").is_ok() {
        return Ok("nano".to_string());
    }

    if which::which("vim").is_ok() {
        return Ok("vim".to_string());
    }

    anyhow::bail!(
        "No editor found. Please set 'editor' in your config.yaml or set the $EDITOR environment variable."
    )
}

/// Resolve the effective package manager type from config or auto-detect from the system.
/// Resolution order: 1) config.package_manager, 2) auto-detect from /etc/os-release
pub fn resolve_package_manager(config: &Config) -> Result<PackageManagerType> {
    // Priority 1: Config file setting
    if let Some(ref pm) = config.package_manager {
        return Ok(pm.clone());
    }

    // Priority 2: Auto-detect from system
    detect_package_manager_type()
}

/// Detect the package manager type from the system
pub fn detect_package_manager_type() -> Result<PackageManagerType> {
    // Check /etc/os-release for distro family
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        let id = content
            .lines()
            .find(|l| l.starts_with("ID="))
            .map(|l| l.trim_start_matches("ID=").trim_matches('"').to_lowercase());
        let id_like = content
            .lines()
            .find(|l| l.starts_with("ID_LIKE="))
            .map(|l| {
                l.trim_start_matches("ID_LIKE=")
                    .trim_matches('"')
                    .to_lowercase()
            });

        if let Some(ref id) = id {
            // Direct ID matches for Arch and Arch-based distros
            if matches!(
                id.as_str(),
                "arch" | "manjaro" | "endeavouros" | "garuda" | "cachyos" | "artix"
            ) {
                return Ok(PackageManagerType::Pacman);
            }
        }

        // Check ID_LIKE for derivative distros
        if let Some(ref id_like) = id_like {
            if id_like.contains("arch") {
                return Ok(PackageManagerType::Pacman);
            }
        }
    }

    // Fallback: check that pacman exists
    if which::which("pacman").is_ok() {
        return Ok(PackageManagerType::Pacman);
    }

    anyhow::bail!(
        "mdots officially supports only Arch and Arch-based distributions (pacman). pacman was not found on this system. If you are building mdots on another distro, ensure pacman is available."
    )
}

/// Resolve the AUR helper to use for package management
/// Resolution order: 1) config.aur_helper, 2) auto-detect (paru, then yay)
pub fn resolve_aur_helper(config: &Config) -> Result<String> {
    // Priority 1: Config file setting
    if let Some(ref aur_helper) = config.aur_helper {
        // Validate the configured helper exists
        if which::which(aur_helper).is_ok() {
            return Ok(aur_helper.clone());
        } else {
            anyhow::bail!(
                "Configured AUR helper '{}' not found in PATH. Please install it or update your config.",
                aur_helper
            );
        }
    }

    // Priority 2: Auto-detect (prefer paru, then yay)
    if which::which("paru").is_ok() {
        return Ok("paru".to_string());
    }

    if which::which("yay").is_ok() {
        return Ok("yay".to_string());
    }

    anyhow::bail!(
        "No AUR helper found. Please install paru or yay, or set 'aur_helper' in your config.yaml."
    )
}

/// Serialize a PackageList to Nix format
pub fn package_list_to_nix(list: &PackageList) -> String {
    let mut out = String::new();
    out.push_str("{ system, pkgs }:\n\n{\n");

    if !list.description.is_empty() {
        out.push_str(&format!("  description = \"{}\";\n", list.description));
    }

    out.push_str("  packages = [\n");
    for entry in &list.packages {
        out.push_str(&format!("    \"{}\"\n", entry.name()));
    }
    out.push_str("  ];\n");

    if !list.exclude.is_empty() {
        out.push_str("  exclude = [\n");
        for e in &list.exclude {
            out.push_str(&format!("    \"{}\"\n", e));
        }
        out.push_str("  ];\n");
    }

    if !list.conflicts.is_empty() {
        out.push_str("  conflicts = [\n");
        for c in &list.conflicts {
            out.push_str(&format!("    \"{}\"\n", c));
        }
        out.push_str("  ];\n");
    }

    if let Some(ref hook) = list.pre_install_hook {
        out.push_str(&format!("  pre_install_hook = \"{}\";\n", hook));
    }
    if let Some(ref hook) = list.post_install_hook {
        out.push_str(&format!("  post_install_hook = \"{}\";\n", hook));
    }

    if !list.hook_behavior.is_empty() && list.hook_behavior != "ask" {
        out.push_str(&format!("  hook_behavior = \"{}\";\n", list.hook_behavior));
    }

    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_entry_simple() {
        let entry = PackageEntry::Simple("vim".to_string());
        assert_eq!(entry.name(), "vim");
        assert_eq!(entry.package_type(), PackageType::Native);
    }

    #[test]
    fn test_package_entry_flatpak_prefix() {
        let entry = PackageEntry::Simple("flatpak:com.spotify.Client".to_string());
        assert_eq!(entry.name(), "com.spotify.Client");
        assert_eq!(entry.package_type(), PackageType::Flatpak);
    }

    #[test]
    fn test_package_entry_with_type() {
        let entry = PackageEntry::WithType {
            name: "vim".to_string(),
            r#type: Some(PackageType::Native),
        };
        assert_eq!(entry.name(), "vim");
        assert_eq!(entry.package_type(), PackageType::Native);
    }

    #[test]
    fn test_secrets_default_empty_when_absent() {
        let config: Config =
            serde_yaml::from_str("host: myhost\n").expect("config without secrets should parse");
        assert!(
            config.secrets.is_empty(),
            "secrets must default to empty when the key is absent"
        );
    }

    #[test]
    fn test_secret_entry_full_deserialization() {
        let yaml = "\
host: myhost
sops_key_path: ~/.config/sops/age/keys.txt
secrets:
  - source: secrets/env.sops
    target: ~/.config/app/.env
    mode: \"0600\"
    name: app-env
";
        let config: Config = serde_yaml::from_str(yaml).expect("config should parse");
        assert_eq!(config.secrets.len(), 1);
        let s = &config.secrets[0];
        assert_eq!(s.source, "secrets/env.sops");
        assert_eq!(s.target, "~/.config/app/.env");
        assert_eq!(s.mode.as_deref(), Some("0600"));
        assert_eq!(s.name.as_deref(), Some("app-env"));
    }

    #[test]
    fn test_secret_entry_optional_fields_default_none() {
        let yaml = "\
host: myhost
secrets:
  - source: secrets/token.sops
    target: ~/.netrc
";
        let config: Config = serde_yaml::from_str(yaml).expect("config should parse");
        let s = &config.secrets[0];
        assert_eq!(s.mode, None, "mode is optional");
        assert_eq!(s.name, None, "name is optional");
    }

    // --- Config::merge characterization tests ---------------------------------
    // These lock the *current* import-merge semantics (main file wins for
    // scalars; collections are deduplicated except packages) so the merge
    // function can be refactored without changing behavior.

    /// Build a Config from YAML. Only `host` is required; every other field
    /// has a serde default, so this exercises the real deserialization path.
    fn cfg(yaml: &str) -> Config {
        serde_yaml::from_str(yaml).expect("test config should be valid YAML")
    }

    #[test]
    fn test_merge_dedups_enabled_modules_keeping_main_order_first() {
        let mut main = cfg("host: main\nenabled_modules: [x, y]");
        main.merge(cfg("host: other\nenabled_modules: [y, z]"));
        assert_eq!(main.enabled_modules, vec!["x", "y", "z"]);
    }

    #[test]
    fn test_merge_concatenates_packages_allowing_duplicates() {
        // Packages are intentionally NOT deduplicated — the package manager
        // collapses duplicates at install time.
        let mut main = cfg("host: main\npackages: [vim]");
        main.merge(cfg("host: other\npackages: [vim, git]"));
        let names: Vec<String> = main.packages.iter().map(|p| p.name().to_string()).collect();
        assert_eq!(names, vec!["vim", "vim", "git"]);
    }

    #[test]
    fn test_merge_dedups_excludes() {
        let mut main = cfg("host: main\nexclude: [a, b]");
        main.merge(cfg("host: other\nexclude: [b, c]"));
        assert_eq!(main.exclude, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_merge_dedups_enabled_and_disabled_services() {
        let mut main = cfg("host: main\nservices:\n  enabled: [s1]\n  disabled: [d1]");
        main.merge(cfg(
            "host: other\nservices:\n  enabled: [s1, s2]\n  disabled: [d1, d2]",
        ));
        assert_eq!(main.services.enabled, vec!["s1", "s2"]);
        assert_eq!(main.services.disabled, vec!["d1", "d2"]);
    }

    #[test]
    fn test_merge_dedups_service_profiles() {
        let mut main = cfg("host: main\nenabled_service_profiles: [p1]");
        main.merge(cfg("host: other\nenabled_service_profiles: [p1, p2]"));
        assert_eq!(main.enabled_service_profiles, vec!["p1", "p2"]);
    }

    #[test]
    fn test_merge_description_main_wins_when_present() {
        let mut main = cfg("host: main\ndescription: from-main");
        main.merge(cfg("host: other\ndescription: from-import"));
        assert_eq!(main.description, "from-main");
    }

    #[test]
    fn test_merge_description_filled_from_import_when_main_empty() {
        let mut main = cfg("host: main");
        main.merge(cfg("host: other\ndescription: from-import"));
        assert_eq!(main.description, "from-import");
    }

    #[test]
    fn test_merge_core_scalars_always_from_main() {
        // host, flatpak_scope and auto_prune must never be changed by an import.
        let mut main = cfg("host: main\nflatpak_scope: system\nauto_prune: true");
        main.merge(cfg("host: other\nflatpak_scope: user\nauto_prune: false"));
        assert_eq!(main.host, "main");
        assert_eq!(main.flatpak_scope, FlatpakScope::System);
        assert!(main.auto_prune);
    }

    #[test]
    fn test_merge_package_manager_filled_when_main_unset() {
        let mut main = cfg("host: main");
        assert!(main.package_manager.is_none());
        main.merge(cfg("host: other\npackage_manager: pacman"));
        assert_eq!(main.package_manager, Some(PackageManagerType::Pacman));
    }

    #[test]
    fn test_merge_package_manager_main_wins_when_set() {
        let mut main = cfg("host: main\npackage_manager: pacman");
        main.merge(cfg("host: other")); // import leaves it unset
        assert_eq!(main.package_manager, Some(PackageManagerType::Pacman));
    }
}
