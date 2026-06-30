//! Lua sandboxing for security
//!
//! Restricts Lua capabilities to prevent arbitrary code execution
//! while allowing necessary hardware detection operations.

use anyhow::{anyhow, Result};
use mlua::{Lua, Value};

/// Apply sandbox restrictions to Lua environment
pub fn apply_sandbox(lua: &Lua) -> Result<()> {
    let globals = lua.globals();

    // Remove dangerous functions
    globals
        .set("os", Value::Nil)
        .map_err(|e| anyhow!("Lua error: {}", e))?; // Remove os.execute, etc.
    globals
        .set("io", Value::Nil)
        .map_err(|e| anyhow!("Lua error: {}", e))?; // Remove io.popen, etc. (we'll add safe versions)
    globals
        .set("loadfile", Value::Nil)
        .map_err(|e| anyhow!("Lua error: {}", e))?; // Prevent arbitrary file loading
    globals
        .set("dofile", Value::Nil)
        .map_err(|e| anyhow!("Lua error: {}", e))?; // Prevent arbitrary file execution
    globals
        .set("load", Value::Nil)
        .map_err(|e| anyhow!("Lua error: {}", e))?; // Prevent dynamic code loading

    // Keep safe standard libraries
    // - string, table, math, utf8 are safe
    // - coroutine is safe but not useful here

    Ok(())
}

/// List of paths that are safe to read
pub const SAFE_READ_PATHS: &[&str] = &[
    "/sys/",
    "/proc/",
    "/etc/os-release",
    "/etc/hostname",
    "/etc/machine-id",
];

/// Check if a path is safe to read
pub fn is_safe_path(path: &str) -> bool {
    SAFE_READ_PATHS.iter().any(|safe| path.starts_with(safe))
}
