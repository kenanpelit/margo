//! Network detection helpers for Lua modules
//!
//! Provides the `mdots.network.*` API for detecting network configuration and connectivity.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Register network detection helpers
pub fn register_network_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    let mdots: Table = globals
        .get("mdots")
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    let network = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.has_wifi() -> boolean
    network
        .set(
            "has_wifi",
            lua.create_function(|_, ()| Ok(has_wifi()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.has_ethernet() -> boolean
    network
        .set(
            "has_ethernet",
            lua.create_function(|_, ()| Ok(has_ethernet()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.has_bluetooth() -> boolean
    network
        .set(
            "has_bluetooth",
            lua.create_function(|_, ()| Ok(has_bluetooth()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.is_connected() -> boolean
    network
        .set(
            "is_connected",
            lua.create_function(|_, ()| Ok(is_connected()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.connection_type() -> "wifi" | "ethernet" | "none" | "unknown"
    network
        .set(
            "connection_type",
            lua.create_function(|_, ()| Ok(get_connection_type()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.list_interfaces() -> array of interface names
    network
        .set(
            "list_interfaces",
            lua.create_function(|lua, ()| {
                let interfaces = list_interfaces();
                let table = lua.create_table()?;
                for (i, iface) in interfaces.iter().enumerate() {
                    table.set(i + 1, iface.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.active_interface() -> interface name or nil
    network
        .set(
            "active_interface",
            lua.create_function(|_, ()| Ok(get_active_interface()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.interface_type(name) -> "wifi" | "ethernet" | "loopback" | "unknown"
    network
        .set(
            "interface_type",
            lua.create_function(|_, name: String| Ok(get_interface_type(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.interface_up(name) -> boolean
    network
        .set(
            "interface_up",
            lua.create_function(|_, name: String| Ok(is_interface_up(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.has_ipv6() -> boolean
    network
        .set(
            "has_ipv6",
            lua.create_function(|_, ()| Ok(has_ipv6()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.network.hostname() -> hostname string
    network
        .set(
            "hostname",
            lua.create_function(|_, ()| Ok(get_hostname()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    mdots
        .set("network", network)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Check if WiFi hardware is present
fn has_wifi() -> bool {
    // Check for wireless interfaces
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.filter_map(|e| e.ok()) {
            let wireless_path = entry.path().join("wireless");
            if wireless_path.exists() {
                return true;
            }
        }
    }
    false
}

/// Check if Ethernet hardware is present
fn has_ethernet() -> bool {
    // Check for ethernet interfaces (non-wireless, non-loopback)
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "lo" {
                continue; // Skip loopback
            }

            let wireless_path = entry.path().join("wireless");
            if !wireless_path.exists() {
                // Not wireless, probably ethernet
                return true;
            }
        }
    }
    false
}

/// Check if Bluetooth hardware is present
fn has_bluetooth() -> bool {
    Path::new("/sys/class/bluetooth").exists()
        && fs::read_dir("/sys/class/bluetooth")
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false)
}

/// Check if system has network connectivity
fn is_connected() -> bool {
    // Check if any non-loopback interface has an IP address
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "lo" {
                continue;
            }

            let operstate_path = entry.path().join("operstate");
            if let Ok(state) = fs::read_to_string(&operstate_path) {
                if state.trim() == "up" {
                    return true;
                }
            }
        }
    }
    false
}

/// Get current connection type
fn get_connection_type() -> String {
    if let Some(iface) = get_active_interface() {
        return get_interface_type(&iface);
    }
    "none".to_string()
}

/// List all network interfaces
fn list_interfaces() -> Vec<String> {
    let mut interfaces = Vec::new();

    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            interfaces.push(name);
        }
    }

    interfaces
}

/// Get the active network interface
fn get_active_interface() -> Option<String> {
    // First check default route
    if let Ok(output) = Command::new("ip")
        .args(["route", "show", "default"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Format: "default via 192.168.1.1 dev eth0 ..."
        // Look for interface name after "dev"
        if let Some(stripped) = stdout.split("dev ").nth(1) {
            if let Some(iface) = stripped.split_whitespace().next() {
                return Some(iface.to_string());
            }
        }
    }

    // Fallback: find first up interface that's not loopback
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "lo" {
                continue;
            }

            let operstate_path = entry.path().join("operstate");
            if let Ok(state) = fs::read_to_string(&operstate_path) {
                if state.trim() == "up" {
                    return Some(name);
                }
            }
        }
    }

    None
}

/// Get interface type
fn get_interface_type(name: &str) -> String {
    if name == "lo" {
        return "loopback".to_string();
    }

    let iface_path = Path::new("/sys/class/net").join(name);

    // Check if wireless
    if iface_path.join("wireless").exists() {
        return "wifi".to_string();
    }

    // Check if it's a virtual interface
    if let Ok(uevent) = fs::read_to_string(iface_path.join("uevent")) {
        if uevent.contains("DEVTYPE=wlan") {
            return "wifi".to_string();
        }
    }

    // Check type file
    if let Ok(type_num) = fs::read_to_string(iface_path.join("type")) {
        match type_num.trim() {
            "1" => return "ethernet".to_string(),
            "24" => return "ethernet".to_string(), // Ethernet emulation
            "32" => return "infiniband".to_string(),
            "512" => return "ppp".to_string(),
            "768" | "769" => return "tunnel".to_string(),
            "772" => return "loopback".to_string(),
            _ => {}
        }
    }

    // Default guess based on name
    if name.starts_with("wl") || name.starts_with("wlan") {
        "wifi".to_string()
    } else if name.starts_with("en") || name.starts_with("eth") {
        "ethernet".to_string()
    } else if name.starts_with("br") {
        "bridge".to_string()
    } else if name.starts_with("docker") || name.starts_with("veth") {
        "virtual".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Check if an interface is up
fn is_interface_up(name: &str) -> bool {
    let operstate_path = Path::new("/sys/class/net").join(name).join("operstate");
    fs::read_to_string(&operstate_path)
        .map(|s| s.trim() == "up")
        .unwrap_or(false)
}

/// Check if IPv6 is enabled and available
fn has_ipv6() -> bool {
    // Check if IPv6 is disabled
    if let Ok(content) = fs::read_to_string("/proc/sys/net/ipv6/conf/all/disable_ipv6") {
        if content.trim() == "1" {
            return false;
        }
    }

    // Check if any interface has an IPv6 address
    Command::new("ip")
        .args(["-6", "addr", "show", "scope", "global"])
        .output()
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.contains("inet6")
        })
        .unwrap_or(false)
}

/// Get system hostname
fn get_hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| {
            fs::read_to_string("/etc/hostname")
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        })
}
