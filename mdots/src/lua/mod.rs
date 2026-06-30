//! Lua module support for mdots
//!
//! This module provides the infrastructure for loading and executing
//! Lua-based module configurations.

mod audio;
mod boot;
mod desktop;
mod hardware;
mod helpers;
mod network;
mod package;
mod power;
mod sandbox;
mod security;
mod service;
pub mod service_profile;
mod storage;

use anyhow::{anyhow, Context, Result};
use mlua::{Lua, Table, Value};
use std::path::{Path, PathBuf};

use crate::config::{
    Config, ConfigBackupsSettings, DefaultAppsConfig, DefaultsScope, DotfileEntry, FlatpakScope,
    ModuleManifest, ModuleProcessing, PackageEntry, PackageType, RunHooksAsUser, ServicesConfig,
    SystemBackupsSettings, UpdateHooksConfig,
};

/// Lua validation result with detailed information
#[derive(Debug, Clone)]
pub struct LuaValidationResult {
    pub valid: bool,
    pub errors: Vec<LuaError>,
    pub warnings: Vec<String>,
}

impl LuaValidationResult {
    pub fn new() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn add_error(&mut self, error: LuaError) {
        self.valid = false;
        self.errors.push(error);
    }

    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

impl Default for LuaValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Detailed Lua error information
#[derive(Debug, Clone)]
pub struct LuaError {
    #[allow(dead_code)]
    pub kind: LuaErrorKind,
    pub message: String,
    pub line: Option<u32>,
    pub hint: Option<String>,
}

impl std::fmt::Display for LuaError {
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

/// Types of Lua errors
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum LuaErrorKind {
    SyntaxError,
    RuntimeError,
    MissingField,
    InvalidType,
    InvalidValue,
    FileNotFound,
    AccessDenied,
}

/// Lua-based module structure
#[derive(Debug, Clone)]
pub struct LuaModule {
    /// Path to the .lua file
    pub path: PathBuf,

    /// Cached module description
    pub description: String,

    /// Cached package list
    pub packages: Vec<PackageEntry>,

    /// Cached services configuration
    #[allow(dead_code)]
    pub services: ServicesConfig,

    /// Cached conflicts list
    pub conflicts: Vec<String>,

    /// Pre-install hook (optional)
    pub pre_install_hook: Option<String>,

    /// Post-install hook (optional)
    pub post_install_hook: Option<String>,

    /// Hook behavior
    pub hook_behavior: String,

    /// Pre-hook behavior override
    pub pre_hook_behavior: Option<String>,

    /// Post-hook behavior override
    pub post_hook_behavior: Option<String>,

    /// Post-disable hook (optional)
    pub post_disable_hook: Option<String>,

    /// Post-disable hook behavior override
    #[allow(dead_code)]
    pub post_disable_behavior: Option<String>,

    /// Run hooks as specified user (without sudo)
    /// Can be: false (default, use sudo), true (current user), or "username"
    pub run_hooks_as_user: RunHooksAsUser,

    /// Optional metadata from Lua
    #[allow(dead_code)]
    pub metadata: Option<serde_json::Value>,

    // === Informational metadata fields (kept on the manifest, not actively read) ===
    /// Module author (e.g. a username)
    #[allow(dead_code)]
    pub author: Option<String>,

    /// Module version (semver format: X.Y.Z)
    #[allow(dead_code)]
    pub version: Option<String>,

    /// Category for organization (defaults to "other" if not specified)
    #[allow(dead_code)]
    pub category: Option<String>,

    /// Tags for search/filtering
    #[allow(dead_code)]
    pub tags: Vec<String>,

    /// License identifier (e.g., "MIT", "GPL-3.0")
    #[allow(dead_code)]
    pub license: Option<String>,

    /// URL to upstream project/documentation
    #[allow(dead_code)]
    pub upstream_url: Option<String>,
}

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

/// Parse a Lua error to extract line number and provide helpful hints
fn parse_lua_error(error: &mlua::Error) -> (String, Option<u32>, Option<String>) {
    let error_str = error.to_string();

    // Try to extract line number from error message
    // Format is typically: "[string \"filename\"]:line: message"
    let line = extract_line_number(&error_str);

    // Generate helpful hints based on error content
    let hint = generate_error_hint(&error_str);

    // Clean up the error message
    let message = clean_error_message(&error_str);

    (message, line, hint)
}

/// Extract line number from Lua error message
fn extract_line_number(error: &str) -> Option<u32> {
    // Pattern: ]:line: or :line:
    let re = regex::Regex::new(r"]:?(\d+):").ok()?;
    re.captures(error)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// Generate helpful hints based on error content
fn generate_error_hint(error: &str) -> Option<String> {
    let error_lower = error.to_lowercase();

    if error_lower.contains("unexpected symbol near '='") {
        return Some("Check for missing 'local' keyword or incorrect table syntax".to_string());
    }

    if error_lower.contains("attempt to index a nil value") {
        if error_lower.contains("mdots") {
            return Some(
                "Make sure you're using the correct mdots.* API (e.g., mdots.hardware.cpu_vendor())"
                    .to_string(),
            );
        }
        return Some("A variable is nil when you're trying to access its fields".to_string());
    }

    if error_lower.contains("attempt to call a nil value") {
        return Some(
            "The function you're trying to call doesn't exist. Check the API documentation."
                .to_string(),
        );
    }

    if error_lower.contains("'}' expected") {
        return Some(
            "Missing closing brace '}'. Check that all tables are properly closed.".to_string(),
        );
    }

    if error_lower.contains("'end' expected") {
        return Some(
            "Missing 'end' keyword. Check that all if/for/while/function blocks are closed."
                .to_string(),
        );
    }

    if error_lower.contains("syntax error near") {
        return Some("Check for typos, missing commas, or incorrect Lua syntax.".to_string());
    }

    if error_lower.contains("table expected") {
        return Some("The module must return a table with 'packages' field.".to_string());
    }

    if error_lower.contains("access denied") {
        return Some("File access is restricted for security. Only /sys/, /proc/, and /etc/os-release are allowed.".to_string());
    }

    None
}

/// Clean up error message for display
fn clean_error_message(error: &str) -> String {
    // Remove the "runtime error: " prefix if present
    let cleaned = error
        .trim_start_matches("runtime error: ")
        .trim_start_matches("syntax error: ");

    // Truncate very long messages
    if cleaned.len() > 200 {
        format!("{}...", &cleaned[..200])
    } else {
        cleaned.to_string()
    }
}

/// Validate the module table structure with detailed reporting
fn validate_module_table_detailed(table: &Table, result: &mut LuaValidationResult) {
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
    if let Ok(Value::String(s)) = table.get::<Value>("hook_behavior") {
        if let Ok(behavior_borrowed) = s.to_str() {
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
fn validate_module_content(table: &Table, path: &Path, result: &mut LuaValidationResult) {
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
    if let Ok(Value::String(hook)) = table.get::<Value>("pre_install_hook") {
        if let Ok(hook_borrowed) = hook.to_str() {
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
    }

    // Validate post_install_hook path if specified
    if let Ok(Value::String(hook)) = table.get::<Value>("post_install_hook") {
        if let Ok(hook_borrowed) = hook.to_str() {
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
fn suggest_field_name(input: &str, valid_fields: &[&str]) -> Option<String> {
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
fn levenshtein_distance(a: &str, b: &str) -> usize {
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

/// Create a sandboxed Lua environment
pub(crate) fn create_sandboxed_lua() -> Result<Lua> {
    let lua = Lua::new();
    sandbox::apply_sandbox(&lua)?;
    Ok(lua)
}

/// Extract module configuration from Lua table
fn extract_module_config(path: &Path, table: &Table) -> Result<LuaModule> {
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
fn extract_packages(table: &Table) -> Result<Vec<PackageEntry>> {
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
                    });
                } else if let Some(name) = pkg_str.strip_prefix("nix:") {
                    packages.push(PackageEntry::WithType {
                        name: name.to_string(),
                        r#type: Some(PackageType::Nix),
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
fn extract_string_array(table: &Table, key: &str) -> Result<Vec<String>> {
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
fn extract_services(table: &Table) -> Result<ServicesConfig> {
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
fn extract_manifest_config(table: &Table) -> Result<ModuleManifest> {
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
fn extract_dotfiles(table: &Table) -> Result<Vec<DotfileEntry>> {
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
fn lua_to_json(value: &Value) -> Result<serde_json::Value> {
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

fn create_config_lua_env() -> Result<Lua> {
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
fn create_config_lua_env_silent() -> Result<Lua> {
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
fn extract_config(table: &Table) -> Result<Config> {
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
fn extract_packages_optional(table: &Table) -> Result<Vec<PackageEntry>> {
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
                    });
                } else if let Some(name) = pkg_str.strip_prefix("nix:") {
                    packages.push(PackageEntry::WithType {
                        name: name.to_string(),
                        r#type: Some(PackageType::Nix),
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
fn extract_additional_packages(table: &Table) -> Result<Vec<PackageEntry>> {
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
fn extract_flatpak_scope(table: &Table) -> Result<FlatpakScope> {
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
fn extract_module_processing(table: &Table) -> Result<ModuleProcessing> {
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
fn extract_config_backups(table: &Table) -> Result<ConfigBackupsSettings> {
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
fn extract_system_backups(table: &Table) -> Result<SystemBackupsSettings> {
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
fn extract_update_hooks(table: &Table) -> Result<UpdateHooksConfig> {
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
fn extract_default_apps(table: &Table) -> Result<DefaultAppsConfig> {
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
fn extract_theming(table: &Table) -> Result<crate::config::ThemingConfig> {
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
fn extract_env_vars(table: &Table) -> Result<std::collections::HashMap<String, String>> {
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
fn extract_mime_types(table: &Table) -> Result<std::collections::HashMap<String, String>> {
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
fn validate_config_table_detailed(table: &Table, result: &mut LuaValidationResult) {
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
    if let Ok(v) = table.get::<Value>("description") {
        if !matches!(v, Value::String(_) | Value::Nil) {
            result.add_error(LuaError {
                kind: LuaErrorKind::InvalidType,
                message: "'description' field must be a string".to_string(),
                line: None,
                hint: Some("Use: description = \"My config description\"".to_string()),
            });
        }
    }

    // Validate flatpak_scope if present
    if let Ok(Value::String(s)) = table.get::<Value>("flatpak_scope") {
        if let Ok(scope_borrowed) = s.to_str() {
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
    }

    // Validate module_processing if present
    if let Ok(Value::String(s)) = table.get::<Value>("module_processing") {
        if let Ok(mode_borrowed) = s.to_str() {
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

fn extract_source_config(table: &Table) -> Result<crate::source::SourceConfig> {
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

fn lua_string_array(table: &Table, key: &str) -> Result<Vec<String>> {
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

// ---------------------------------------------------------------------------
// Lua API golden tests
// ---------------------------------------------------------------------------
//
// These tests guarantee that `mdots` is the canonical Lua global (locking in
// the rename from the legacy name) and that every registered sub-table
// (mdots.hardware, mdots.security, …) is reachable without triggering
// "attempt to index a nil value" errors in user Lua manifests.
//
// All tests are co-located here (not in tests/) because this is a bin-only
// crate and the functions under test are module-private.  No external
// binaries are required — tests are always hermetic.

#[cfg(test)]
mod tests {
    use super::*;

    // --- Helper: spin up a fully-registered Lua environment ------------------

    fn full_lua_env() -> Lua {
        let lua = create_sandboxed_lua().unwrap();
        helpers::register_helpers(&lua).unwrap();
        hardware::register_hardware_helpers(&lua).unwrap();
        package::register_package_helpers(&lua).unwrap();
        service::register_service_helpers(&lua).unwrap();
        power::register_power_helpers(&lua).unwrap();
        security::register_security_helpers(&lua).unwrap();
        desktop::register_desktop_helpers(&lua).unwrap();
        boot::register_boot_helpers(&lua).unwrap();
        network::register_network_helpers(&lua).unwrap();
        audio::register_audio_helpers(&lua).unwrap();
        storage::register_storage_helpers(&lua).unwrap();
        lua
    }

    // --- Test: the canonical global is `mdots`, and it is non-empty ----------

    #[test]
    fn lua_api_global_mdots_exists_and_is_non_empty() {
        let lua = create_sandboxed_lua().unwrap();
        helpers::register_helpers(&lua).unwrap();

        // `mdots` must exist and be a non-empty table.
        let mdots: mlua::Table = lua
            .globals()
            .get("mdots")
            .expect("mdots global must exist after register_helpers");
        assert!(
            !mlua::Table::is_empty(&mdots),
            "mdots table must not be empty"
        );
    }

    // --- Test: every registered sub-table is reachable ----------------------

    #[test]
    fn lua_api_all_registered_sub_tables_are_reachable() {
        let lua = full_lua_env();

        // The full set of sub-tables registered across all helper modules.
        let sub_tables = [
            // from helpers.rs
            "file", "system", "log", "env", "util",
            // from dedicated helper modules
            "hardware", "package", "service", "power", "security", "desktop", "boot", "network",
            "audio", "storage",
        ];

        for key in &sub_tables {
            let is_table: bool = lua
                .load(format!("return type(mdots.{}) == 'table'", key))
                .eval()
                .unwrap_or_else(|e| {
                    panic!(
                        "mdots.{} raised an error (sub-table may be nil): {}",
                        key, e
                    )
                });
            assert!(
                is_table,
                "mdots.{} is not a table — sub-table was not registered",
                key
            );
        }
    }

    // --- Test: a Lua module that probes every sub-table loads cleanly --------
    //
    // `validate_lua_module_detailed` exercises the full load + eval path,
    // which is identical to what a real user module goes through.  Any
    // "attempt to index a nil value" error in the probe script means a
    // sub-table is missing and would break every manifest that uses it.

    #[test]
    fn lua_api_golden_module_has_no_nil_sub_table_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let module_path = tmp.path().join("golden.lua");

        std::fs::write(
            &module_path,
            r#"
-- Lua API golden: access every registered mdots.* sub-table.
-- Lua raises "attempt to index a nil value" if any is missing.
local function probe(tbl, name)
    assert(type(tbl) == 'table', 'mdots.' .. name .. ' is not a table')
end

probe(mdots.file,     "file")
probe(mdots.system,   "system")
probe(mdots.log,      "log")
probe(mdots.env,      "env")
probe(mdots.util,     "util")
probe(mdots.hardware, "hardware")
probe(mdots.package,  "package")
probe(mdots.service,  "service")
probe(mdots.power,    "power")
probe(mdots.security, "security")
probe(mdots.desktop,  "desktop")
probe(mdots.boot,     "boot")
probe(mdots.network,  "network")
probe(mdots.audio,    "audio")
probe(mdots.storage,  "storage")

return {
    description = "Lua API golden test module",
    packages = { "test-package" },
}
"#,
        )
        .unwrap();

        let result = validate_lua_module_detailed(&module_path);

        // Any "attempt to index a nil value" means a sub-table is missing.
        let nil_errors: Vec<&str> = result
            .errors
            .iter()
            .map(|e| e.message.as_str())
            .filter(|m| m.contains("attempt to index a nil value"))
            .collect();
        assert!(
            nil_errors.is_empty(),
            "Lua API surface has nil sub-table(s) — the global-rename regression guard failed: {:?}",
            nil_errors
        );

        // The module must load without any hard errors.
        let all_errors: Vec<&str> = result.errors.iter().map(|e| e.message.as_str()).collect();
        assert!(
            result.valid,
            "Lua API golden module must be valid, errors: {:?}",
            all_errors
        );
    }
}
