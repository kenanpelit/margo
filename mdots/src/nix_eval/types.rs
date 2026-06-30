use std::path::PathBuf;

use serde::Deserialize;

use crate::config::{
    Config, DotfileEntry, FlatpakScope, ModuleManifest, ModuleProcessing, PackageEntry,
    PackageType, RunHooksAsUser, ServiceScope, ServicesConfig,
};

#[derive(Debug, Clone)]
pub struct NixModule {
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
    pub metadata: Option<serde_json::Value>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub license: Option<String>,
    pub upstream_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NixValidationResult {
    pub valid: bool,
    pub errors: Vec<NixError>,
    pub warnings: Vec<String>,
}

impl NixValidationResult {
    pub fn new() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn add_error(&mut self, error: NixError) {
        self.valid = false;
        self.errors.push(error);
    }

    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

impl Default for NixValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct NixError {
    /// categorization; reserved
    #[allow(dead_code)]
    pub kind: NixErrorKind,
    pub message: String,
    pub line: Option<u32>,
    pub hint: Option<String>,
}

impl std::fmt::Display for NixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(line) = self.line {
            write!(f, "Line {}: ", line)?;
        }
        write!(f, "{}", self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, "\n  HINT: {}", hint)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum NixErrorKind {
    SyntaxError,
    EvalError,
    MissingField,
    InvalidType,
    InvalidValue,
    FileNotFound,
    NixNotInstalled,
    AccessDenied,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NixConfigRaw {
    pub host: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub import: Vec<String>,
    #[serde(default)]
    pub enabled_modules: Vec<String>,
    #[serde(default)]
    pub packages: Vec<NixPackageEntry>,
    #[serde(default)]
    pub flatpak_packages: Vec<String>,
    #[serde(default)]
    pub nix_packages: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub additional_packages: Vec<NixPackageEntry>,
    #[serde(default)]
    pub auto_prune: bool,
    #[serde(default = "default_module_processing_serde")]
    pub module_processing: String,
    #[serde(default)]
    pub strict_package_order: bool,
    #[serde(default)]
    pub services: NixServicesRaw,
    #[serde(default)]
    pub enabled_service_profiles: Vec<String>,
    #[serde(default)]
    pub flatpak_scope: String,
    #[serde(default)]
    pub update_hooks: Option<NixUpdateHooksRaw>,
    #[serde(default)]
    pub default_apps: Option<NixDefaultAppsRaw>,
    #[serde(default)]
    pub theming: Option<NixThemingRaw>,
    #[serde(default)]
    pub editor: Option<String>,
    #[serde(default)]
    pub package_manager: Option<String>,
    #[serde(default)]
    pub aur_helper: Option<String>,
    #[serde(default)]
    pub sync_sudo: bool,
    #[serde(default)]
    pub auto_commit: bool,
    #[serde(default)]
    pub nix: Option<NixNixConfigRaw>,
    #[serde(default)]
    pub config_backups: Option<NixConfigBackupsRaw>,
    #[serde(default)]
    pub system_backups: Option<NixSystemBackupsRaw>,
    /// parsed but not yet wired into to_config()
    #[allow(dead_code)]
    #[serde(default)]
    pub run_hooks_as_user: Option<serde_json::Value>,
}

fn default_module_processing_serde() -> String {
    "parallel".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NixServicesRaw {
    #[serde(default)]
    pub enabled: Vec<String>,
    #[serde(default)]
    pub disabled: Vec<String>,
    #[serde(default)]
    pub scope: String,
}

impl Default for NixServicesRaw {
    fn default() -> Self {
        Self {
            enabled: Vec::new(),
            disabled: Vec::new(),
            scope: "system".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NixUpdateHooksRaw {
    #[serde(default)]
    pub pre_update: Option<String>,
    #[serde(default)]
    pub post_update: Option<String>,
    #[serde(default = "default_hook_behavior_serde")]
    pub behavior: String,
    #[serde(default)]
    pub devel: bool,
    #[serde(default)]
    pub run_as_user: bool,
}

fn default_hook_behavior_serde() -> String {
    "ask".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NixDefaultAppsRaw {
    #[serde(default)]
    pub scope: Option<String>,
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
    #[serde(default)]
    pub mime_types: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NixThemingRaw {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub cursor: Option<NixCursorRaw>,
    #[serde(default)]
    pub icons: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub dark_or_light: Option<String>,
    #[serde(default)]
    pub font: Option<NixFontRaw>,
    #[serde(default)]
    pub gtk: Option<NixGtkThemingRaw>,
    #[serde(default)]
    pub qt: Option<NixQtThemingRaw>,
    #[serde(default)]
    pub env_vars: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NixCursorRaw {
    pub theme: String,
    #[serde(default)]
    pub size: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NixFontRaw {
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub size: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NixGtkThemingRaw {
    #[serde(default)]
    pub decorations: Option<bool>,
    #[serde(default)]
    pub primary_button: Option<String>,
    #[serde(default)]
    pub enable_animations: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NixQtThemingRaw {
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub icon_theme: Option<String>,
    #[serde(default)]
    pub font: Option<NixFontRaw>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NixNixConfigRaw {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub home_manager_enabled: bool,
    #[serde(default)]
    pub flake_enabled: bool,
    #[serde(default)]
    pub nixpkgs_channel: Option<String>,
    #[serde(default)]
    pub home_manager_channel: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NixConfigBackupsRaw {
    #[serde(default = "default_true_serde")]
    pub enabled: bool,
    #[serde(default = "default_max_backups_serde")]
    pub max_backups: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NixSystemBackupsRaw {
    #[serde(default = "default_true_serde")]
    pub enabled: bool,
    #[serde(default = "default_true_serde")]
    pub backup_on_sync: bool,
    #[serde(default = "default_true_serde")]
    pub backup_on_update: bool,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default = "default_snapper_config_serde")]
    pub snapper_config: String,
    #[serde(default = "default_max_backups_serde")]
    pub max_backups: u32,
}

fn default_true_serde() -> bool {
    true
}

fn default_max_backups_serde() -> u32 {
    5
}

fn default_snapper_config_serde() -> String {
    "root".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NixModuleRaw {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub packages: Vec<NixPackageEntry>,
    #[serde(default)]
    pub flatpak_packages: Vec<String>,
    #[serde(default)]
    pub nix_packages: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub services: Option<NixServicesRaw>,
    #[serde(default = "default_hook_behavior_serde")]
    pub hook_behavior: String,
    #[serde(default)]
    pub pre_install_hook: Option<String>,
    #[serde(default)]
    pub post_install_hook: Option<String>,
    #[serde(default)]
    pub pre_hook_behavior: Option<String>,
    #[serde(default)]
    pub post_hook_behavior: Option<String>,
    #[serde(default)]
    pub post_disable_hook: Option<String>,
    #[serde(default)]
    pub post_disable_behavior: Option<String>,
    #[serde(default)]
    pub run_hooks_as_user: Option<serde_json::Value>,
    #[serde(default)]
    pub dotfiles: Option<Vec<NixDotfileEntry>>,
    #[serde(default)]
    pub dotfiles_sync: Option<bool>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub upstream_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NixDotfileEntry {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum NixPackageEntry {
    Simple(String),
    WithType {
        name: String,
        #[serde(default)]
        r#type: Option<String>,
    },
}

impl NixPackageEntry {
    pub fn to_package_entry(&self) -> PackageEntry {
        match self {
            NixPackageEntry::Simple(name) => {
                if let Some(stripped) = name.strip_prefix("flatpak:") {
                    PackageEntry::WithType {
                        name: stripped.to_string(),
                        r#type: Some(PackageType::Flatpak),
                    }
                } else if let Some(stripped) = name.strip_prefix("nix:") {
                    PackageEntry::WithType {
                        name: stripped.to_string(),
                        r#type: Some(PackageType::Nix),
                    }
                } else {
                    PackageEntry::Simple(name.clone())
                }
            }
            NixPackageEntry::WithType { name, r#type } => {
                let pkg_type = match r#type.as_deref() {
                    Some("flatpak") => Some(PackageType::Flatpak),
                    Some("nix") => Some(PackageType::Nix),
                    _ => None,
                };
                PackageEntry::WithType {
                    name: name.clone(),
                    r#type: pkg_type,
                }
            }
        }
    }
}

fn parse_run_hooks_as_user(value: &Option<serde_json::Value>) -> RunHooksAsUser {
    match value {
        None => RunHooksAsUser::Bool(false),
        Some(serde_json::Value::Bool(b)) => RunHooksAsUser::Bool(*b),
        Some(serde_json::Value::String(s)) => RunHooksAsUser::Username(s.clone()),
        Some(serde_json::Value::Null) => RunHooksAsUser::Bool(false),
        _ => RunHooksAsUser::Bool(false),
    }
}

impl NixConfigRaw {
    #[allow(deprecated)]
    // Consuming conversion: takes `self` by value intentionally.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_config(self) -> Config {
        let mut packages: Vec<PackageEntry> =
            self.packages.iter().map(|e| e.to_package_entry()).collect();

        for fp in &self.flatpak_packages {
            packages.push(PackageEntry::WithType {
                name: fp.clone(),
                r#type: Some(PackageType::Flatpak),
            });
        }

        for np in &self.nix_packages {
            packages.push(PackageEntry::WithType {
                name: np.clone(),
                r#type: Some(PackageType::Nix),
            });
        }

        let module_processing = match self.module_processing.as_str() {
            "sequential" => ModuleProcessing::Sequential,
            _ => ModuleProcessing::Parallel,
        };

        let flatpak_scope = match self.flatpak_scope.as_str() {
            "system" => FlatpakScope::System,
            _ => FlatpakScope::User,
        };

        let services = ServicesConfig {
            enabled: self.services.enabled,
            disabled: self.services.disabled,
            scope: match self.services.scope.as_str() {
                "user" => ServiceScope::User,
                _ => ServiceScope::System,
            },
        };

        let update_hooks = if let Some(h) = self.update_hooks {
            crate::config::UpdateHooksConfig {
                pre_update: h.pre_update,
                post_update: h.post_update,
                behavior: h.behavior,
                devel: h.devel,
                run_as_user: h.run_as_user,
            }
        } else {
            crate::config::UpdateHooksConfig::default()
        };

        let default_apps = if let Some(a) = self.default_apps {
            crate::config::DefaultAppsConfig {
                scope: match a.scope.as_deref() {
                    Some("user") => crate::config::DefaultsScope::User,
                    _ => crate::config::DefaultsScope::System,
                },
                browser: a.browser,
                text_editor: a.text_editor,
                file_manager: a.file_manager,
                terminal: a.terminal,
                video_player: a.video_player,
                audio_player: a.audio_player,
                image_viewer: a.image_viewer,
                pdf_viewer: a.pdf_viewer,
                mime_types: a.mime_types.unwrap_or_default(),
            }
        } else {
            crate::config::DefaultAppsConfig::default()
        };

        let theming = if let Some(t) = self.theming {
            crate::config::ThemingConfig {
                scope: match t.scope.as_deref() {
                    Some("system") => crate::config::ThemingScope::System,
                    _ => crate::config::ThemingScope::User,
                },
                cursor: t.cursor.map(|c| crate::config::CursorConfig {
                    theme: c.theme,
                    size: c.size,
                }),
                icons: t.icons,
                theme: t.theme,
                dark_or_light: t.dark_or_light,
                font: t.font.map(|f| crate::config::FontConfig {
                    family: f.family,
                    size: f.size,
                }),
                gtk: t.gtk.map(|g| crate::config::GtkThemingConfig {
                    decorations: g.decorations,
                    primary_button: g.primary_button,
                    enable_animations: g.enable_animations,
                }),
                qt: t.qt.map(|q| crate::config::QtThemingConfig {
                    backend: match q.backend.as_deref() {
                        Some("kde") => crate::config::QtBackend::Kde,
                        Some("qt5ct") => crate::config::QtBackend::Qt5ct,
                        _ => crate::config::QtBackend::Auto,
                    },
                    style: q.style,
                    icon_theme: q.icon_theme,
                    font: q.font.map(|f| crate::config::FontConfig {
                        family: f.family,
                        size: f.size,
                    }),
                }),
                env_vars: t.env_vars.unwrap_or_default(),
            }
        } else {
            crate::config::ThemingConfig::default()
        };

        let config_backups = if let Some(b) = self.config_backups {
            crate::config::ConfigBackupsSettings {
                enabled: b.enabled,
                max_backups: b.max_backups,
            }
        } else {
            crate::config::ConfigBackupsSettings::default()
        };

        let system_backups = if let Some(b) = self.system_backups {
            crate::config::SystemBackupsSettings {
                enabled: b.enabled,
                backup_on_sync: b.backup_on_sync,
                backup_on_update: b.backup_on_update,
                tool: b.tool,
                snapper_config: b.snapper_config,
                max_backups: b.max_backups,
            }
        } else {
            crate::config::SystemBackupsSettings::default()
        };

        let nix = if let Some(n) = self.nix {
            crate::config::NixConfig {
                enabled: n.enabled,
                home_manager_enabled: n.home_manager_enabled,
                flake_enabled: n.flake_enabled,
                nixpkgs_channel: n
                    .nixpkgs_channel
                    .unwrap_or_else(|| "nixpkgs-unstable".to_string()),
                home_manager_channel: n
                    .home_manager_channel
                    .unwrap_or_else(|| "release-25.05".to_string()),
            }
        } else {
            crate::config::NixConfig::default()
        };

        let package_manager = self.package_manager.and_then(|s| match s.as_str() {
            "pacman" => Some(crate::config::PackageManagerType::Pacman),
            _ => None,
        });

        let additional_packages: Vec<PackageEntry> = self
            .additional_packages
            .iter()
            .map(|e| e.to_package_entry())
            .collect();

        Config {
            host: self.host,
            sops_key_path: None,
            secrets: Vec::new(),
            description: self.description,
            import: self.import,
            enabled_modules: self.enabled_modules,
            packages,
            exclude: self.exclude,
            additional_packages,
            backup_tool: None,
            snapper_config: system_backups.snapper_config.clone(),
            flatpak_scope,
            auto_prune: self.auto_prune,
            module_processing,
            strict_package_order: self.strict_package_order,
            config_backups,
            system_backups,
            services,
            enabled_service_profiles: self.enabled_service_profiles,
            update_hooks,
            default_apps,
            theming,
            editor: self.editor,
            package_manager,
            aur_helper: self.aur_helper,
            sync_sudo: self.sync_sudo,
            auto_commit: self.auto_commit,
            nix,
        }
    }
}

impl NixModuleRaw {
    // Consuming conversion: takes `self` by value intentionally.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_nix_module(self, path: PathBuf) -> NixModule {
        let mut packages: Vec<PackageEntry> =
            self.packages.iter().map(|e| e.to_package_entry()).collect();

        for fp in &self.flatpak_packages {
            packages.push(PackageEntry::WithType {
                name: fp.clone(),
                r#type: Some(PackageType::Flatpak),
            });
        }

        for np in &self.nix_packages {
            packages.push(PackageEntry::WithType {
                name: np.clone(),
                r#type: Some(PackageType::Nix),
            });
        }

        let services = if let Some(s) = self.services {
            ServicesConfig {
                enabled: s.enabled,
                disabled: s.disabled,
                scope: match s.scope.as_str() {
                    "user" => ServiceScope::User,
                    _ => ServiceScope::System,
                },
            }
        } else {
            ServicesConfig::default()
        };

        let run_hooks_as_user = parse_run_hooks_as_user(&self.run_hooks_as_user);

        NixModule {
            path,
            description: self.description,
            packages,
            services,
            conflicts: self.conflicts,
            pre_install_hook: self.pre_install_hook,
            post_install_hook: self.post_install_hook,
            hook_behavior: self.hook_behavior,
            pre_hook_behavior: self.pre_hook_behavior,
            post_hook_behavior: self.post_hook_behavior,
            post_disable_hook: self.post_disable_hook,
            post_disable_behavior: self.post_disable_behavior,
            run_hooks_as_user,
            metadata: None,
            author: self.author,
            version: self.version,
            category: self.category,
            tags: self.tags,
            license: self.license,
            upstream_url: self.upstream_url,
        }
    }

    // Consuming conversion: takes `self` by value intentionally.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_module_manifest(self) -> ModuleManifest {
        let run_hooks_as_user = parse_run_hooks_as_user(&self.run_hooks_as_user);

        ModuleManifest {
            description: self.description,
            conflicts: self.conflicts,
            pre_install_hook: self.pre_install_hook,
            post_install_hook: self.post_install_hook,
            hook_behavior: self.hook_behavior,
            pre_hook_behavior: self.pre_hook_behavior,
            post_hook_behavior: self.post_hook_behavior,
            run_hooks_as_user,
            post_disable_hook: self.post_disable_hook,
            post_disable_behavior: self.post_disable_behavior,
            package_files: Vec::new(),
            dotfiles_sync: self.dotfiles_sync,
            dotfiles: self
                .dotfiles
                .unwrap_or_default()
                .into_iter()
                .map(|d| DotfileEntry {
                    source: d.source,
                    target: d.target,
                })
                .collect(),
            author: self.author,
            version: self.version,
            category: self.category,
            tags: self.tags,
            license: self.license,
            upstream_url: self.upstream_url,
        }
    }
}
