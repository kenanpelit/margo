//! Loading `source.lua` (the configuration source settings).

use super::*;

/// Load a source.lua config file and return a SourceConfig
pub fn load_lua_source(path: &Path) -> Result<crate::source::SourceConfig> {
    let lua = create_sandboxed_lua()?;

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

    let script =
        std::fs::read_to_string(path).context(format!("Failed to read Lua source: {:?}", path))?;

    let result: Table = lua
        .load(&script)
        .set_name(path.to_string_lossy())
        .eval()
        .map_err(|e| anyhow!("Failed to execute Lua source {:?}: {}", path, e))?;

    extract_source_config(&result)
}

pub(super) fn extract_source_config(table: &Table) -> Result<crate::source::SourceConfig> {
    let name: String = table
        .get("name")
        .map_err(|e| anyhow!("source.lua must have a 'name' field: {}", e))?;

    let description: String = table.get("description").unwrap_or_default();

    let url: String = table
        .get("url")
        .map_err(|e| anyhow!("source.lua must have a 'url' field: {}", e))?;

    let branch: Option<String> = table.get("branch").ok();

    let dependencies = lua_string_array(table, "dependencies")?;
    let runtime_dependencies = lua_string_array(table, "runtime_dependencies")?;
    let build_commands = lua_string_array(table, "build_commands")?;
    let package_commands = lua_string_array(table, "package_commands")?;

    let custom_pkgbuild: Option<String> = table.get("custom_pkgbuild").ok();
    let cache_builds: bool = table.get("cache_builds").unwrap_or(false);

    Ok(crate::source::SourceConfig {
        name,
        description,
        url,
        branch,
        dependencies,
        runtime_dependencies,
        build_commands,
        package_commands,
        custom_pkgbuild,
        cache_builds,
    })
}

pub(super) fn lua_string_array(table: &Table, key: &str) -> Result<Vec<String>> {
    let value: Value = match table.get(key) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };

    match value {
        Value::Nil => Ok(Vec::new()),
        Value::Table(t) => {
            let mut result = Vec::new();
            for pair in t.pairs::<i64, Value>() {
                let (_, v) = pair.map_err(|e| anyhow!("Lua error: {}", e))?;
                if let Value::String(s) = v {
                    result.push(
                        s.to_str()
                            .map_err(|e| anyhow!("Lua error: {}", e))?
                            .to_string(),
                    );
                }
            }
            Ok(result)
        }
        _ => anyhow::bail!("'{}' must be a table/array of strings", key),
    }
}
