use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::process::Command;

use crate::config::{Config, ConfigPaths, PackageEntry, PackageType};

/// Represents a package with its metadata
#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub package_type: PackageType,
}

impl From<&PackageEntry> for Package {
    fn from(entry: &PackageEntry) -> Self {
        Self {
            name: entry.name().to_string(),
            package_type: entry.package_type(),
        }
    }
}

/// Package manager for handling all package operations
pub struct PackageManager {
    paths: ConfigPaths,
}

impl PackageManager {
    pub fn new(paths: ConfigPaths) -> Self {
        Self { paths }
    }

    /// Get all declared packages from base, system-packages, declared-packages, host, modules, and additional packages
    pub fn get_declared_packages(&self, config: &Config) -> Result<Vec<Package>> {
        let mut packages = Vec::new();
        let mut excluded = HashSet::new();

        // 1. Load base packages
        let base_file = self.paths.base_packages_file();
        if base_file.exists() {
            let base_list = crate::config::load_package_list_any(&base_file)?;
            for entry in &base_list.packages {
                packages.push(Package::from(entry));
            }
        }

        // 2. Load declared-packages (created by mdots install/search)
        let (preferred_declared, fallback_declared) =
            crate::config::declared_packages_paths(&self.paths)?;
        let declared_packages_file = if preferred_declared.exists() {
            preferred_declared
        } else if fallback_declared.exists() {
            fallback_declared
        } else {
            preferred_declared
        };
        if declared_packages_file.exists() {
            let declared_list = crate::config::load_package_list_any(&declared_packages_file)?;
            for entry in &declared_list.packages {
                packages.push(Package::from(entry));
            }
        }

        // 3. Load system-packages-{host}.yaml (created by mdots merge)
        let system_packages_file = self
            .paths
            .config_dir
            .join(format!("system-packages-{}.yaml", config.host));
        if system_packages_file.exists() {
            let system_list = crate::config::load_package_list_any(&system_packages_file)?;
            for entry in &system_list.packages {
                packages.push(Package::from(entry));
            }
        }

        // 4. Load host-specific packages and exclusions from resolved config
        for entry in &config.packages {
            packages.push(Package::from(entry));
        }
        excluded.extend(config.exclude.iter().cloned());

        // 5. Load enabled module packages (supports legacy yaml, lua, and directory modules)
        for module_name in &config.enabled_modules {
            // Try to find module as either file or directory
            let modules_dir = self.paths.modules_dir();
            let module_yaml = modules_dir.join(format!("{}.yaml", module_name));
            let module_lua = modules_dir.join(format!("{}.lua", module_name));
            let module_dir = modules_dir.join(module_name);

            let module_nix = modules_dir.join(format!("{}.nix", module_name));

            let module_path = if module_dir.exists()
                && (module_dir.join("module.yaml").exists()
                    || module_dir.join("module.lua").exists()
                    || module_dir.join("module.nix").exists())
            {
                module_dir
            } else if module_yaml.exists() {
                module_yaml
            } else if module_lua.exists() {
                module_lua
            } else if module_nix.exists() {
                module_nix
            } else {
                log::warn!("Module '{}' not found", module_name);
                continue;
            };

            match crate::config::load_module(&module_path) {
                Ok(module) => {
                    for entry in module.packages() {
                        packages.push(Package::from(&entry));
                    }
                }
                Err(e) => {
                    log::warn!("Failed to load module '{}': {}", module_name, e);
                }
            }
        }

        // 6. Load additional packages from config (backwards compatibility)
        for entry in &config.additional_packages {
            packages.push(Package::from(entry));
        }

        // Remove duplicates and excluded packages
        let mut unique_packages: HashMap<String, Package> = HashMap::new();

        for pkg in packages {
            if excluded.contains(&pkg.name) {
                continue;
            }

            // Simply use the last occurrence (allows later definitions to override earlier ones)
            unique_packages.insert(pkg.name.clone(), pkg);
        }

        Ok(unique_packages.into_values().collect())
    }

    /// Get installed native packages with versions (uses the configured backend)
    pub fn get_installed_native_packages(
        &self,
        config: &Config,
    ) -> Result<HashMap<String, String>> {
        let backend = crate::backend::create_backend(config)?;
        backend.get_installed_packages()
    }

    /// Get installed native packages with versions (legacy name, uses pacman directly)
    /// Prefer get_installed_native_packages() for new code
    pub fn get_installed_pacman_packages(&self) -> Result<HashMap<String, String>> {
        let output = std::process::Command::new("pacman")
            .args(["-Q"])
            .output()
            .context("Failed to run pacman -Q")?;

        if !output.status.success() {
            anyhow::bail!("pacman -Q failed");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut packages = HashMap::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() == 2 {
                packages.insert(parts[0].to_string(), parts[1].to_string());
            }
        }

        Ok(packages)
    }

    /// Get installed flatpak packages
    pub fn get_installed_flatpaks(&self, scope: &str) -> Result<Vec<String>> {
        let output = Command::new("flatpak")
            .args(["list", "--columns=application", scope])
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(stdout.lines().map(|s| s.to_string()).collect())
            }
            _ => Ok(Vec::new()), // Flatpak not installed or no packages
        }
    }
}

/// Compare two version strings using `vercmp` if available, falling back to lexicographic order.
/// Returns: -1 if v1 < v2, 0 if v1 == v2, 1 if v1 > v2
// kept: tested utility; Lua helpers has an independent copy but this one delegates to `vercmp`
#[allow(dead_code)]
pub fn compare_versions(v1: &str, v2: &str) -> i32 {
    // Use vercmp from pacman if available, otherwise do simple comparison
    let output = Command::new("vercmp").args([v1, v2]).output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.trim().parse().unwrap_or(0)
        }
        _ => {
            // Fallback: simple string comparison
            if v1 == v2 {
                0
            } else if v1 < v2 {
                -1
            } else {
                1
            }
        }
    }
}

/// Represents the sync plan (what needs to be done)
#[allow(dead_code)] // kept: package sync-plan API exercised by tests; not yet wired into sync flow
#[derive(Debug, Default)]
pub struct SyncPlan {
    pub to_install: Vec<Package>,
    pub to_remove: Vec<String>,
    pub flatpak_to_install: Vec<String>,
    pub flatpak_to_remove: Vec<String>,
}

impl SyncPlan {
    #[allow(dead_code)] // kept: see SyncPlan; exercised by tests, not yet wired into sync flow
    pub fn is_empty(&self) -> bool {
        self.to_install.is_empty()
            && self.to_remove.is_empty()
            && self.flatpak_to_install.is_empty()
            && self.flatpak_to_remove.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_versions() {
        // Note: This will fallback to string comparison if vercmp not available
        assert_eq!(compare_versions("1.0.0", "1.0.0"), 0);
    }

    #[test]
    fn test_sync_plan_empty() {
        let plan = SyncPlan::default();
        assert!(plan.is_empty());
    }
}
