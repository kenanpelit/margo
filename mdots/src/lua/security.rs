//! Security features detection helpers for Lua modules
//!
//! Provides the `mdots.security.*` API for detecting security features.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Register security detection helpers
pub fn register_security_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    let mdots: Table = globals
        .get("mdots")
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    let security = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.has_selinux() -> boolean
    security
        .set(
            "has_selinux",
            lua.create_function(|_, ()| Ok(has_selinux()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.selinux_enabled() -> boolean
    security
        .set(
            "selinux_enabled",
            lua.create_function(|_, ()| Ok(is_selinux_enabled()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.has_apparmor() -> boolean
    security
        .set(
            "has_apparmor",
            lua.create_function(|_, ()| Ok(has_apparmor()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.apparmor_enabled() -> boolean
    security
        .set(
            "apparmor_enabled",
            lua.create_function(|_, ()| Ok(is_apparmor_enabled()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.has_secureboot() -> boolean
    security
        .set(
            "has_secureboot",
            lua.create_function(|_, ()| Ok(has_secureboot()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.secureboot_enabled() -> boolean
    security
        .set(
            "secureboot_enabled",
            lua.create_function(|_, ()| Ok(is_secureboot_enabled()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.has_tpm() -> boolean
    security
        .set(
            "has_tpm",
            lua.create_function(|_, ()| Ok(has_tpm()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.tpm_version() -> "1.2" | "2.0" | nil
    security
        .set(
            "tpm_version",
            lua.create_function(|_, ()| Ok(get_tpm_version()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.firewall_active() -> boolean
    security
        .set(
            "firewall_active",
            lua.create_function(|_, ()| Ok(is_firewall_active()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.firewall_type() -> "ufw" | "firewalld" | "iptables" | "nftables" | "none"
    security
        .set(
            "firewall_type",
            lua.create_function(|_, ()| Ok(get_firewall_type()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.has_luks() -> boolean
    security
        .set(
            "has_luks",
            lua.create_function(|_, ()| Ok(has_luks_encryption()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.security.kernel_lockdown() -> "none" | "integrity" | "confidentiality"
    security
        .set(
            "kernel_lockdown",
            lua.create_function(|_, ()| Ok(get_kernel_lockdown()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    mdots.set("security", security)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Check if SELinux is available
fn has_selinux() -> bool {
    Path::new("/sys/fs/selinux").exists() || Path::new("/etc/selinux/config").exists()
}

/// Check if SELinux is enabled
fn is_selinux_enabled() -> bool {
    if let Ok(content) = fs::read_to_string("/sys/fs/selinux/enforce") {
        return content.trim() == "1";
    }

    Command::new("getenforce")
        .output()
        .map(|o| {
            let output = String::from_utf8_lossy(&o.stdout);
            output.trim().to_lowercase() == "enforcing"
        })
        .unwrap_or(false)
}

/// Check if AppArmor is available
fn has_apparmor() -> bool {
    Path::new("/sys/kernel/security/apparmor").exists()
        || Path::new("/sys/module/apparmor").exists()
}

/// Check if AppArmor is enabled
fn is_apparmor_enabled() -> bool {
    if let Ok(content) = fs::read_to_string("/sys/module/apparmor/parameters/enabled") {
        return content.trim() == "Y";
    }

    Command::new("aa-enabled")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if Secure Boot is supported
fn has_secureboot() -> bool {
    Path::new("/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c").exists()
}

/// Check if Secure Boot is enabled
fn is_secureboot_enabled() -> bool {
    if let Ok(data) =
        fs::read("/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c")
    {
        // Last byte indicates secure boot status: 1 = enabled
        return data.last() == Some(&1);
    }

    // Alternative method using mokutil
    Command::new("mokutil")
        .args(["--sb-state"])
        .output()
        .map(|o| {
            let output = String::from_utf8_lossy(&o.stdout);
            output.contains("SecureBoot enabled")
        })
        .unwrap_or(false)
}

/// Check if TPM is present
fn has_tpm() -> bool {
    Path::new("/sys/class/tpm/tpm0").exists()
        || Path::new("/dev/tpm0").exists()
        || Path::new("/dev/tpmrm0").exists()
}

/// Get TPM version
fn get_tpm_version() -> Option<String> {
    // Check TPM 2.0 first
    if Path::new("/sys/class/tpm/tpm0/tpm_version_major").exists() {
        if let Ok(major) = fs::read_to_string("/sys/class/tpm/tpm0/tpm_version_major") {
            if major.trim() == "2" {
                return Some("2.0".to_string());
            }
        }
    }

    // Check for TPM 1.2
    if Path::new("/sys/class/tpm/tpm0/device/caps").exists() {
        return Some("1.2".to_string());
    }

    // Alternative: check using tpm2_getcap or tpm_version
    if Command::new("tpm2_getcap")
        .args(["properties-fixed"])
        .output()
        .is_ok()
    {
        return Some("2.0".to_string());
    }

    None
}

/// Check if any firewall is active
fn is_firewall_active() -> bool {
    // Check ufw
    if Command::new("ufw")
        .args(["status"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("Status: active"))
        .unwrap_or(false)
    {
        return true;
    }

    // Check firewalld
    if Command::new("firewall-cmd")
        .args(["--state"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return true;
    }

    // Check iptables
    if Command::new("iptables")
        .args(["-L", "-n"])
        .output()
        .map(|o| {
            let output = String::from_utf8_lossy(&o.stdout);
            // If there are rules beyond default policies, firewall is considered active
            output.lines().count() > 8
        })
        .unwrap_or(false)
    {
        return true;
    }

    false
}

/// Detect firewall type
fn get_firewall_type() -> String {
    // Check ufw
    if Command::new("ufw")
        .args(["status"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "ufw".to_string();
    }

    // Check firewalld
    if Command::new("firewall-cmd")
        .args(["--state"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "firewalld".to_string();
    }

    // Check nftables
    if Command::new("nft")
        .args(["list", "ruleset"])
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
    {
        return "nftables".to_string();
    }

    // Check iptables
    if Command::new("iptables")
        .args(["-L", "-n"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "iptables".to_string();
    }

    "none".to_string()
}

/// Check if LUKS encryption is used
fn has_luks_encryption() -> bool {
    // Check if any LUKS devices are present
    Command::new("lsblk")
        .args(["-o", "TYPE", "-n"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("crypt"))
        .unwrap_or(false)
}

/// Get kernel lockdown mode
fn get_kernel_lockdown() -> String {
    if let Ok(content) = fs::read_to_string("/sys/kernel/security/lockdown") {
        // Format: "none [integrity] confidentiality" with [] around active mode
        if content.contains("[none]") {
            return "none".to_string();
        } else if content.contains("[integrity]") {
            return "integrity".to_string();
        } else if content.contains("[confidentiality]") {
            return "confidentiality".to_string();
        }
    }
    "none".to_string()
}
