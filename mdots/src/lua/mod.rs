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

// Loading/validating Lua modules, directory-module manifests, host
// configs and `source.lua`; the Lua API tables are the modules above.
mod config_validate;
mod host_config;
mod manifest;
mod module_load;
mod module_validate;
mod source;

use host_config::extract_packages_optional;
pub use host_config::{detect_pointer_lua_config, load_lua_config};
pub use manifest::load_lua_directory_module;
use manifest::lua_to_json;
use module_load::extract_services;
pub use module_load::load_lua_module;
use module_validate::suggest_field_name;
pub use module_validate::validate_lua_module_detailed;
pub use source::load_lua_source;

use anyhow::{Context, Result, anyhow};
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

// Lua error diagnostics (parse_lua_error + its helpers) live in `error_hints.rs`.
mod error_hints;
use error_hints::parse_lua_error;

/// Create a sandboxed Lua environment
pub(crate) fn create_sandboxed_lua() -> Result<Lua> {
    let lua = Lua::new();
    sandbox::apply_sandbox(&lua)?;
    Ok(lua)
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
