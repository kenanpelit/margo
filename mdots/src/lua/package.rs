//! Package query helpers for Lua modules
//!
//! Provides the `mdots.package.*` API for querying installed packages
//! and package availability.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};
use std::process::Command;

/// Register package query helpers in the mdots table
pub fn register_package_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    let mdots: Table = globals
        .get("mdots")
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    let package = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.is_installed(name) -> boolean
    // Check if a pacman package is installed
    package
        .set(
            "is_installed",
            lua.create_function(|_, name: String| Ok(is_pacman_installed(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.version(name) -> string or nil
    // Get the installed version of a pacman package
    package
        .set(
            "version",
            lua.create_function(|_, name: String| Ok(get_pacman_version(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.is_available(name) -> boolean
    // Check if a package is available in the repos (pacman -Ss)
    package
        .set(
            "is_available",
            lua.create_function(|_, name: String| Ok(is_pacman_available(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.repo(name) -> string or nil
    // Get the repository a package belongs to (core, extra, multilib, etc.)
    package
        .set(
            "repo",
            lua.create_function(|_, name: String| Ok(get_pacman_repo(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.flatpak_installed(app_id) -> boolean
    // Check if a Flatpak application is installed
    package
        .set(
            "flatpak_installed",
            lua.create_function(|_, app_id: String| Ok(is_flatpak_installed(&app_id)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.flatpak_version(app_id) -> string or nil
    // Get the installed version of a Flatpak application
    package
        .set(
            "flatpak_version",
            lua.create_function(|_, app_id: String| Ok(get_flatpak_version(&app_id)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.aur_available(name) -> boolean
    // Check if a package is available in the AUR
    // Note: This requires network access and may be slow
    package
        .set(
            "aur_available",
            lua.create_function(|_, name: String| Ok(is_aur_available(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.list_installed() -> array of package names
    // Get a list of all installed pacman packages
    package
        .set(
            "list_installed",
            lua.create_function(|lua, ()| {
                let packages = list_installed_packages();
                let table = lua.create_table()?;
                for (i, pkg) in packages.iter().enumerate() {
                    table.set(i + 1, pkg.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.list_explicit() -> array of explicitly installed package names
    // Get a list of explicitly installed packages (not dependencies)
    package
        .set(
            "list_explicit",
            lua.create_function(|lua, ()| {
                let packages = list_explicit_packages();
                let table = lua.create_table()?;
                for (i, pkg) in packages.iter().enumerate() {
                    table.set(i + 1, pkg.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.is_foreign(name) -> boolean
    // Check if a package is foreign (AUR/manual install, not in repos)
    package
        .set(
            "is_foreign",
            lua.create_function(|_, name: String| Ok(is_foreign_package(&name)))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.depends_on(name) -> array of dependencies
    // Get the dependencies of an installed package
    package
        .set(
            "depends_on",
            lua.create_function(|lua, name: String| {
                let deps = get_package_depends(&name);
                let table = lua.create_table()?;
                for (i, dep) in deps.iter().enumerate() {
                    table.set(i + 1, dep.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.package.required_by(name) -> array of packages that depend on this one
    // Get packages that depend on the given package
    package
        .set(
            "required_by",
            lua.create_function(|lua, name: String| {
                let deps = get_required_by(&name);
                let table = lua.create_table()?;
                for (i, dep) in deps.iter().enumerate() {
                    table.set(i + 1, dep.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    mdots
        .set("package", package)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

// ============================================================================
// Pacman helper functions
// ============================================================================

/// Check if a pacman package is installed
fn is_pacman_installed(name: &str) -> bool {
    Command::new("pacman")
        .args(["-Q", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get the installed version of a pacman package
fn get_pacman_version(name: &str) -> Option<String> {
    let output = Command::new("pacman")
        .args(["-Q", name])
        .output()
        .ok()
        .filter(|o| o.status.success())?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Output format: "package-name version"
    stdout.split_whitespace().nth(1).map(|s| s.to_string())
}

/// Check if a package is available in the repos
fn is_pacman_available(name: &str) -> bool {
    Command::new("pacman")
        .args(["-Si", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get the repository a package belongs to
fn get_pacman_repo(name: &str) -> Option<String> {
    let output = Command::new("pacman")
        .args(["-Si", name])
        .output()
        .ok()
        .filter(|o| o.status.success())?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Repository") {
            // Format: "Repository      : extra"
            return rest.trim().strip_prefix(':').map(|s| s.trim().to_string());
        }
    }
    None
}

/// List all installed packages
fn list_installed_packages() -> Vec<String> {
    let output = match Command::new("pacman").args(["-Qq"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect()
}

/// List explicitly installed packages
fn list_explicit_packages() -> Vec<String> {
    let output = match Command::new("pacman").args(["-Qqe"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect()
}

/// Check if a package is foreign (not in repos, e.g., AUR)
fn is_foreign_package(name: &str) -> bool {
    // -Qm lists foreign packages
    let output = match Command::new("pacman").args(["-Qm", name]).output() {
        Ok(o) => o,
        _ => return false,
    };
    output.status.success()
}

/// Get dependencies of a package
fn get_package_depends(name: &str) -> Vec<String> {
    let output = match Command::new("pacman").args(["-Qi", name]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Depends On") {
            // Format: "Depends On      : pkg1  pkg2  pkg3" or "None"
            let deps_str = rest.trim().strip_prefix(':').unwrap_or("").trim();
            if deps_str == "None" {
                return Vec::new();
            }
            return deps_str
                .split_whitespace()
                .map(|s| {
                    // Remove version constraints like ">=1.0"
                    s.split(['>', '<', '=']).next().unwrap_or(s).to_string()
                })
                .collect();
        }
    }
    Vec::new()
}

/// Get packages that require the given package
fn get_required_by(name: &str) -> Vec<String> {
    let output = match Command::new("pacman").args(["-Qi", name]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Required By") {
            // Format: "Required By     : pkg1  pkg2  pkg3" or "None"
            let deps_str = rest.trim().strip_prefix(':').unwrap_or("").trim();
            if deps_str == "None" {
                return Vec::new();
            }
            return deps_str.split_whitespace().map(|s| s.to_string()).collect();
        }
    }
    Vec::new()
}

// ============================================================================
// Flatpak helper functions
// ============================================================================

/// Check if a Flatpak application is installed
fn is_flatpak_installed(app_id: &str) -> bool {
    // Check user installation
    let user_check = Command::new("flatpak")
        .args(["info", "--user", app_id])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if user_check {
        return true;
    }

    // Check system installation
    Command::new("flatpak")
        .args(["info", "--system", app_id])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get the installed version of a Flatpak application
fn get_flatpak_version(app_id: &str) -> Option<String> {
    // Try user first
    let output = Command::new("flatpak")
        .args(["info", "--user", app_id])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .or_else(|| {
            Command::new("flatpak")
                .args(["info", "--system", app_id])
                .output()
                .ok()
                .filter(|o| o.status.success())
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Version:") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

// ============================================================================
// AUR helper functions
// ============================================================================

/// Check if a package is available in the AUR
/// Note: This uses a simple HTTPS request to the AUR RPC
fn is_aur_available(name: &str) -> bool {
    // Use curl to check AUR RPC (avoid adding HTTP dependencies)
    // This is a simple check - just see if the package exists
    let output = Command::new("curl")
        .args([
            "-s",
            "-f",
            &format!("https://aur.archlinux.org/rpc/?v=5&type=info&arg={}", name),
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // Check if results are non-empty
            // Response format: {"resultcount":1,"results":[...]}
            stdout.contains("\"resultcount\":1") || stdout.contains("\"resultcount\": 1")
        }
        _ => false,
    }
}
