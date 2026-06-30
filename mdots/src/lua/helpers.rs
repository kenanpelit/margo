//! dcli helper functions exposed to Lua
//!
//! Provides the `dcli.*` API for Lua modules.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table, Value};

use super::sandbox;

/// Register all dcli helpers in Lua globals
pub fn register_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();

    // Create dcli table
    let dcli = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // Add sub-tables
    dcli.set("file", create_file_helpers(lua)?)
        .map_err(|e| anyhow!("Lua error: {}", e))?;
    dcli.set("system", create_system_helpers(lua)?)
        .map_err(|e| anyhow!("Lua error: {}", e))?;
    dcli.set("log", create_log_helpers(lua)?)
        .map_err(|e| anyhow!("Lua error: {}", e))?;
    dcli.set("env", create_env_helpers(lua)?)
        .map_err(|e| anyhow!("Lua error: {}", e))?;
    dcli.set("util", create_util_helpers(lua)?)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    globals
        .set("dcli", dcli)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Register dcli helpers with silent/no-op log functions
/// Used during config detection to avoid duplicate log output
pub fn register_helpers_silent(lua: &Lua) -> Result<()> {
    let globals = lua.globals();

    // Create dcli table
    let dcli = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // Add sub-tables
    dcli.set("file", create_file_helpers(lua)?)
        .map_err(|e| anyhow!("Lua error: {}", e))?;
    dcli.set("system", create_system_helpers(lua)?)
        .map_err(|e| anyhow!("Lua error: {}", e))?;
    dcli.set("log", create_silent_log_helpers(lua)?)
        .map_err(|e| anyhow!("Lua error: {}", e))?;
    dcli.set("env", create_env_helpers(lua)?)
        .map_err(|e| anyhow!("Lua error: {}", e))?;
    dcli.set("util", create_util_helpers(lua)?)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    globals
        .set("dcli", dcli)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Create dcli.file helpers
fn create_file_helpers(lua: &Lua) -> Result<Table> {
    let file = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.file.exists(path)
    file.set(
        "exists",
        lua.create_function(|_, path: String| Ok(std::path::Path::new(&path).exists()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.file.is_dir(path)
    file.set(
        "is_dir",
        lua.create_function(|_, path: String| Ok(std::path::Path::new(&path).is_dir()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.file.is_file(path)
    file.set(
        "is_file",
        lua.create_function(|_, path: String| Ok(std::path::Path::new(&path).is_file()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.file.read(path) - sandboxed
    file.set(
        "read",
        lua.create_function(|_, path: String| {
            if !sandbox::is_safe_path(&path) {
                return Err(mlua::Error::RuntimeError(format!(
                    "Access denied: {} is not in safe path list",
                    path
                )));
            }

            match std::fs::read_to_string(&path) {
                Ok(content) => Ok(Some(content)),
                Err(_) => Ok(None),
            }
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.file.read_lines(path) - returns array of lines
    file.set(
        "read_lines",
        lua.create_function(|lua, path: String| {
            if !sandbox::is_safe_path(&path) {
                return Err(mlua::Error::RuntimeError(format!(
                    "Access denied: {} is not in safe path list",
                    path
                )));
            }

            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
                    let table = lua.create_table()?;
                    for (i, line) in lines.iter().enumerate() {
                        table.set(i + 1, line.clone())?;
                    }
                    Ok(Value::Table(table))
                }
                Err(_) => Ok(Value::Nil),
            }
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(file)
}

/// Create dcli.system helpers
fn create_system_helpers(lua: &Lua) -> Result<Table> {
    let system = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.system.hostname()
    system
        .set(
            "hostname",
            lua.create_function(|_, ()| {
                Ok(hostname::get()
                    .ok()
                    .and_then(|h| h.into_string().ok())
                    .unwrap_or_else(|| "unknown".to_string()))
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.system.kernel_version()
    system
        .set(
            "kernel_version",
            lua.create_function(|_, ()| {
                match std::fs::read_to_string("/proc/version") {
                    Ok(content) => {
                        // Extract version from "Linux version X.Y.Z ..."
                        let version = content.split_whitespace().nth(2).unwrap_or("unknown");
                        Ok(version.to_string())
                    }
                    Err(_) => Ok("unknown".to_string()),
                }
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.system.arch() -> "x86_64" | "aarch64" | etc.
    system
        .set(
            "arch",
            lua.create_function(|_, ()| Ok(std::env::consts::ARCH.to_string()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.system.os() -> "linux" | "macos" | "windows"
    system
        .set(
            "os",
            lua.create_function(|_, ()| Ok(std::env::consts::OS.to_string()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.system.distro() -> "arch" | "endeavouros" | "manjaro" | etc.
    system
        .set(
            "distro",
            lua.create_function(|_, ()| Ok(get_distro_id()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.system.distro_name() -> "Arch Linux" | "EndeavourOS" | etc.
    system
        .set(
            "distro_name",
            lua.create_function(|_, ()| Ok(get_distro_name()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.system.distro_version() -> version string or "rolling"
    system
        .set(
            "distro_version",
            lua.create_function(|_, ()| Ok(get_distro_version()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.system.memory_total_mb() -> total RAM in MB
    system
        .set(
            "memory_total_mb",
            lua.create_function(|_, ()| Ok(get_memory_total_mb()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.system.cpu_cores() -> number of CPU cores
    system
        .set(
            "cpu_cores",
            lua.create_function(|_, ()| Ok(get_cpu_cores()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(system)
}

/// Create dcli.log helpers
fn create_log_helpers(lua: &Lua) -> Result<Table> {
    let log_table = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.log.info(msg)
    log_table
        .set(
            "info",
            lua.create_function(|_, msg: String| {
                log::info!("[lua] {}", msg);
                Ok(())
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.log.warn(msg)
    log_table
        .set(
            "warn",
            lua.create_function(|_, msg: String| {
                log::warn!("[lua] {}", msg);
                Ok(())
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.log.debug(msg)
    log_table
        .set(
            "debug",
            lua.create_function(|_, msg: String| {
                log::debug!("[lua] {}", msg);
                Ok(())
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.log.error(msg)
    log_table
        .set(
            "error",
            lua.create_function(|_, msg: String| {
                log::error!("[lua] {}", msg);
                Ok(())
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(log_table)
}

/// Create silent/no-op log helpers that don't output anything
/// Used during config detection to avoid duplicate log messages
fn create_silent_log_helpers(lua: &Lua) -> Result<Table> {
    let log_table = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // Silent versions of log functions - they accept the same args but do nothing
    log_table
        .set(
            "info",
            lua.create_function(|_, _: String| Ok(()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    log_table
        .set(
            "warn",
            lua.create_function(|_, _: String| Ok(()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    log_table
        .set(
            "debug",
            lua.create_function(|_, _: String| Ok(()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    log_table
        .set(
            "error",
            lua.create_function(|_, _: String| Ok(()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(log_table)
}

/// Create dcli.env helpers for environment variables
fn create_env_helpers(lua: &Lua) -> Result<Table> {
    let env = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.env.get(name) -> string or nil
    env.set(
        "get",
        lua.create_function(|_, name: String| Ok(std::env::var(&name).ok()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.env.home() -> home directory path
    env.set(
        "home",
        lua.create_function(|_, ()| {
            Ok(std::env::var("HOME")
                .ok()
                .or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().to_string()))
                .unwrap_or_else(|| "/home".to_string()))
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.env.user() -> current username
    env.set(
        "user",
        lua.create_function(|_, ()| {
            Ok(std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "unknown".to_string()))
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.env.config_dir() -> XDG config directory
    env.set(
        "config_dir",
        lua.create_function(|_, ()| {
            Ok(std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/home".to_string());
                format!("{}/.config", home)
            }))
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.env.data_dir() -> XDG data directory
    env.set(
        "data_dir",
        lua.create_function(|_, ()| {
            Ok(std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/home".to_string());
                format!("{}/.local/share", home)
            }))
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.env.cache_dir() -> XDG cache directory
    env.set(
        "cache_dir",
        lua.create_function(|_, ()| {
            Ok(std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/home".to_string());
                format!("{}/.cache", home)
            }))
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.env.shell() -> current shell
    env.set(
        "shell",
        lua.create_function(|_, ()| {
            Ok(std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()))
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(env)
}

/// Create dcli.util helpers for utility functions
fn create_util_helpers(lua: &Lua) -> Result<Table> {
    let util = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.contains(table, value) -> boolean
    // Check if an array-like table contains a value
    util.set(
        "contains",
        lua.create_function(|_, (table, value): (Table, Value)| {
            for (_, v) in table.pairs::<i64, Value>().flatten() {
                if values_equal(&v, &value) {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.keys(table) -> array of keys
    util.set(
        "keys",
        lua.create_function(|lua, table: Table| {
            let keys = lua.create_table()?;
            for (i, (k, _)) in table.pairs::<Value, Value>().flatten().enumerate() {
                keys.set(i + 1, k)?;
            }
            Ok(keys)
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.values(table) -> array of values
    util.set(
        "values",
        lua.create_function(|lua, table: Table| {
            let values = lua.create_table()?;
            for (i, (_, v)) in table.pairs::<Value, Value>().flatten().enumerate() {
                values.set(i + 1, v)?;
            }
            Ok(values)
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.merge(t1, t2) -> merged table (t2 values override t1)
    util.set(
        "merge",
        lua.create_function(|lua, (t1, t2): (Table, Table)| {
            let result = lua.create_table()?;

            // Copy from t1
            for (k, v) in t1.pairs::<Value, Value>().flatten() {
                result.set(k, v)?;
            }

            // Copy/override from t2
            for (k, v) in t2.pairs::<Value, Value>().flatten() {
                result.set(k, v)?;
            }

            Ok(result)
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.extend(target, source) -> extends target array with source values
    util.set(
        "extend",
        lua.create_function(|_, (target, source): (Table, Table)| {
            // Find the current length of target
            let mut max_idx: i64 = 0;
            for (idx, _) in target.clone().pairs::<i64, Value>().flatten() {
                max_idx = max_idx.max(idx);
            }

            // Append items from source
            for (_, v) in source.pairs::<i64, Value>().flatten() {
                max_idx += 1;
                target.set(max_idx, v)?;
            }

            Ok(target)
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.version_compare(v1, v2) -> -1, 0, or 1
    // Compares semantic version strings
    util.set(
        "version_compare",
        lua.create_function(|_, (v1, v2): (String, String)| Ok(compare_versions(&v1, &v2)))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.version_gte(v1, v2) -> boolean (v1 >= v2)
    util.set(
        "version_gte",
        lua.create_function(|_, (v1, v2): (String, String)| Ok(compare_versions(&v1, &v2) >= 0))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.version_gt(v1, v2) -> boolean (v1 > v2)
    util.set(
        "version_gt",
        lua.create_function(|_, (v1, v2): (String, String)| Ok(compare_versions(&v1, &v2) > 0))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.version_lte(v1, v2) -> boolean (v1 <= v2)
    util.set(
        "version_lte",
        lua.create_function(|_, (v1, v2): (String, String)| Ok(compare_versions(&v1, &v2) <= 0))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.version_lt(v1, v2) -> boolean (v1 < v2)
    util.set(
        "version_lt",
        lua.create_function(|_, (v1, v2): (String, String)| Ok(compare_versions(&v1, &v2) < 0))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.split(str, delimiter) -> array of strings
    util.set(
        "split",
        lua.create_function(|lua, (s, delim): (String, String)| {
            let parts: Vec<&str> = s.split(&delim).collect();
            let result = lua.create_table()?;
            for (i, part) in parts.iter().enumerate() {
                result.set(i + 1, *part)?;
            }
            Ok(result)
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.trim(str) -> trimmed string
    util.set(
        "trim",
        lua.create_function(|_, s: String| Ok(s.trim().to_string()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.starts_with(str, prefix) -> boolean
    util.set(
        "starts_with",
        lua.create_function(|_, (s, prefix): (String, String)| Ok(s.starts_with(&prefix)))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.util.ends_with(str, suffix) -> boolean
    util.set(
        "ends_with",
        lua.create_function(|_, (s, suffix): (String, String)| Ok(s.ends_with(&suffix)))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(util)
}

// ============================================================================
// Helper functions
// ============================================================================

/// Compare two Lua values for equality
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Boolean(a), Value::Boolean(b)) => a == b,
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => (a - b).abs() < f64::EPSILON,
        (Value::Integer(a), Value::Number(b)) | (Value::Number(b), Value::Integer(a)) => {
            ((*a as f64) - b).abs() < f64::EPSILON
        }
        (Value::String(a), Value::String(b)) => a.as_bytes() == b.as_bytes(),
        _ => false,
    }
}

/// Parse /etc/os-release and get the ID field
fn get_distro_id() -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(id) = line.strip_prefix("ID=") {
                return id.trim_matches('"').to_lowercase();
            }
        }
    }
    "unknown".to_string()
}

/// Parse /etc/os-release and get the NAME field
fn get_distro_name() -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(name) = line.strip_prefix("NAME=") {
                return name.trim_matches('"').to_string();
            }
        }
    }
    "Unknown".to_string()
}

/// Parse /etc/os-release and get the VERSION_ID field
fn get_distro_version() -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(version) = line.strip_prefix("VERSION_ID=") {
                return version.trim_matches('"').to_string();
            }
        }
    }
    // Arch and derivatives don't have VERSION_ID
    "rolling".to_string()
}

/// Get total system memory in MB
fn get_memory_total_mb() -> u64 {
    if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                // Format: "MemTotal:       16384000 kB"
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if let Some(kb_str) = parts.first() {
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        return kb / 1024; // Convert to MB
                    }
                }
            }
        }
    }
    0
}

/// Get number of CPU cores
fn get_cpu_cores() -> u32 {
    if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
        let count = content
            .lines()
            .filter(|line| line.starts_with("processor"))
            .count();
        return count as u32;
    }
    1
}

/// Compare two version strings
/// Returns -1 if v1 < v2, 0 if v1 == v2, 1 if v1 > v2
fn compare_versions(v1: &str, v2: &str) -> i32 {
    // Strip common prefixes like 'v'
    let v1 = v1.trim_start_matches('v').trim_start_matches('V');
    let v2 = v2.trim_start_matches('v').trim_start_matches('V');

    // Split by common version separators
    let parts1: Vec<&str> = v1.split(['.', '-', '_']).collect();
    let parts2: Vec<&str> = v2.split(['.', '-', '_']).collect();

    let max_len = parts1.len().max(parts2.len());

    for i in 0..max_len {
        let p1 = parts1.get(i).unwrap_or(&"0");
        let p2 = parts2.get(i).unwrap_or(&"0");

        // Try to parse as numbers first
        match (p1.parse::<u64>(), p2.parse::<u64>()) {
            (Ok(n1), Ok(n2)) => {
                if n1 < n2 {
                    return -1;
                } else if n1 > n2 {
                    return 1;
                }
            }
            _ => {
                // Fall back to string comparison
                match p1.cmp(p2) {
                    std::cmp::Ordering::Less => return -1,
                    std::cmp::Ordering::Greater => return 1,
                    std::cmp::Ordering::Equal => {}
                }
            }
        }
    }

    0
}
