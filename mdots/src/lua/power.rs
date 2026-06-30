//! Power management detection helpers for Lua modules
//!
//! Provides the `mdots.power.*` API for querying power management features.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};
use std::fs;
use std::path::Path;

/// Register power management helpers
pub fn register_power_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    let mdots: Table = globals
        .get("mdots")
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    let power = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.power.on_battery() -> boolean
    power
        .set(
            "on_battery",
            lua.create_function(|_, ()| Ok(is_on_battery()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.power.on_ac() -> boolean
    power
        .set(
            "on_ac",
            lua.create_function(|_, ()| Ok(is_on_ac()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.power.battery_percent() -> number or nil
    power
        .set(
            "battery_percent",
            lua.create_function(|_, ()| Ok(get_battery_percent()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.power.battery_status() -> "charging" | "discharging" | "full" | "unknown"
    power
        .set(
            "battery_status",
            lua.create_function(|_, ()| Ok(get_battery_status()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.power.has_suspend() -> boolean
    power
        .set(
            "has_suspend",
            lua.create_function(|_, ()| Ok(has_suspend()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.power.has_hibernate() -> boolean
    power
        .set(
            "has_hibernate",
            lua.create_function(|_, ()| Ok(has_hibernate()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.power.cpu_governor() -> "performance" | "powersave" | "schedutil" | etc.
    power
        .set(
            "cpu_governor",
            lua.create_function(|_, ()| Ok(get_cpu_governor()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.power.available_governors() -> array of governor names
    power
        .set(
            "available_governors",
            lua.create_function(|lua, ()| {
                let governors = get_available_governors();
                let table = lua.create_table()?;
                for (i, gov) in governors.iter().enumerate() {
                    table.set(i + 1, gov.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.power.supports_turbo() -> boolean
    power
        .set(
            "supports_turbo",
            lua.create_function(|_, ()| Ok(supports_turbo_boost()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.power.turbo_enabled() -> boolean
    power
        .set(
            "turbo_enabled",
            lua.create_function(|_, ()| Ok(is_turbo_enabled()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    mdots.set("power", power)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Check if running on battery power
fn is_on_battery() -> bool {
    // Check for battery that is discharging
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.filter_map(|e| e.ok()) {
            let type_path = entry.path().join("type");
            let status_path = entry.path().join("status");

            if let Ok(supply_type) = fs::read_to_string(&type_path) {
                if supply_type.trim() == "Battery" {
                    if let Ok(status) = fs::read_to_string(&status_path) {
                        if status.trim() == "Discharging" {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Check if running on AC power
fn is_on_ac() -> bool {
    // Check for AC adapter that is online
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.filter_map(|e| e.ok()) {
            let type_path = entry.path().join("type");
            let online_path = entry.path().join("online");

            if let Ok(supply_type) = fs::read_to_string(&type_path) {
                if supply_type.trim() == "Mains" {
                    if let Ok(online) = fs::read_to_string(&online_path) {
                        if online.trim() == "1" {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Get battery percentage (0-100)
fn get_battery_percent() -> Option<u8> {
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.filter_map(|e| e.ok()) {
            let type_path = entry.path().join("type");
            let capacity_path = entry.path().join("capacity");

            if let Ok(supply_type) = fs::read_to_string(&type_path) {
                if supply_type.trim() == "Battery" {
                    if let Ok(capacity) = fs::read_to_string(&capacity_path) {
                        if let Ok(percent) = capacity.trim().parse::<u8>() {
                            return Some(percent);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Get battery charging status
fn get_battery_status() -> String {
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.filter_map(|e| e.ok()) {
            let type_path = entry.path().join("type");
            let status_path = entry.path().join("status");

            if let Ok(supply_type) = fs::read_to_string(&type_path) {
                if supply_type.trim() == "Battery" {
                    if let Ok(status) = fs::read_to_string(&status_path) {
                        return status.trim().to_lowercase();
                    }
                }
            }
        }
    }
    "unknown".to_string()
}

/// Check if system supports suspend
fn has_suspend() -> bool {
    Path::new("/sys/power/state").exists()
        && fs::read_to_string("/sys/power/state")
            .map(|s| s.contains("mem"))
            .unwrap_or(false)
}

/// Check if system supports hibernate
fn has_hibernate() -> bool {
    Path::new("/sys/power/state").exists()
        && fs::read_to_string("/sys/power/state")
            .map(|s| s.contains("disk"))
            .unwrap_or(false)
}

/// Get current CPU governor
fn get_cpu_governor() -> String {
    // Check first CPU's governor
    let governor_path = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor";
    fs::read_to_string(governor_path)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Get list of available CPU governors
fn get_available_governors() -> Vec<String> {
    let governors_path = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors";
    fs::read_to_string(governors_path)
        .map(|s| s.split_whitespace().map(|g| g.to_string()).collect())
        .unwrap_or_else(|_| Vec::new())
}

/// Check if CPU supports turbo boost
fn supports_turbo_boost() -> bool {
    // Intel turbo boost
    if Path::new("/sys/devices/system/cpu/intel_pstate/no_turbo").exists() {
        return true;
    }
    // AMD boost
    if Path::new("/sys/devices/system/cpu/cpufreq/boost").exists() {
        return true;
    }
    false
}

/// Check if turbo boost is enabled
fn is_turbo_enabled() -> bool {
    // Intel: no_turbo = 0 means enabled
    if let Ok(content) = fs::read_to_string("/sys/devices/system/cpu/intel_pstate/no_turbo") {
        return content.trim() == "0";
    }
    // AMD: boost = 1 means enabled
    if let Ok(content) = fs::read_to_string("/sys/devices/system/cpu/cpufreq/boost") {
        return content.trim() == "1";
    }
    false
}
