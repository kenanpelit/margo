//! Detailed validation of Lua module files: structure, field names (with
//! "did you mean" suggestions) and the printed report.

use super::*;

/// Validate a Lua module with detailed error information
pub fn validate_lua_module_detailed(path: &Path) -> LuaValidationResult {
    let mut result = LuaValidationResult::new();

    // Check file exists
    if !path.exists() {
        result.add_error(LuaError {
            kind: LuaErrorKind::FileNotFound,
            message: format!("Lua module file not found: {:?}", path),
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
                message: format!("Failed to read Lua module: {}", e),
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

    // Validate the returned table structure
    validate_module_table_detailed(&table, &mut result);

    // Additional validation checks
    validate_module_content(&table, path, &mut result);

    result
}

/// Validate the module table structure with detailed reporting
pub(super) fn validate_module_table_detailed(table: &Table, result: &mut LuaValidationResult) {
    // Check for 'packages' field
    match table.get::<Value>("packages") {
        Ok(Value::Table(_)) => {
            // Valid packages table
        }
        Ok(Value::Nil) => {
            // packages is nil - could be intentional for empty module
            result.add_warning("Module has no 'packages' field or it is nil".to_string());
        }
        Ok(_) => {
            result.add_error(LuaError {
                kind: LuaErrorKind::InvalidType,
                message: "'packages' field must be a table/array".to_string(),
                line: None,
                hint: Some("Use: packages = { \"pkg1\", \"pkg2\" }".to_string()),
            });
        }
        Err(_) => {
            result.add_warning("Could not read 'packages' field".to_string());
        }
    }

    // Check for valid description
    match table.get::<Value>("description") {
        Ok(Value::String(_)) => {
            // Valid description
        }
        Ok(Value::Nil) => {
            result.add_warning("Module has no 'description' field".to_string());
        }
        Ok(_) => {
            result.add_error(LuaError {
                kind: LuaErrorKind::InvalidType,
                message: "'description' field must be a string".to_string(),
                line: None,
                hint: Some("Use: description = \"My module description\"".to_string()),
            });
        }
        Err(_) => {}
    }

    // Validate hook_behavior if present
    if let Ok(Value::String(s)) = table.get::<Value>("hook_behavior")
        && let Ok(behavior_borrowed) = s.to_str()
    {
        let behavior = behavior_borrowed.to_string();
        if !["ask", "once", "always", "skip", "never"].contains(&behavior.as_str()) {
            result.add_error(LuaError {
                kind: LuaErrorKind::InvalidValue,
                message: format!("Invalid hook_behavior: '{}'", behavior),
                line: None,
                hint: Some(
                    "Valid values are: 'ask', 'once', 'always', 'skip', 'never'".to_string(),
                ),
            });
        }
    }

    // Check for common typos in field names
    let valid_fields = [
        "description",
        "packages",
        "conflicts",
        "services",
        "pre_install_hook",
        "post_install_hook",
        "hook_behavior",
        "pre_hook_behavior",
        "post_hook_behavior",
        "metadata",
        "package_files",
        "dotfiles_sync",
        "dotfiles",
    ];

    if let Ok(pairs) = table
        .clone()
        .pairs::<String, Value>()
        .collect::<Result<Vec<_>, _>>()
    {
        for (key, _) in pairs {
            if !valid_fields.contains(&key.as_str()) {
                // Check for common typos
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

/// Validate module content (packages, hooks, etc.)
pub(super) fn validate_module_content(
    table: &Table,
    path: &Path,
    result: &mut LuaValidationResult,
) {
    let module_dir = path.parent().unwrap_or(Path::new("."));

    // Validate packages array content
    if let Ok(Value::Table(packages)) = table.get::<Value>("packages") {
        let mut seen_packages = std::collections::HashSet::new();

        for (_, value) in packages.pairs::<i64, Value>().flatten() {
            match &value {
                Value::String(s) => {
                    if let Ok(pkg_borrowed) = s.to_str() {
                        let pkg_name = pkg_borrowed.to_string();
                        if pkg_name.is_empty() {
                            result.add_warning(
                                "Empty package name found in packages array".to_string(),
                            );
                        } else if !seen_packages.insert(pkg_name.clone()) {
                            result.add_warning(format!("Duplicate package: {}", pkg_name));
                        }
                    }
                }
                Value::Table(t) => {
                    // Validate table format package
                    if t.get::<String>("name").is_err() {
                        result.add_error(LuaError {
                            kind: LuaErrorKind::MissingField,
                            message: "Package table entry must have 'name' field".to_string(),
                            line: None,
                            hint: Some(
                                "Use: { name = \"package-name\", type = \"pacman\" }".to_string(),
                            ),
                        });
                    }
                }
                _ => {
                    result.add_error(LuaError {
                        kind: LuaErrorKind::InvalidType,
                        message: "Package entry must be a string or table".to_string(),
                        line: None,
                        hint: Some(
                            "Use: \"package-name\" or { name = \"pkg\", type = \"flatpak\" }"
                                .to_string(),
                        ),
                    });
                }
            }
        }

        if seen_packages.is_empty() {
            result.add_warning("Module has empty packages list".to_string());
        }
    }

    // Validate pre_install_hook path if specified
    if let Ok(Value::String(hook)) = table.get::<Value>("pre_install_hook")
        && let Ok(hook_borrowed) = hook.to_str()
    {
        let hook_str = hook_borrowed.to_string();
        if !hook_str.is_empty() {
            let hook_path = module_dir.join(&hook_str);
            if !hook_path.exists() {
                result.add_error(LuaError {
                    kind: LuaErrorKind::FileNotFound,
                    message: format!("pre_install_hook script not found: {}", hook_str),
                    line: None,
                    hint: Some(format!("Expected at: {:?}", hook_path)),
                });
            }
        }
    }

    // Validate post_install_hook path if specified
    if let Ok(Value::String(hook)) = table.get::<Value>("post_install_hook")
        && let Ok(hook_borrowed) = hook.to_str()
    {
        let hook_str = hook_borrowed.to_string();
        if !hook_str.is_empty() {
            let hook_path = module_dir.join(&hook_str);
            if !hook_path.exists() {
                result.add_error(LuaError {
                    kind: LuaErrorKind::FileNotFound,
                    message: format!("post_install_hook script not found: {}", hook_str),
                    line: None,
                    hint: Some(format!("Expected at: {:?}", hook_path)),
                });
            }
        }
    }

    // Validate services structure if present
    if let Ok(Value::Table(services)) = table.get::<Value>("services") {
        if let Ok(Value::Table(_)) = services.get::<Value>("enabled") {
            // Valid
        } else if services.get::<Value>("enabled").is_ok() {
            result.add_error(LuaError {
                kind: LuaErrorKind::InvalidType,
                message: "services.enabled must be a table/array".to_string(),
                line: None,
                hint: Some("Use: services = { enabled = { \"svc.service\" } }".to_string()),
            });
        }

        if let Ok(Value::Table(_)) = services.get::<Value>("disabled") {
            // Valid
        } else if services.get::<Value>("disabled").is_ok() {
            result.add_error(LuaError {
                kind: LuaErrorKind::InvalidType,
                message: "services.disabled must be a table/array".to_string(),
                line: None,
                hint: Some("Use: services = { disabled = { \"svc.service\" } }".to_string()),
            });
        }
    }
}

/// Suggest a correct field name for a typo
pub(super) fn suggest_field_name(input: &str, valid_fields: &[&str]) -> Option<String> {
    let input_lower = input.to_lowercase();

    // Direct substring match
    for &field in valid_fields {
        if field.contains(&input_lower) || input_lower.contains(field) {
            return Some(field.to_string());
        }
    }

    // Simple edit distance check (very basic)
    for &field in valid_fields {
        if levenshtein_distance(&input_lower, field) <= 2 {
            return Some(field.to_string());
        }
    }

    None
}

/// Simple Levenshtein distance calculation
// allow: Levenshtein matrix needs both axes for `a_chars[i-1]` and `b_chars[j-1]` lookups;
//        iterator rewrite would require unsafe indexing or equivalent complexity.
#[allow(clippy::needless_range_loop)]
pub(super) fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];

    for i in 0..=a_len {
        matrix[i][0] = i;
    }
    for j in 0..=b_len {
        matrix[0][j] = j;
    }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = std::cmp::min(
                std::cmp::min(matrix[i - 1][j] + 1, matrix[i][j - 1] + 1),
                matrix[i - 1][j - 1] + cost,
            );
        }
    }

    matrix[a_len][b_len]
}

/// Format a Lua validation result for display
// kept: pub API counterpart to validate_lua_module_detailed used by callers wanting formatted output
#[allow(dead_code)]
pub fn format_validation_result(result: &LuaValidationResult, module_name: &str) -> String {
    let mut output = String::new();

    if result.valid && result.warnings.is_empty() {
        output.push_str(&format!("✓ {} - Valid\n", module_name));
    } else if result.valid {
        output.push_str(&format!("⚠ {} - Valid with warnings\n", module_name));
        for warning in &result.warnings {
            output.push_str(&format!("  Warning: {}\n", warning));
        }
    } else {
        output.push_str(&format!("✗ {} - Invalid\n", module_name));
        for error in &result.errors {
            output.push_str(&format!("  Error: {}\n", error));
        }
        for warning in &result.warnings {
            output.push_str(&format!("  Warning: {}\n", warning));
        }
    }

    output
}
