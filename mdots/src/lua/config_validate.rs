//! Detailed validation of Lua host configs.

use super::*;

/// Validate a Lua config file
// kept: pub API for Lua config validation (counterpart to validate_lua_module)
#[allow(dead_code)]
pub fn validate_lua_config(path: &Path) -> Result<()> {
    let result = validate_lua_config_detailed(path);

    if !result.valid {
        let error_messages: Vec<String> = result.errors.iter().map(|e| e.to_string()).collect();
        anyhow::bail!("{}", error_messages.join("\n"));
    }

    Ok(())
}

/// Validate a Lua config file with detailed error information
// kept: pub API for Lua config validation (counterpart to validate_lua_module_detailed)
#[allow(dead_code)]
pub fn validate_lua_config_detailed(path: &Path) -> LuaValidationResult {
    let mut result = LuaValidationResult::new();

    // Check file exists
    if !path.exists() {
        result.add_error(LuaError {
            kind: LuaErrorKind::FileNotFound,
            message: format!("Lua config file not found: {:?}", path),
            line: None,
            hint: Some("Check that the file path is correct".to_string()),
        });
        return result;
    }

    // Read the file
    let script = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            result.add_error(LuaError {
                kind: LuaErrorKind::FileNotFound,
                message: format!("Failed to read Lua config: {}", e),
                line: None,
                hint: Some("Check file permissions".to_string()),
            });
            return result;
        }
    };

    // Create Lua environment
    let lua = match create_sandboxed_lua() {
        Ok(l) => l,
        Err(e) => {
            result.add_error(LuaError {
                kind: LuaErrorKind::RuntimeError,
                message: format!("Failed to create Lua environment: {}", e),
                line: None,
                hint: None,
            });
            return result;
        }
    };

    // Register helpers
    if let Err(e) = helpers::register_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    if let Err(e) = hardware::register_hardware_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register hardware helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    if let Err(e) = package::register_package_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register package helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    if let Err(e) = service::register_service_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register service helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    if let Err(e) = power::register_power_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register power helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    if let Err(e) = security::register_security_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register security helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    if let Err(e) = desktop::register_desktop_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register desktop helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    if let Err(e) = boot::register_boot_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register boot helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    if let Err(e) = network::register_network_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register network helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    if let Err(e) = audio::register_audio_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register audio helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    if let Err(e) = storage::register_storage_helpers(&lua) {
        result.add_error(LuaError {
            kind: LuaErrorKind::RuntimeError,
            message: format!("Failed to register storage helpers: {}", e),
            line: None,
            hint: None,
        });
        return result;
    }

    // Try to load and execute
    let table: Table = match lua.load(&script).set_name(path.to_string_lossy()).eval() {
        Ok(t) => t,
        Err(e) => {
            let (error, line, hint) = parse_lua_error(&e);
            result.add_error(LuaError {
                kind: if error.contains("syntax error") || error.contains("unexpected") {
                    LuaErrorKind::SyntaxError
                } else {
                    LuaErrorKind::RuntimeError
                },
                message: error,
                line,
                hint,
            });
            return result;
        }
    };

    // Validate the returned table structure for config
    validate_config_table_detailed(&table, &mut result);

    result
}

/// Validate the config table structure with detailed reporting
pub(super) fn validate_config_table_detailed(table: &Table, result: &mut LuaValidationResult) {
    // Check for required 'host' field
    match table.get::<Value>("host") {
        Ok(Value::String(_)) => {
            // Valid host
        }
        Ok(Value::Nil) => {
            result.add_error(LuaError {
                kind: LuaErrorKind::MissingField,
                message: "Config must have a 'host' field".to_string(),
                line: None,
                hint: Some("Add: host = \"your-hostname\"".to_string()),
            });
        }
        Ok(_) => {
            result.add_error(LuaError {
                kind: LuaErrorKind::InvalidType,
                message: "'host' field must be a string".to_string(),
                line: None,
                hint: Some("Use: host = \"your-hostname\"".to_string()),
            });
        }
        Err(_) => {
            result.add_error(LuaError {
                kind: LuaErrorKind::MissingField,
                message: "Could not read 'host' field".to_string(),
                line: None,
                hint: Some("Add: host = \"your-hostname\"".to_string()),
            });
        }
    }

    // Check for valid description (optional)
    if let Ok(v) = table.get::<Value>("description")
        && !matches!(v, Value::String(_) | Value::Nil)
    {
        result.add_error(LuaError {
            kind: LuaErrorKind::InvalidType,
            message: "'description' field must be a string".to_string(),
            line: None,
            hint: Some("Use: description = \"My config description\"".to_string()),
        });
    }

    // Validate flatpak_scope if present
    if let Ok(Value::String(s)) = table.get::<Value>("flatpak_scope")
        && let Ok(scope_borrowed) = s.to_str()
    {
        let scope = scope_borrowed.to_string();
        if !["user", "system"].contains(&scope.as_str()) {
            result.add_error(LuaError {
                kind: LuaErrorKind::InvalidValue,
                message: format!("Invalid flatpak_scope: '{}'", scope),
                line: None,
                hint: Some("Valid values are: 'user', 'system'".to_string()),
            });
        }
    }

    // Validate module_processing if present
    if let Ok(Value::String(s)) = table.get::<Value>("module_processing")
        && let Ok(mode_borrowed) = s.to_str()
    {
        let mode = mode_borrowed.to_string();
        if !["parallel", "sequential"].contains(&mode.as_str()) {
            result.add_error(LuaError {
                kind: LuaErrorKind::InvalidValue,
                message: format!("Invalid module_processing: '{}'", mode),
                line: None,
                hint: Some("Valid values are: 'parallel', 'sequential'".to_string()),
            });
        }
    }

    // Check for common typos in field names
    let valid_fields = [
        "host",
        "description",
        "import",
        "enabled_modules",
        "packages",
        "exclude",
        "additional_packages",
        "backup_tool",
        "snapper_config",
        "flatpak_scope",
        "auto_prune",
        "module_processing",
        "strict_package_order",
        "config_backups",
        "system_backups",
        "services",
        "update_hooks",
        "default_apps",
        "editor",
        "aur_helper",
    ];

    if let Ok(pairs) = table
        .clone()
        .pairs::<String, Value>()
        .collect::<Result<Vec<_>, _>>()
    {
        for (key, _) in pairs {
            if !valid_fields.contains(&key.as_str()) {
                let suggestion = suggest_field_name(&key, &valid_fields);
                if let Some(suggested) = suggestion {
                    result.add_warning(format!(
                        "Unknown field '{}'. Did you mean '{}'?",
                        key, suggested
                    ));
                }
            }
        }
    }
}
