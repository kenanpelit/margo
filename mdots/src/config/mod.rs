use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// The `Config` root lives here; its sections, package lists, modules,
// paths, loading and tool resolution live in config/*.rs.
mod load;
mod module;
mod package;
mod paths;
mod resolve;
mod settings;
use load::is_pointer_config_raw;
pub use load::load_config;
use module::default_hook_behavior;
pub use module::{
    DotfileEntry, DynamicModule, ModuleManifest, ModuleStructure, SecretEntry, load_module,
    validate_module,
};
pub use package::{
    PackageEntry, PackageList, PackageManagerType, PackageType, load_package_list,
    load_package_list_any, write_package_list_any,
};

pub use paths::{
    ConfigPaths, declared_packages_paths, is_lua_config, is_nix_config, resolve_config_path,
};
pub use resolve::{
    detect_package_manager_type, resolve_aur_helper, resolve_editor, resolve_package_manager,
};
pub use settings::{
    ConfigBackupsSettings, CursorConfig, DefaultAppsConfig, DefaultsScope, FlatpakScope,
    FontConfig, GtkThemingConfig, ModuleProcessing, QtBackend, QtThemingConfig, RunHooksAsUser,
    ServiceProfile, ServiceScope, ServicesConfig, SystemBackupsSettings, ThemingConfig,
    ThemingScope, UpdateHooksConfig, resolve_flatpak_remote,
};
use settings::{default_module_processing, default_strict_package_order};

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

    /// Flatpak remote apps are installed from, unless a package sets its own `remote:` (default: flathub)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flatpak_remote: Option<String>,

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

        // Note: host, snapper_config, flatpak_scope, flatpak_remote, auto_prune from main file always win
        // This prevents imports from accidentally changing core settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatpak_remote_is_read_from_the_host_and_kept_over_imports() {
        let mut main = cfg("host: main\nflatpak_remote: fedora");
        assert_eq!(main.flatpak_remote.as_deref(), Some("fedora"));
        main.merge(cfg("host: other\nflatpak_remote: elsewhere"));
        assert_eq!(main.flatpak_remote.as_deref(), Some("fedora"));
        assert_eq!(cfg("host: x").flatpak_remote, None);
    }

    #[test]
    fn test_secrets_default_empty_when_absent() {
        let config: Config =
            yaml_serde::from_str("host: myhost\n").expect("config without secrets should parse");
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
        let config: Config = yaml_serde::from_str(yaml).expect("config should parse");
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
        let config: Config = yaml_serde::from_str(yaml).expect("config should parse");
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
        yaml_serde::from_str(yaml).expect("test config should be valid YAML")
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
