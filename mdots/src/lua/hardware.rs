//! Hardware detection helpers for Lua modules
//!
//! Provides the `mdots.hardware.*` API for detecting system hardware.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};
use std::fs;

/// Register hardware detection helpers
pub fn register_hardware_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    let mdots: Table = globals
        .get("mdots")
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    let hardware = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.hardware.cpu_vendor() -> "intel" | "amd" | "unknown"
    hardware
        .set(
            "cpu_vendor",
            lua.create_function(|_, ()| Ok(detect_cpu_vendor()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.hardware.gpu_vendors() -> {"nvidia", "amd", "intel"}
    hardware
        .set(
            "gpu_vendors",
            lua.create_function(|lua, ()| {
                let vendors = detect_gpu_vendors();
                let table = lua.create_table()?;
                for (i, vendor) in vendors.iter().enumerate() {
                    table.set(i + 1, vendor.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.hardware.has_nvidia() -> boolean
    hardware
        .set(
            "has_nvidia",
            lua.create_function(|_, ()| Ok(detect_gpu_vendors().contains(&"nvidia".to_string())))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.hardware.has_amd_gpu() -> boolean
    hardware
        .set(
            "has_amd_gpu",
            lua.create_function(|_, ()| Ok(detect_gpu_vendors().contains(&"amd".to_string())))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.hardware.has_intel_gpu() -> boolean
    hardware
        .set(
            "has_intel_gpu",
            lua.create_function(|_, ()| Ok(detect_gpu_vendors().contains(&"intel".to_string())))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.hardware.is_laptop() -> boolean
    hardware
        .set(
            "is_laptop",
            lua.create_function(|_, ()| Ok(detect_is_laptop()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.hardware.has_battery() -> boolean
    hardware
        .set(
            "has_battery",
            lua.create_function(|_, ()| Ok(detect_has_battery()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.hardware.chassis_type() -> "desktop" | "laptop" | "server" | "unknown"
    hardware
        .set(
            "chassis_type",
            lua.create_function(|_, ()| Ok(detect_chassis_type()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    mdots
        .set("hardware", hardware)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Detect CPU vendor (intel or amd)
fn detect_cpu_vendor() -> String {
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if line.starts_with("vendor_id") {
                if line.contains("GenuineIntel") {
                    return "intel".to_string();
                } else if line.contains("AuthenticAMD") {
                    return "amd".to_string();
                }
            }
        }
    }
    "unknown".to_string()
}

/// Detect GPU vendors present in system
fn detect_gpu_vendors() -> Vec<String> {
    let mut vendors = Vec::new();

    // Check /sys/bus/pci/devices for VGA controllers
    if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
        for entry in entries.filter_map(|e| e.ok()) {
            let class_path = entry.path().join("class");
            let vendor_path = entry.path().join("vendor");

            // Check if it's a VGA controller (class 0x03xxxx)
            if let Ok(class) = fs::read_to_string(&class_path) {
                if class.trim().starts_with("0x03") {
                    if let Ok(vendor) = fs::read_to_string(&vendor_path) {
                        let vendor = vendor.trim();
                        match vendor {
                            "0x10de" if !vendors.contains(&"nvidia".to_string()) => {
                                vendors.push("nvidia".to_string());
                            }
                            "0x1002" if !vendors.contains(&"amd".to_string()) => {
                                vendors.push("amd".to_string());
                            }
                            "0x8086" if !vendors.contains(&"intel".to_string()) => {
                                vendors.push("intel".to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    vendors
}

/// Detect if running on a laptop
fn detect_is_laptop() -> bool {
    // Method 1: Check DMI chassis type (most reliable)
    if let Ok(chassis) = fs::read_to_string("/sys/class/dmi/id/chassis_type") {
        let chassis_type: u32 = chassis.trim().parse().unwrap_or(0);

        // Laptop chassis types: 8, 9, 10, 11, 14
        if matches!(chassis_type, 8 | 9 | 10 | 11 | 14) {
            return true;
        }

        // If it's explicitly a non-laptop chassis type (desktop, server, etc.),
        // trust it and don't fall through to battery/lid checks
        // This prevents false positives from UPS/battery on desktop systems
        if matches!(
            chassis_type,
            3 | 4 | 5 | 6 | 7 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 | 25
        ) {
            return false;
        }

        // Chassis types 1 (other) and 2 (unknown) fall through to secondary methods
    }

    // Method 2: Check for battery (fallback for unknown chassis types)
    if detect_has_battery() {
        return true;
    }

    // Method 3: Check for lid switch (fallback for unknown chassis types)
    if std::path::Path::new("/proc/acpi/button/lid").exists() {
        return true;
    }

    false
}

/// Detect if system has a battery
fn detect_has_battery() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.filter_map(|e| e.ok()) {
            let type_path = entry.path().join("type");
            if let Ok(supply_type) = fs::read_to_string(&type_path) {
                if supply_type.trim() == "Battery" {
                    return true;
                }
            }
        }
    }
    false
}

/// Detect chassis type as string
fn detect_chassis_type() -> String {
    if let Ok(chassis) = fs::read_to_string("/sys/class/dmi/id/chassis_type") {
        let chassis_type: u32 = chassis.trim().parse().unwrap_or(0);
        match chassis_type {
            1 => "other",
            2 => "unknown",
            3 | 4 | 5 | 6 | 7 | 15 | 16 => "desktop",
            8 | 9 | 10 | 11 | 14 => "laptop",
            17..=25 => "server",
            30..=32 => "tablet",
            _ => "unknown",
        }
        .to_string()
    } else {
        "unknown".to_string()
    }
}
