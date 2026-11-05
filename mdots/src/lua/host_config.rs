//! Loading a Lua host config (`config.lua`) into a `Config`.

use super::*;

/// Load a Lua config/host file and return a Config struct
/// This allows users to write their entire configuration in Lua
pub fn load_lua_config(path: &Path) -> Result<Config> {
    let lua = create_config_lua_env()?;

    // Load and execute the Lua file
    let script =
        std::fs::read_to_string(path).context(format!("Failed to read Lua config: {:?}", path))?;

    let result: Table = lua
        .load(&script)
        .set_name(path.to_string_lossy())
        .eval()
        .map_err(|e| anyhow!("Failed to execute Lua config {:?}: {}", path, e))?;

    // Extract config from returned table
    extract_config(&result)
}

/// Detect whether a Lua config file is a pointer (only contains a host field).
pub fn detect_pointer_lua_config(path: &Path) -> Result<Option<String>> {
    let lua = create_config_lua_env_silent()?;

    let script =
        std::fs::read_to_string(path).context(format!("Failed to read Lua config: {:?}", path))?;

    let result: Table = lua
        .load(&script)
        .set_name(path.to_string_lossy())
        .eval()
        .map_err(|e| anyhow!("Failed to execute Lua config {:?}: {}", path, e))?;

    let mut host: Option<String> = None;
    for pair in result.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| anyhow!("Lua error: {}", e))?;
        match key {
            Value::String(key_str) => {
                let key_str = key_str.to_str().map_err(|e| anyhow!("Lua error: {}", e))?;
                if key_str == "host" {
                    if let Value::String(host_str) = value {
                        host = Some(
                            host_str
                                .to_str()
                                .map_err(|e| anyhow!("Lua error: {}", e))?
                                .to_string(),
                        );
                    } else {
                        return Ok(None);
                    }
                } else {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        }
    }

    Ok(host)
}

pub(super) fn create_config_lua_env() -> Result<Lua> {
    let lua = create_sandboxed_lua()?;

    // Register mdots helpers
    helpers::register_helpers(&lua)?;
    hardware::register_hardware_helpers(&lua)?;
    package::register_package_helpers(&lua)?;
    service::register_service_helpers(&lua)?;
    power::register_power_helpers(&lua)?;
    security::register_security_helpers(&lua)?;
    desktop::register_desktop_helpers(&lua)?;
    boot::register_boot_helpers(&lua)?;
    network::register_network_helpers(&lua)?;
    audio::register_audio_helpers(&lua)?;
    storage::register_storage_helpers(&lua)?;

    Ok(lua)
}

/// Create a Lua environment for config detection that suppresses log output
/// to avoid duplicate log messages when the config is loaded later
pub(super) fn create_config_lua_env_silent() -> Result<Lua> {
    let lua = create_sandboxed_lua()?;

    // Register mdots helpers (excluding log helpers)
    helpers::register_helpers_silent(&lua)?;
    hardware::register_hardware_helpers(&lua)?;
    package::register_package_helpers(&lua)?;
    service::register_service_helpers(&lua)?;
    power::register_power_helpers(&lua)?;
    security::register_security_helpers(&lua)?;
    desktop::register_desktop_helpers(&lua)?;
    boot::register_boot_helpers(&lua)?;
    network::register_network_helpers(&lua)?;
    audio::register_audio_helpers(&lua)?;
    storage::register_storage_helpers(&lua)?;

    Ok(lua)
}

/// Extract a full Config struct from a Lua table
pub(super) fn extract_config(table: &Table) -> Result<Config> {
    // Required: host
    let host: String = table
        .get("host")
        .map_err(|_| anyhow!("Lua config must have a 'host' field"))?;

    // Optional fields with defaults
    let description: String = table.get("description").unwrap_or_default();
    let import: Vec<String> = table.get("import").unwrap_or_default();
    let enabled_modules: Vec<String> = table.get("enabled_modules").unwrap_or_default();
    let exclude: Vec<String> = table.get("exclude").unwrap_or_default();

    // Extract packages (optional, defaults to empty)
    let packages = extract_packages_optional(table)?;

    // Additional packages (backwards compat)
    let additional_packages = extract_additional_packages(table)?;

    // Flatpak scope
    let flatpak_scope = extract_flatpak_scope(table)?;

    // Auto prune
    let auto_prune: bool = table.get("auto_prune").unwrap_or(false);

    // Module processing mode
    let module_processing = extract_module_processing(table)?;

    // Strict package order
    let strict_package_order: bool = table.get("strict_package_order").unwrap_or(false);

    // Config backups
    let config_backups = extract_config_backups(table)?;

    // System backups
    let system_backups = extract_system_backups(table)?;

    // Services
    let services = extract_services(table)?;

    // Enabled service profiles
    let enabled_service_profiles: Vec<String> =
        table.get("enabled_service_profiles").unwrap_or_default();

    // Update hooks
    let update_hooks = extract_update_hooks(table)?;

    // Default apps
    let default_apps = extract_default_apps(table)?;

    // Theming
    let theming = extract_theming(table)?;

    // Editor
    let editor: Option<String> = table.get("editor").ok();

    // AUR helper
    let aur_helper: Option<String> = table.get("aur_helper").ok();

    // Package manager (auto-detect if not specified)
    let package_manager_str: Option<String> = table.get("package_manager").ok();
    let package_manager: Option<crate::config::PackageManagerType> =
        package_manager_str.and_then(|s| match s.as_str() {
            "pacman" => Some(crate::config::PackageManagerType::Pacman),
            _ => None,
        });

    // Sync sudo
    let sync_sudo: bool = table.get("sync_sudo").unwrap_or(false);

    // Auto commit
    let auto_commit: bool = table.get("auto_commit").unwrap_or(false);

    // Nix configuration
    let nix: crate::config::NixConfig = if let Ok(nix_table) = table.get::<mlua::Table>("nix") {
        let enabled: bool = nix_table.get("enabled").unwrap_or(false);
        let home_manager_enabled: bool = nix_table.get("home_manager_enabled").unwrap_or(false);
        let flake_enabled: bool = nix_table.get("flake_enabled").unwrap_or(false);
        let nixpkgs_channel: String = nix_table
            .get("nixpkgs_channel")
            .unwrap_or_else(|_| "nixpkgs-unstable".to_string());
        let home_manager_channel: String = nix_table
            .get("home_manager_channel")
            .unwrap_or_else(|_| "release-25.05".to_string());
        crate::config::NixConfig {
            enabled,
            home_manager_enabled,
            flake_enabled,
            nixpkgs_channel,
            home_manager_channel,
        }
    } else {
        crate::config::NixConfig::default()
    };

    // Handle deprecated fields
    #[allow(deprecated)]
    let backup_tool: Option<String> = table.get("backup_tool").ok();
    #[allow(deprecated)]
    let snapper_config: String = table
        .get("snapper_config")
        .unwrap_or_else(|_| "root".to_string());

    #[allow(deprecated)]
    Ok(Config {
        host,
        sops_key_path: None,
        secrets: Vec::new(),
        description,
        import,
        enabled_modules,
        packages,
        exclude,
        additional_packages,
        backup_tool,
        snapper_config,
        flatpak_scope,
        flatpak_remote: table.get::<Option<String>>("flatpak_remote").ok().flatten(),
        auto_prune,
        module_processing,
        strict_package_order,
        config_backups,
        system_backups,
        services,
        enabled_service_profiles,
        update_hooks,
        default_apps,
        theming,
        editor,
        package_manager,
        aur_helper,
        sync_sudo,
        auto_commit,
        nix,
    })
}

/// Extract packages from Lua table (optional, for config files)
pub(super) fn extract_packages_optional(table: &Table) -> Result<Vec<PackageEntry>> {
    let packages_value: Value = match table.get("packages") {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };

    let packages_table = match packages_value {
        Value::Table(t) => t,
        Value::Nil => return Ok(Vec::new()),
        _ => anyhow::bail!("'packages' must be a table/array"),
    };

    let mut packages = Vec::new();

    for pair in packages_table.pairs::<i64, Value>() {
        let (_, value) = pair.map_err(|e| anyhow!("Lua error: {}", e))?;
        match value {
            Value::String(s) => {
                let pkg_str = s
                    .to_str()
                    .map_err(|e| anyhow!("Lua error: {}", e))?
                    .to_string();
                if let Some(name) = pkg_str.strip_prefix("flatpak:") {
                    packages.push(PackageEntry::WithType {
                        name: name.to_string(),
                        r#type: Some(PackageType::Flatpak),
                        remote: None,
                    });
                } else if let Some(name) = pkg_str.strip_prefix("nix:") {
                    packages.push(PackageEntry::WithType {
                        name: name.to_string(),
                        r#type: Some(PackageType::Nix),
                        remote: None,
                    });
                } else {
                    packages.push(PackageEntry::Simple(pkg_str));
                }
            }
            Value::Table(t) => {
                let name: String = t.get("name").map_err(|e| anyhow!("Lua error: {}", e))?;
                let pkg_type: Option<String> = t.get("type").ok();

                if let Some(pt) = pkg_type {
                    packages.push(PackageEntry::WithType {
                        name,
                        r#type: Some(match pt.as_str() {
                            "flatpak" => PackageType::Flatpak,
                            "nix" => PackageType::Nix,
                            _ => PackageType::Native,
                        }),
                        remote: None,
                    });
                } else {
                    packages.push(PackageEntry::Simple(name));
                }
            }
            _ => anyhow::bail!("Package entry must be a string or table"),
        }
    }

    Ok(packages)
}

/// Extract additional_packages from Lua table (backwards compat)
pub(super) fn extract_additional_packages(table: &Table) -> Result<Vec<PackageEntry>> {
    let packages_value: Value = match table.get("additional_packages") {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };

    let packages_table = match packages_value {
        Value::Table(t) => t,
        Value::Nil => return Ok(Vec::new()),
        _ => anyhow::bail!("'additional_packages' must be a table/array"),
    };

    let mut packages = Vec::new();

    for pair in packages_table.pairs::<i64, Value>() {
        let (_, value) = pair.map_err(|e| anyhow!("Lua error: {}", e))?;
        match value {
            Value::String(s) => {
                let pkg_str = s
                    .to_str()
                    .map_err(|e| anyhow!("Lua error: {}", e))?
                    .to_string();
                packages.push(PackageEntry::Simple(pkg_str));
            }
            Value::Table(t) => {
                let name: String = t.get("name").map_err(|e| anyhow!("Lua error: {}", e))?;
                let pkg_type: Option<String> = t.get("type").ok();

                if let Some(pt) = pkg_type {
                    packages.push(PackageEntry::WithType {
                        name,
                        r#type: Some(match pt.as_str() {
                            "flatpak" => PackageType::Flatpak,
                            _ => PackageType::Native,
                        }),
                        remote: None,
                    });
                } else {
                    packages.push(PackageEntry::Simple(name));
                }
            }
            _ => anyhow::bail!("Package entry must be a string or table"),
        }
    }

    Ok(packages)
}

/// Extract flatpak_scope from Lua table
pub(super) fn extract_flatpak_scope(table: &Table) -> Result<FlatpakScope> {
    let scope: Option<String> = table.get("flatpak_scope").ok();

    match scope.as_deref() {
        Some("system") => Ok(FlatpakScope::System),
        Some("user") | None => Ok(FlatpakScope::User),
        Some(other) => anyhow::bail!(
            "Invalid flatpak_scope: '{}'. Must be 'user' or 'system'",
            other
        ),
    }
}

/// Extract module_processing from Lua table
pub(super) fn extract_module_processing(table: &Table) -> Result<ModuleProcessing> {
    let mode: Option<String> = table.get("module_processing").ok();

    match mode.as_deref() {
        Some("sequential") => Ok(ModuleProcessing::Sequential),
        Some("parallel") | None => Ok(ModuleProcessing::Parallel),
        Some(other) => anyhow::bail!(
            "Invalid module_processing: '{}'. Must be 'parallel' or 'sequential'",
            other
        ),
    }
}

/// Extract config_backups settings from Lua table
pub(super) fn extract_config_backups(table: &Table) -> Result<ConfigBackupsSettings> {
    let backups_value: Value = match table.get("config_backups") {
        Ok(v) => v,
        Err(_) => return Ok(ConfigBackupsSettings::default()),
    };

    match backups_value {
        Value::Nil => Ok(ConfigBackupsSettings::default()),
        Value::Table(t) => {
            let enabled: bool = t.get("enabled").unwrap_or(true);
            let max_backups: u32 = t.get("max_backups").unwrap_or(5);

            Ok(ConfigBackupsSettings {
                enabled,
                max_backups,
            })
        }
        _ => anyhow::bail!("'config_backups' must be a table"),
    }
}

/// Extract system_backups settings from Lua table
pub(super) fn extract_system_backups(table: &Table) -> Result<SystemBackupsSettings> {
    let backups_value: Value = match table.get("system_backups") {
        Ok(v) => v,
        Err(_) => return Ok(SystemBackupsSettings::default()),
    };

    match backups_value {
        Value::Nil => Ok(SystemBackupsSettings::default()),
        Value::Table(t) => {
            let enabled: bool = t.get("enabled").unwrap_or(true);
            let backup_on_sync: bool = t.get("backup_on_sync").unwrap_or(true);
            let backup_on_update: bool = t.get("backup_on_update").unwrap_or(true);
            let tool: Option<String> = t.get("tool").ok();
            let snapper_config: String = t
                .get("snapper_config")
                .unwrap_or_else(|_| "root".to_string());
            let max_backups: u32 = t.get("max_backups").unwrap_or(5);

            Ok(SystemBackupsSettings {
                enabled,
                backup_on_sync,
                backup_on_update,
                tool,
                snapper_config,
                max_backups,
            })
        }
        _ => anyhow::bail!("'system_backups' must be a table"),
    }
}

/// Extract update_hooks settings from Lua table
pub(super) fn extract_update_hooks(table: &Table) -> Result<UpdateHooksConfig> {
    let hooks_value: Value = match table.get("update_hooks") {
        Ok(v) => v,
        Err(_) => return Ok(UpdateHooksConfig::default()),
    };

    match hooks_value {
        Value::Nil => Ok(UpdateHooksConfig::default()),
        Value::Table(t) => {
            let pre_update: Option<String> = t.get("pre_update").ok();
            let post_update: Option<String> = t.get("post_update").ok();
            let behavior: String = t.get("behavior").unwrap_or_else(|_| "ask".to_string());
            let devel: bool = t.get("devel").unwrap_or(false);
            let run_as_user: bool = t.get("run_as_user").unwrap_or(false);

            Ok(UpdateHooksConfig {
                pre_update,
                post_update,
                behavior,
                devel,
                run_as_user,
            })
        }
        _ => anyhow::bail!("'update_hooks' must be a table"),
    }
}

/// Extract default_apps settings from Lua table
pub(super) fn extract_default_apps(table: &Table) -> Result<DefaultAppsConfig> {
    let apps_value: Value = match table.get("default_apps") {
        Ok(v) => v,
        Err(_) => return Ok(DefaultAppsConfig::default()),
    };

    match apps_value {
        Value::Nil => Ok(DefaultAppsConfig::default()),
        Value::Table(t) => {
            // Scope
            let scope_str: Option<String> = t.get("scope").ok();
            let scope = match scope_str.as_deref() {
                Some("user") => DefaultsScope::User,
                Some("system") | None => DefaultsScope::System,
                Some(other) => anyhow::bail!(
                    "Invalid default_apps.scope: '{}'. Must be 'user' or 'system'",
                    other
                ),
            };

            // App settings
            let browser: Option<String> = t.get("browser").ok();
            let text_editor: Option<String> = t.get("text_editor").ok();
            let file_manager: Option<String> = t.get("file_manager").ok();
            let terminal: Option<String> = t.get("terminal").ok();
            let video_player: Option<String> = t.get("video_player").ok();
            let audio_player: Option<String> = t.get("audio_player").ok();
            let image_viewer: Option<String> = t.get("image_viewer").ok();
            let pdf_viewer: Option<String> = t.get("pdf_viewer").ok();

            // MIME types
            let mime_types = extract_mime_types(&t)?;

            Ok(DefaultAppsConfig {
                scope,
                browser,
                text_editor,
                file_manager,
                terminal,
                video_player,
                audio_player,
                image_viewer,
                pdf_viewer,
                mime_types,
            })
        }
        _ => anyhow::bail!("'default_apps' must be a table"),
    }
}

/// Extract theming configuration from Lua table
pub(super) fn extract_theming(table: &Table) -> Result<crate::config::ThemingConfig> {
    use crate::config::{
        CursorConfig, FontConfig, GtkThemingConfig, QtBackend, QtThemingConfig, ThemingScope,
    };

    let theming_value: Value = match table.get("theming") {
        Ok(v) => v,
        Err(_) => return Ok(crate::config::ThemingConfig::default()),
    };

    match theming_value {
        Value::Nil => Ok(crate::config::ThemingConfig::default()),
        Value::Table(t) => {
            // Scope
            let scope_str: Option<String> = t.get("scope").ok();
            let scope = match scope_str.as_deref() {
                Some("system") => ThemingScope::System,
                Some("user") | None => ThemingScope::User,
                Some(other) => anyhow::bail!(
                    "Invalid theming.scope: '{}'. Must be 'user' or 'system'",
                    other
                ),
            };

            // Cursor config
            let cursor: Option<CursorConfig> = match t.get("cursor") {
                Ok(Value::Table(cursor_table)) => {
                    let theme: String = cursor_table.get("theme").map_err(|_| {
                        anyhow!("theming.cursor.theme is required when cursor is specified")
                    })?;
                    let size: Option<u32> = cursor_table.get("size").ok();
                    Some(CursorConfig { theme, size })
                }
                _ => None,
            };

            // Icons
            let icons: Option<String> = t.get("icons").ok();

            // Theme
            let theme: Option<String> = t.get("theme").ok();

            // Dark or light
            let dark_or_light: Option<String> = t.get("dark_or_light").ok();

            // Font config
            let font: Option<FontConfig> = match t.get("font") {
                Ok(Value::Table(font_table)) => {
                    let family: Option<String> = font_table.get("family").ok();
                    let size: Option<f32> = font_table.get("size").ok();
                    Some(FontConfig { family, size })
                }
                _ => None,
            };

            // GTK config
            let gtk: Option<GtkThemingConfig> = match t.get("gtk") {
                Ok(Value::Table(gtk_table)) => {
                    let decorations: Option<bool> = gtk_table.get("decorations").ok();
                    let primary_button: Option<String> = gtk_table.get("primary_button").ok();
                    let enable_animations: Option<bool> = gtk_table.get("enable_animations").ok();
                    Some(GtkThemingConfig {
                        decorations,
                        primary_button,
                        enable_animations,
                    })
                }
                _ => None,
            };

            // Qt config
            let qt: Option<QtThemingConfig> = match t.get("qt") {
                Ok(Value::Table(qt_table)) => {
                    let backend_str: Option<String> = qt_table.get("backend").ok();
                    let backend = match backend_str.as_deref() {
                        Some("kde") => QtBackend::Kde,
                        Some("qt5ct") => QtBackend::Qt5ct,
                        Some("auto") | None => QtBackend::Auto,
                        Some(other) => anyhow::bail!(
                            "Invalid theming.qt.backend: '{}'. Must be 'auto', 'kde', or 'qt5ct'",
                            other
                        ),
                    };
                    let style: Option<String> = qt_table.get("style").ok();
                    let icon_theme: Option<String> = qt_table.get("icon_theme").ok();
                    let font: Option<FontConfig> = match qt_table.get("font") {
                        Ok(Value::Table(font_table)) => {
                            let family: Option<String> = font_table.get("family").ok();
                            let size: Option<f32> = font_table.get("size").ok();
                            Some(FontConfig { family, size })
                        }
                        _ => None,
                    };
                    Some(QtThemingConfig {
                        backend,
                        style,
                        icon_theme,
                        font,
                    })
                }
                _ => None,
            };

            // Environment variables
            let env_vars = extract_env_vars(&t)?;

            Ok(crate::config::ThemingConfig {
                scope,
                cursor,
                icons,
                theme,
                dark_or_light,
                font,
                gtk,
                qt,
                env_vars,
            })
        }
        _ => anyhow::bail!("'theming' must be a table"),
    }
}

/// Extract environment variables HashMap from Lua table
pub(super) fn extract_env_vars(table: &Table) -> Result<std::collections::HashMap<String, String>> {
    let env_value: Value = match table.get("env_vars") {
        Ok(v) => v,
        Err(_) => return Ok(std::collections::HashMap::new()),
    };

    match env_value {
        Value::Nil => Ok(std::collections::HashMap::new()),
        Value::Table(t) => {
            let mut env_vars = std::collections::HashMap::new();

            for pair in t.pairs::<String, String>() {
                let (k, v) = pair.map_err(|e| anyhow!("Lua error: {}", e))?;
                env_vars.insert(k, v);
            }

            Ok(env_vars)
        }
        _ => anyhow::bail!("'env_vars' must be a table"),
    }
}

/// Extract mime_types HashMap from Lua table
pub(super) fn extract_mime_types(
    table: &Table,
) -> Result<std::collections::HashMap<String, String>> {
    let mime_value: Value = match table.get("mime_types") {
        Ok(v) => v,
        Err(_) => return Ok(std::collections::HashMap::new()),
    };

    match mime_value {
        Value::Nil => Ok(std::collections::HashMap::new()),
        Value::Table(t) => {
            let mut mime_types = std::collections::HashMap::new();

            for pair in t.pairs::<String, String>() {
                let (k, v) = pair.map_err(|e| anyhow!("Lua error: {}", e))?;
                mime_types.insert(k, v);
            }

            Ok(mime_types)
        }
        _ => anyhow::bail!("'mime_types' must be a table"),
    }
}
