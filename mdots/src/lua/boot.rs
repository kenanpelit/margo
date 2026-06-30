//! Bootloader detection helpers for Lua modules
//!
//! Provides the `dcli.boot.*` API for detecting bootloader configuration.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Register bootloader detection helpers
pub fn register_boot_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    let dcli: Table = globals
        .get("dcli")
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    let boot = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.boot.bootloader() -> "grub" | "systemd-boot" | "refind" | "unknown"
    boot.set(
        "bootloader",
        lua.create_function(|_, ()| Ok(detect_bootloader()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.boot.is_uefi() -> boolean
    boot.set(
        "is_uefi",
        lua.create_function(|_, ()| Ok(is_uefi()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.boot.is_bios() -> boolean
    boot.set(
        "is_bios",
        lua.create_function(|_, ()| Ok(!is_uefi()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.boot.init_system() -> "systemd" | "openrc" | "runit" | "unknown"
    boot.set(
        "init_system",
        lua.create_function(|_, ()| Ok(detect_init_system()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.boot.kernel_params() -> array of kernel parameters
    boot.set(
        "kernel_params",
        lua.create_function(|lua, ()| {
            let params = get_kernel_params();
            let table = lua.create_table()?;
            for (i, param) in params.iter().enumerate() {
                table.set(i + 1, param.clone())?;
            }
            Ok(table)
        })
        .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.boot.has_kernel_param(param) -> boolean
    boot.set(
        "has_kernel_param",
        lua.create_function(|_, param: String| Ok(has_kernel_param(&param)))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.boot.efi_vars_supported() -> boolean
    boot.set(
        "efi_vars_supported",
        lua.create_function(|_, ()| Ok(efi_vars_supported()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    // dcli.boot.boot_id() -> boot ID string
    boot.set(
        "boot_id",
        lua.create_function(|_, ()| Ok(get_boot_id()))
            .map_err(|e| anyhow!("Lua error: {}", e))?,
    )
    .map_err(|e| anyhow!("Lua error: {}", e))?;

    dcli.set("boot", boot)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Detect which bootloader is being used
fn detect_bootloader() -> String {
    // Check for GRUB
    if Path::new("/boot/grub").exists() || Path::new("/boot/grub2").exists() {
        return "grub".to_string();
    }

    // Check for systemd-boot
    if Path::new("/boot/loader/loader.conf").exists()
        || Path::new("/efi/loader/loader.conf").exists()
        || Path::new("/boot/efi/loader/loader.conf").exists()
    {
        return "systemd-boot".to_string();
    }

    // Check for rEFInd
    if Path::new("/boot/efi/EFI/refind").exists() || Path::new("/boot/EFI/refind").exists() {
        return "refind".to_string();
    }

    // Check for LILO (legacy)
    if Path::new("/etc/lilo.conf").exists() {
        return "lilo".to_string();
    }

    // Check for syslinux
    if Path::new("/boot/syslinux").exists() {
        return "syslinux".to_string();
    }

    // Use efibootmgr to check UEFI boot entries
    if let Ok(output) = Command::new("efibootmgr").output() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
        if stdout.contains("grub") {
            return "grub".to_string();
        }
        if stdout.contains("systemd") {
            return "systemd-boot".to_string();
        }
        if stdout.contains("refind") {
            return "refind".to_string();
        }
    }

    "unknown".to_string()
}

/// Check if system is booted in UEFI mode
fn is_uefi() -> bool {
    Path::new("/sys/firmware/efi").exists()
}

/// Detect init system
fn detect_init_system() -> String {
    // Check for systemd
    if Path::new("/run/systemd/system").exists() {
        return "systemd".to_string();
    }

    // Check for OpenRC
    if Path::new("/run/openrc").exists() || Path::new("/etc/init.d/functions.sh").exists() {
        return "openrc".to_string();
    }

    // Check for runit
    if Path::new("/run/runit").exists() || Path::new("/etc/runit").exists() {
        return "runit".to_string();
    }

    // Check for s6
    if Path::new("/run/s6").exists() {
        return "s6".to_string();
    }

    // Check /proc/1/comm as fallback
    if let Ok(init) = fs::read_to_string("/proc/1/comm") {
        let init = init.trim();
        if init == "systemd" {
            return "systemd".to_string();
        }
        if init == "init" {
            // Try to determine which init
            if Path::new("/etc/inittab").exists() {
                return "sysvinit".to_string();
            }
        }
        return init.to_string();
    }

    "unknown".to_string()
}

/// Get kernel boot parameters
fn get_kernel_params() -> Vec<String> {
    if let Ok(cmdline) = fs::read_to_string("/proc/cmdline") {
        return cmdline.split_whitespace().map(|s| s.to_string()).collect();
    }
    Vec::new()
}

/// Check if a specific kernel parameter exists
fn has_kernel_param(param: &str) -> bool {
    if let Ok(cmdline) = fs::read_to_string("/proc/cmdline") {
        for p in cmdline.split_whitespace() {
            // Handle both "param" and "param=value" forms
            if p == param || p.starts_with(&format!("{}=", param)) {
                return true;
            }
        }
    }
    false
}

/// Check if EFI variables are supported
fn efi_vars_supported() -> bool {
    Path::new("/sys/firmware/efi/efivars").exists()
        && fs::read_dir("/sys/firmware/efi/efivars")
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false)
}

/// Get boot ID (unique ID for each boot)
fn get_boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}
