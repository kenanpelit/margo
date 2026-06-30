//! Lua service profile loader
//!
//! Loads service profiles from the services/ directory.
//! Service profiles are Lua files that define which systemd services to enable/disable.

use anyhow::{anyhow, Context, Result};
use mlua::{Table, Value};
use std::path::Path;

use crate::config::{ServiceProfile, ServicesConfig};

/// Load a service profile from a Lua file
pub fn load_service_profile(path: &Path) -> Result<ServiceProfile> {
    let lua = super::create_sandboxed_lua()?;

    // Register mdots helpers (for conditional logic)
    super::helpers::register_helpers(&lua)?;
    super::hardware::register_hardware_helpers(&lua)?;
    super::package::register_package_helpers(&lua)?;
    super::service::register_service_helpers(&lua)?;
    super::power::register_power_helpers(&lua)?;
    super::security::register_security_helpers(&lua)?;
    super::desktop::register_desktop_helpers(&lua)?;
    super::boot::register_boot_helpers(&lua)?;
    super::network::register_network_helpers(&lua)?;
    super::audio::register_audio_helpers(&lua)?;
    super::storage::register_storage_helpers(&lua)?;

    // Load and execute the Lua file
    let script = std::fs::read_to_string(path)
        .context(format!("Failed to read service profile: {:?}", path))?;

    let result: Table = lua
        .load(&script)
        .set_name(path.to_string_lossy())
        .eval()
        .map_err(|e| anyhow!("Failed to execute service profile {:?}: {}", path, e))?;

    // Extract profile configuration
    extract_service_profile(path, &result)
}

/// Extract ServiceProfile from a Lua table
fn extract_service_profile(path: &Path, table: &Table) -> Result<ServiceProfile> {
    // Get profile name from filename
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Extract description (optional)
    let description: String = table.get("description").unwrap_or_default();

    // Extract services configuration (required)
    let services = extract_services(table)?;

    // Extract conflicts (optional)
    let conflicts: Vec<String> = table.get("conflicts").unwrap_or_default();

    Ok(ServiceProfile {
        name,
        path: path.to_path_buf(),
        description,
        services,
        conflicts,
    })
}

/// Extract services configuration from Lua table
fn extract_services(table: &Table) -> Result<ServicesConfig> {
    use crate::config::ServiceScope;

    let services_value: Value = table
        .get("services")
        .map_err(|_| anyhow!("Service profile must have a 'services' field"))?;

    match services_value {
        Value::Nil => Ok(ServicesConfig::default()),
        Value::Table(t) => {
            let enabled: Vec<String> = t.get("enabled").unwrap_or_default();
            let disabled: Vec<String> = t.get("disabled").unwrap_or_default();
            let scope_str: Option<String> = t.get("scope").ok();
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

/// Validate a service profile file
// kept: pub API for validating service profiles (validates parse without returning parsed result)
#[allow(dead_code)]
pub fn validate_service_profile(path: &Path) -> Result<()> {
    load_service_profile(path)?;
    Ok(())
}
