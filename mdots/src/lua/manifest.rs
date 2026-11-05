//! Lua directory modules: `module.lua` manifests and their dotfile entries.

use super::*;

/// Load a Lua manifest file for a directory module
/// This is used when a directory module has module.lua instead of module.yaml
// kept: pub API for loading a standalone Lua manifest (directory-module variant)
#[allow(dead_code)]
pub fn load_lua_manifest(path: &Path) -> Result<ModuleManifest> {
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
    let script = std::fs::read_to_string(path)
        .context(format!("Failed to read Lua manifest: {:?}", path))?;

    let result: Table = lua
        .load(&script)
        .set_name(path.to_string_lossy())
        .eval()
        .map_err(|e| anyhow!("Failed to execute Lua manifest {:?}: {}", path, e))?;

    // Extract manifest configuration from returned table
    extract_manifest_config(&result)
}

/// Lua directory module parsed from module.lua (manifest + optional inline packages).
pub struct LuaDirectoryModule {
    pub manifest: ModuleManifest,
    pub packages: Vec<PackageEntry>,
}

/// Load a Lua directory module (manifest + optional packages) from module.lua.
pub fn load_lua_directory_module(path: &Path) -> Result<LuaDirectoryModule> {
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
    let script = std::fs::read_to_string(path)
        .context(format!("Failed to read Lua manifest: {:?}", path))?;

    let result: Table = lua
        .load(&script)
        .set_name(path.to_string_lossy())
        .eval()
        .map_err(|e| anyhow!("Failed to execute Lua manifest {:?}: {}", path, e))?;

    let manifest = extract_manifest_config(&result)?;
    let packages = extract_packages_optional(&result)?;

    Ok(LuaDirectoryModule { manifest, packages })
}

/// Extract ModuleManifest from a Lua table
pub(super) fn extract_manifest_config(table: &Table) -> Result<ModuleManifest> {
    // Extract description (optional, defaults to empty)
    let description: String = table.get("description").unwrap_or_default();

    // Extract conflicts (optional)
    let conflicts: Vec<String> = table.get("conflicts").unwrap_or_default();

    // Extract hooks (optional)
    let pre_install_hook: Option<String> = table.get("pre_install_hook").ok();
    let post_install_hook: Option<String> = table.get("post_install_hook").ok();

    // Extract hook behaviors
    let hook_behavior: String = table
        .get("hook_behavior")
        .unwrap_or_else(|_| "ask".to_string());
    let pre_hook_behavior: Option<String> = table.get("pre_hook_behavior").ok();
    let post_hook_behavior: Option<String> = table.get("post_hook_behavior").ok();
    let post_disable_hook: Option<String> = table.get("post_disable_hook").ok();
    let post_disable_behavior: Option<String> = table.get("post_disable_behavior").ok();
    let run_hooks_as_user: RunHooksAsUser = table
        .get::<bool>("run_hooks_as_user")
        .map(RunHooksAsUser::Bool)
        .unwrap_or_else(|_| RunHooksAsUser::Bool(false));

    // Extract package_files (optional - empty means auto-discover)
    let package_files: Vec<String> = table.get("package_files").unwrap_or_default();

    // Extract dotfiles_sync (optional)
    let dotfiles_sync: Option<bool> = table.get("dotfiles_sync").ok();

    // Extract dotfiles list (optional)
    let dotfiles = extract_dotfiles(table)?;

    Ok(ModuleManifest {
        description,
        conflicts,
        pre_install_hook,
        post_install_hook,
        hook_behavior,
        pre_hook_behavior,
        post_hook_behavior,
        run_hooks_as_user,
        post_disable_hook,
        post_disable_behavior,
        package_files,
        dotfiles_sync,
        dotfiles,
        // Sharing metadata - not available from directory-based Lua manifest parsing
        author: None,
        version: None,
        category: None,
        tags: Vec::new(),
        license: None,
        upstream_url: None,
    })
}

/// Extract dotfiles array from Lua table
pub(super) fn extract_dotfiles(table: &Table) -> Result<Vec<DotfileEntry>> {
    let dotfiles_value: Value = match table.get("dotfiles") {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };

    let dotfiles_table = match dotfiles_value {
        Value::Table(t) => t,
        Value::Nil => return Ok(Vec::new()),
        _ => anyhow::bail!("'dotfiles' must be a table/array"),
    };

    let mut dotfiles = Vec::new();

    for pair in dotfiles_table.pairs::<i64, Value>() {
        let (_, value) = pair.map_err(|e| anyhow!("Lua error: {}", e))?;
        match value {
            Value::Table(t) => {
                let source: String = t
                    .get("source")
                    .map_err(|_| anyhow!("Dotfile entry must have 'source' field"))?;
                let target: String = t
                    .get("target")
                    .map_err(|_| anyhow!("Dotfile entry must have 'target' field"))?;
                dotfiles.push(DotfileEntry { source, target });
            }
            _ => anyhow::bail!("Dotfile entry must be a table with 'source' and 'target' fields"),
        }
    }

    Ok(dotfiles)
}

/// Convert Lua value to JSON (for metadata)
pub(super) fn lua_to_json(value: &Value) -> Result<serde_json::Value> {
    match value {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(i) => Ok(serde_json::Value::Number((*i).into())),
        Value::Number(n) => Ok(serde_json::json!(*n)),
        Value::String(s) => Ok(serde_json::Value::String(
            s.to_str()
                .map_err(|e| anyhow!("Lua error: {}", e))?
                .to_string(),
        )),
        Value::Table(t) => {
            // Check if it's an array or object
            let mut is_array = true;
            let mut max_index = 0i64;

            for pair in t.clone().pairs::<Value, Value>() {
                let (k, _) = pair.map_err(|e| anyhow!("Lua error: {}", e))?;
                match k {
                    Value::Integer(i) if i > 0 => {
                        max_index = max_index.max(i);
                    }
                    _ => {
                        is_array = false;
                        break;
                    }
                }
            }

            if is_array && max_index > 0 {
                let mut arr = Vec::new();
                for i in 1..=max_index {
                    let v: Value = t.get(i).map_err(|e| anyhow!("Lua error: {}", e))?;
                    arr.push(lua_to_json(&v)?);
                }
                Ok(serde_json::Value::Array(arr))
            } else {
                let mut map = serde_json::Map::new();
                for pair in t.pairs::<String, Value>() {
                    let (k, v) = pair.map_err(|e| anyhow!("Lua error: {}", e))?;
                    map.insert(k, lua_to_json(&v)?);
                }
                Ok(serde_json::Value::Object(map))
            }
        }
        _ => Ok(serde_json::Value::Null),
    }
}
