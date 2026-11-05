//! Loading the host config: the pointer file, imports and merging.

use super::*;

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
        } else if packages_yaml.exists()
            && let Ok(pkg_list) = load_package_list_any(&packages_yaml)
        {
            config.packages.extend(pkg_list.packages);
        }
    }

    Ok(config)
}

/// Read the package_manager field from the pointer config.yaml (if it's a pointer)
pub(super) fn read_pointer_package_manager(paths: &ConfigPaths) -> Option<PackageManagerType> {
    let content = std::fs::read_to_string(&paths.config_file).ok()?;
    if !is_pointer_config_raw(&content) {
        return None;
    }
    let config: Config = yaml_serde::from_str(&content).ok()?;
    config.package_manager
}

/// Check if a config file is a "pointer" config by examining raw YAML content
/// A pointer config only has the "host" field (and optionally "package_manager")
pub(super) fn is_pointer_config_raw(yaml_content: &str) -> bool {
    // Parse as a raw YAML value to check field count
    let value: yaml_serde::Value = match yaml_serde::from_str(yaml_content) {
        Ok(v) => v,
        Err(_) => return false,
    };

    // Check if it's a mapping with only pointer fields (host, and optionally package_manager)
    if let yaml_serde::Value::Mapping(map) = value {
        let has_host = map.contains_key(yaml_serde::Value::String("host".to_string()));
        let has_pkg_mgr =
            map.contains_key(yaml_serde::Value::String("package_manager".to_string()));

        // A pointer config has "host" and optionally "package_manager", nothing else
        if has_host && (map.len() == 1 || (map.len() == 2 && has_pkg_mgr)) {
            return true;
        }
    }

    false
}

/// Load a config file from a specific path (used for host files)
/// Supports YAML, Lua, and Nix config files based on extension
pub(super) fn load_config_from_file(paths: &ConfigPaths, file_path: &Path) -> Result<Config> {
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

            yaml_serde::from_str(&content)
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
pub(super) fn load_config_with_imports(
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
            yaml_serde::from_str(&content)
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
