//! Systemd service detection helpers for Lua modules
//!
//! Provides the `mdots.service.*` API for querying systemd services.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};
use std::process::Command;

/// Register service detection helpers
pub fn register_service_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    let mdots: Table = globals
        .get("mdots")
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    let service = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.service.is_enabled(name) -> boolean
    service
        .set(
            "is_enabled",
            lua.create_function(|_, name: String| Ok(is_service_enabled(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.service.is_active(name) -> boolean
    service
        .set(
            "is_active",
            lua.create_function(|_, name: String| Ok(is_service_active(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.service.is_running(name) -> boolean (alias for is_active)
    service
        .set(
            "is_running",
            lua.create_function(|_, name: String| Ok(is_service_active(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.service.exists(name) -> boolean
    service
        .set(
            "exists",
            lua.create_function(|_, name: String| Ok(service_exists(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.service.status(name) -> "active" | "inactive" | "failed" | "unknown"
    service
        .set(
            "status",
            lua.create_function(|_, name: String| Ok(get_service_status(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.service.list_enabled() -> array of enabled service names
    service
        .set(
            "list_enabled",
            lua.create_function(|lua, ()| {
                let services = list_enabled_services();
                let table = lua.create_table()?;
                for (i, svc) in services.iter().enumerate() {
                    table.set(i + 1, svc.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.service.list_active() -> array of active service names
    service
        .set(
            "list_active",
            lua.create_function(|lua, ()| {
                let services = list_active_services();
                let table = lua.create_table()?;
                for (i, svc) in services.iter().enumerate() {
                    table.set(i + 1, svc.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.service.list_failed() -> array of failed service names
    service
        .set(
            "list_failed",
            lua.create_function(|lua, ()| {
                let services = list_failed_services();
                let table = lua.create_table()?;
                for (i, svc) in services.iter().enumerate() {
                    table.set(i + 1, svc.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.service.is_user_service(name) -> boolean
    service
        .set(
            "is_user_service",
            lua.create_function(|_, name: String| Ok(is_user_service_active(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    mdots
        .set("service", service)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Check if a systemd service is enabled
fn is_service_enabled(name: &str) -> bool {
    Command::new("systemctl")
        .args(["is-enabled", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if a systemd service is active/running
fn is_service_active(name: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if a service unit file exists
fn service_exists(name: &str) -> bool {
    Command::new("systemctl")
        .args(["status", name])
        .output()
        .map(|o| {
            let stderr = String::from_utf8_lossy(&o.stderr);
            !stderr.contains("could not be found") && !stderr.contains("Unit") || o.status.success()
        })
        .unwrap_or(false)
}

/// Get the status of a service
fn get_service_status(name: &str) -> String {
    let output = match Command::new("systemctl").args(["is-active", name]).output() {
        Ok(o) => o,
        Err(_) => return "unknown".to_string(),
    };

    let status = String::from_utf8_lossy(&output.stdout).trim().to_string();

    match status.as_str() {
        "active" => "active".to_string(),
        "inactive" => "inactive".to_string(),
        "failed" => "failed".to_string(),
        "activating" => "activating".to_string(),
        "deactivating" => "deactivating".to_string(),
        _ => "unknown".to_string(),
    }
}

/// List all enabled services
fn list_enabled_services() -> Vec<String> {
    let output = match Command::new("systemctl")
        .args([
            "list-unit-files",
            "--state=enabled",
            "--type=service",
            "--no-pager",
            "--no-legend",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            parts.first().map(|s| s.to_string())
        })
        .collect()
}

/// List all active services
fn list_active_services() -> Vec<String> {
    let output = match Command::new("systemctl")
        .args([
            "list-units",
            "--state=active",
            "--type=service",
            "--no-pager",
            "--no-legend",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            parts.first().map(|s| s.to_string())
        })
        .collect()
}

/// List all failed services
fn list_failed_services() -> Vec<String> {
    let output = match Command::new("systemctl")
        .args([
            "list-units",
            "--state=failed",
            "--type=service",
            "--no-pager",
            "--no-legend",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            parts.first().map(|s| s.to_string())
        })
        .collect()
}

/// Check if a user service is active
fn is_user_service_active(name: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
