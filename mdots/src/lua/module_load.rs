//! Loading a Lua module file into a `LuaModule` (packages, services, hooks).

use super::*;

/// Load a Lua module from a file path
pub fn load_lua_module(path: &Path) -> Result<LuaModule> {
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

    // Load and execute the Lua file
    let script =
        std::fs::read_to_string(path).context(format!("Failed to read Lua module: {:?}", path))?;

    let result: Table = lua
        .load(&script)
        .set_name(path.to_string_lossy())
        .eval()
        .map_err(|e| anyhow!("Failed to execute Lua module {:?}: {}", path, e))?;

    // Extract module configuration from returned table
    let module = extract_module_config(path, &result)?;

    Ok(module)
}

/// Validate a Lua module without fully loading it
// kept: pub API convenience wrapper over validate_lua_module_detailed for callers wanting Result<()>
#[allow(dead_code)]
pub fn validate_lua_module(path: &Path) -> Result<()> {
    let result = validate_lua_module_detailed(path);

    if !result.valid {
        let error_messages: Vec<String> = result.errors.iter().map(|e| e.to_string()).collect();
        anyhow::bail!("{}", error_messages.join("\n"));
    }

    Ok(())
}

/// Extract module configuration from Lua table
pub(super) fn extract_module_config(path: &Path, table: &Table) -> Result<LuaModule> {
    // Extract description (optional, defaults to empty)
    let description: String = table
        .get("description")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .unwrap_or_default();

    // Extract packages (required)
    let mut packages = extract_packages(table)?;

    // Extract flatpak_packages (optional, new separate field)
    let flatpak_packages: Vec<String> = table
        .get("flatpak_packages")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .unwrap_or_default();
    for fp in &flatpak_packages {
        packages.push(PackageEntry::WithType {
            name: fp.clone(),
            r#type: Some(PackageType::Flatpak),
            remote: None,
        });
    }

    // Extract nix_packages (optional, new separate field)
    let nix_packages: Vec<String> = table
        .get("nix_packages")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .unwrap_or_default();
    for np in &nix_packages {
        packages.push(PackageEntry::WithType {
            name: np.clone(),
            r#type: Some(PackageType::Nix),
            remote: None,
        });
    }

    // Extract services (optional)
    let services = extract_services(table)?;

    // Extract conflicts (optional)
    let conflicts: Vec<String> = table
        .get("conflicts")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .unwrap_or_default();

    // Extract hooks (optional)
    let pre_install_hook: Option<String> = table
        .get("pre_install_hook")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();
    let post_install_hook: Option<String> = table
        .get("post_install_hook")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();
    let hook_behavior: String = table
        .get("hook_behavior")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .unwrap_or_else(|_| "ask".to_string());
    let pre_hook_behavior: Option<String> = table
        .get("pre_hook_behavior")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();
    let post_hook_behavior: Option<String> = table
        .get("post_hook_behavior")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();
    let post_disable_hook: Option<String> = table
        .get("post_disable_hook")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();
    let post_disable_behavior: Option<String> = table
        .get("post_disable_behavior")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();
    let run_hooks_as_user: RunHooksAsUser = table
        .get::<bool>("run_hooks_as_user")
        .map(RunHooksAsUser::Bool)
        .unwrap_or_else(|_| RunHooksAsUser::Bool(false));

    // Extract metadata (optional)
    let metadata: Option<serde_json::Value> = table
        .get::<Value>("metadata")
        .ok()
        .and_then(|v| lua_to_json(&v).ok());

    // Extract sharing metadata fields (optional - for module upload/download)
    let author: Option<String> = table
        .get("author")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();
    let version: Option<String> = table
        .get("version")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();
    let category: Option<String> = table
        .get("category")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();
    let tags: Vec<String> = table
        .get("tags")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .unwrap_or_default();
    let license: Option<String> = table
        .get("license")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();
    let upstream_url: Option<String> = table
        .get("upstream_url")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .ok();

    Ok(LuaModule {
        path: path.to_path_buf(),
        description,
        packages,
        services,
        conflicts,
        pre_install_hook,
        post_install_hook,
        hook_behavior,
        pre_hook_behavior,
        post_hook_behavior,
        run_hooks_as_user,
        post_disable_hook,
        post_disable_behavior,
        metadata,
        author,
        version,
        category,
        tags,
        license,
        upstream_url,
    })
}

/// Extract packages from Lua table
pub(super) fn extract_packages(table: &Table) -> Result<Vec<PackageEntry>> {
    let packages_value: Value = table
        .get("packages")
        .map_err(|e| anyhow!("Lua module must return a 'packages' field: {}", e))?;

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
                // Check for flatpak: or nix: prefix
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
                // Handle {name = "pkg", type = "flatpak"} format
                let name: String = t.get("name").map_err(|e| anyhow!("Lua error: {}", e))?;
                let pkg_type: Option<String> =
                    t.get("type").map_err(|e| anyhow!("Lua error: {}", e)).ok();

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

/// Extract a Lua array of strings by key from a table
pub(super) fn extract_string_array(table: &Table, key: &str) -> Result<Vec<String>> {
    let value: Value = table
        .get(key)
        .map_err(|e| anyhow!("Lua error reading '{}': {}", key, e))?;

    match value {
        Value::Nil => Ok(Vec::new()),
        Value::Table(arr) => {
            let mut result = Vec::new();
            for pair in arr.pairs::<i64, Value>() {
                let (_, v) = pair.map_err(|e| anyhow!("Lua error iterating '{}': {}", key, e))?;
                match v {
                    Value::String(s) => {
                        result.push(
                            s.to_str()
                                .map_err(|e| anyhow!("Lua string error in '{}': {}", key, e))?
                                .to_string(),
                        );
                    }
                    _ => anyhow::bail!("'{}' entries must be strings", key),
                }
            }
            Ok(result)
        }
        _ => anyhow::bail!("'{}' must be an array", key),
    }
}

/// Extract services configuration from Lua table
pub(super) fn extract_services(table: &Table) -> Result<ServicesConfig> {
    use crate::config::ServiceScope;

    let services_value: Value = table
        .get("services")
        .map_err(|e| anyhow!("Lua error: {}", e))
        .unwrap_or(Value::Nil);

    match services_value {
        Value::Nil => Ok(ServicesConfig::default()),
        Value::Table(t) => {
            let enabled = extract_string_array(&t, "enabled")?;
            let disabled = extract_string_array(&t, "disabled")?;
            let scope_str: Option<String> =
                t.get("scope").map_err(|e| anyhow!("Lua error: {}", e)).ok();
            let scope = match scope_str.as_deref() {
                Some("user") => ServiceScope::User,
                _ => ServiceScope::System,
            };

            Ok(ServicesConfig {
                enabled,
                disabled,
                scope,
            })
        }
        _ => anyhow::bail!("'services' must be a table"),
    }
}
