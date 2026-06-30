//! Storage device detection helpers for Lua modules
//!
//! Provides the `dcli.storage.*` API for detecting storage devices and filesystems.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};
use std::fs;
use std::process::Command;

/// Register storage detection helpers
pub fn register_storage_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    let dcli: Table = globals
        .get("dcli")
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    let storage = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.has_ssd() -> boolean
    storage
        .set(
            "has_ssd",
            lua.create_function(|_, ()| Ok(has_ssd()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.has_hdd() -> boolean
    storage
        .set(
            "has_hdd",
            lua.create_function(|_, ()| Ok(has_hdd()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.has_nvme() -> boolean
    storage
        .set(
            "has_nvme",
            lua.create_function(|_, ()| Ok(has_nvme()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.list_disks() -> array of disk names
    storage
        .set(
            "list_disks",
            lua.create_function(|lua, ()| {
                let disks = list_disks();
                let table = lua.create_table()?;
                for (i, disk) in disks.iter().enumerate() {
                    table.set(i + 1, disk.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.disk_type(name) -> "ssd" | "hdd" | "nvme" | "unknown"
    storage
        .set(
            "disk_type",
            lua.create_function(|_, name: String| Ok(get_disk_type(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.disk_size(name) -> size in bytes or nil
    storage
        .set(
            "disk_size",
            lua.create_function(|_, name: String| Ok(get_disk_size(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.filesystem(path) -> filesystem type or nil
    storage
        .set(
            "filesystem",
            lua.create_function(|_, path: String| Ok(get_filesystem_type(&path)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.mount_point(device) -> mount point or nil
    storage
        .set(
            "mount_point",
            lua.create_function(|_, device: String| Ok(get_mount_point(&device)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.is_mounted(device) -> boolean
    storage
        .set(
            "is_mounted",
            lua.create_function(|_, device: String| Ok(is_mounted(&device)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.free_space(path) -> free space in bytes or nil
    storage
        .set(
            "free_space",
            lua.create_function(|_, path: String| Ok(get_free_space(&path)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.total_space(path) -> total space in bytes or nil
    storage
        .set(
            "total_space",
            lua.create_function(|_, path: String| Ok(get_total_space(&path)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.has_swap() -> boolean
    storage
        .set(
            "has_swap",
            lua.create_function(|_, ()| Ok(has_swap()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.storage.swap_size() -> swap size in bytes or nil
    storage
        .set(
            "swap_size",
            lua.create_function(|_, ()| Ok(get_swap_size()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    dcli.set("storage", storage)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Check if system has any SSD
fn has_ssd() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/block") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_real_disk(&name) && get_disk_type(&name) == "ssd" {
                return true;
            }
        }
    }
    false
}

/// Check if system has any HDD
fn has_hdd() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/block") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_real_disk(&name) && get_disk_type(&name) == "hdd" {
                return true;
            }
        }
    }
    false
}

/// Check if system has any NVMe drive
fn has_nvme() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/block") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("nvme") {
                return true;
            }
        }
    }
    false
}

/// List all disk devices
fn list_disks() -> Vec<String> {
    let mut disks = Vec::new();

    if let Ok(entries) = fs::read_dir("/sys/block") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_real_disk(&name) {
                disks.push(name);
            }
        }
    }

    disks
}

/// Check if a block device is a real disk (not loop, ram, etc.)
fn is_real_disk(name: &str) -> bool {
    // Filter out virtual/temporary devices
    !name.starts_with("loop") &&
    !name.starts_with("ram") &&
    !name.starts_with("dm-") &&
    !name.starts_with("sr") && // CD-ROM
    !name.starts_with("zram")
}

/// Get disk type (SSD, HDD, NVMe)
fn get_disk_type(name: &str) -> String {
    // NVMe drives
    if name.starts_with("nvme") {
        return "nvme".to_string();
    }

    // Check rotational flag (0 = SSD, 1 = HDD)
    let rotational_path = format!("/sys/block/{}/queue/rotational", name);
    if let Ok(content) = fs::read_to_string(&rotational_path) {
        return match content.trim() {
            "0" => "ssd".to_string(),
            "1" => "hdd".to_string(),
            _ => "unknown".to_string(),
        };
    }

    "unknown".to_string()
}

/// Get disk size in bytes
fn get_disk_size(name: &str) -> Option<u64> {
    let size_path = format!("/sys/block/{}/size", name);
    if let Ok(content) = fs::read_to_string(&size_path) {
        if let Ok(sectors) = content.trim().parse::<u64>() {
            // Size is in 512-byte sectors
            return Some(sectors * 512);
        }
    }
    None
}

/// Get filesystem type for a path
fn get_filesystem_type(path: &str) -> Option<String> {
    // Use df to get filesystem type
    if let Ok(output) = Command::new("df").args(["-T", path]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Skip header line
        if let Some(line) = stdout.lines().nth(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
        }
    }

    // Alternative: read /proc/mounts
    if let Ok(content) = fs::read_to_string("/proc/mounts") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 && parts[1] == path {
                return Some(parts[2].to_string());
            }
        }
    }

    None
}

/// Get mount point for a device
fn get_mount_point(device: &str) -> Option<String> {
    // Ensure device path
    let dev_path = if device.starts_with("/dev/") {
        device.to_string()
    } else {
        format!("/dev/{}", device)
    };

    // Read /proc/mounts
    if let Ok(content) = fs::read_to_string("/proc/mounts") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[0] == dev_path {
                return Some(parts[1].to_string());
            }
        }
    }

    None
}

/// Check if a device is mounted
fn is_mounted(device: &str) -> bool {
    get_mount_point(device).is_some()
}

/// Get free space in bytes for a path
fn get_free_space(path: &str) -> Option<u64> {
    if let Ok(output) = Command::new("df").args(["-B1", path]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Skip header line
        if let Some(line) = stdout.lines().nth(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // Format: Filesystem 1B-blocks Used Available Use% Mounted
            if parts.len() >= 4 {
                if let Ok(free) = parts[3].parse::<u64>() {
                    return Some(free);
                }
            }
        }
    }
    None
}

/// Get total space in bytes for a path
fn get_total_space(path: &str) -> Option<u64> {
    if let Ok(output) = Command::new("df").args(["-B1", path]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Skip header line
        if let Some(line) = stdout.lines().nth(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // Format: Filesystem 1B-blocks Used Available Use% Mounted
            if parts.len() >= 2 {
                if let Ok(total) = parts[1].parse::<u64>() {
                    return Some(total);
                }
            }
        }
    }
    None
}

/// Check if swap is enabled
fn has_swap() -> bool {
    if let Ok(content) = fs::read_to_string("/proc/swaps") {
        // More than just header line means swap is active
        return content.lines().count() > 1;
    }
    false
}

/// Get total swap size in bytes
fn get_swap_size() -> Option<u64> {
    if let Ok(content) = fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("SwapTotal:") {
                // Format: "SwapTotal:       8388604 kB"
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if let Some(kb_str) = parts.first() {
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        return Some(kb * 1024); // Convert to bytes
                    }
                }
            }
        }
    }
    None
}
