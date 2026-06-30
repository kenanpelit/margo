use anyhow::Result;
use colored::*;
use serde::Serialize;
use std::path::PathBuf;

use crate::config::{
    declared_packages_paths, load_config, load_package_list_any, resolve_config_path, ConfigPaths,
};
use crate::package::PackageManager;

#[derive(Serialize)]
struct FindOutput {
    package: String,
    found: bool,
    installed: bool,
    locations: Vec<PackageLocation>,
}

#[derive(Serialize)]
struct PackageLocation {
    source: String,
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    module: Option<String>,
}

pub fn run(paths: &ConfigPaths, package_name: &str, json: bool) -> Result<()> {
    let config = load_config(paths)?;
    let config_path = resolve_config_path(paths)?;
    let mut locations = Vec::new();

    // Check if package is installed (check both pacman and flatpak)
    let pm = PackageManager::new(paths.clone());
    let installed_pacman = pm.get_installed_pacman_packages().unwrap_or_default();

    let flatpak_scope = match &config.flatpak_scope {
        crate::config::FlatpakScope::User => "--user",
        crate::config::FlatpakScope::System => "--system",
    };
    let installed_flatpaks = pm.get_installed_flatpaks(flatpak_scope).unwrap_or_default();

    let is_installed = installed_pacman.contains_key(package_name)
        || installed_flatpaks.contains(&package_name.to_string());

    // 1. Search in base.yaml
    let base_file = paths.base_packages_file();
    if base_file.exists() && package_in_file(&base_file, package_name)? {
        locations.push(PackageLocation {
            source: "base".to_string(),
            file: base_file.display().to_string(),
            module: None,
        });
    }

    // 2. Search in host-specific config packages
    if config.packages.iter().any(|p| p.name() == package_name) {
        locations.push(PackageLocation {
            source: format!("host ({})", config.host),
            file: config_path.display().to_string(),
            module: None,
        });
    }

    // 3. Search in declared-packages
    let (preferred_declared, fallback_declared) = declared_packages_paths(paths)?;
    let declared_packages_file = if preferred_declared.exists() {
        preferred_declared
    } else if fallback_declared.exists() {
        fallback_declared
    } else {
        preferred_declared
    };
    if declared_packages_file.exists() && package_in_file(&declared_packages_file, package_name)? {
        locations.push(PackageLocation {
            source: "declared-packages".to_string(),
            file: declared_packages_file.display().to_string(),
            module: None,
        });
    }

    // 4. Search in enabled modules
    for module_name in &config.enabled_modules {
        let modules_dir = paths.modules_dir();
        let module_file = modules_dir.join(format!("{}.yaml", module_name));
        let module_lua = modules_dir.join(format!("{}.lua", module_name));
        let module_dir = modules_dir.join(module_name);

        let module_path = if module_dir.exists()
            && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
        {
            module_dir
        } else if module_file.exists() {
            module_file
        } else if module_lua.exists() {
            module_lua
        } else {
            continue;
        };

        if package_in_module(&module_path, package_name)? {
            let module_display = if module_path.is_dir() {
                if module_path.join("module.lua").exists() {
                    module_path.join("module.lua")
                } else if module_path.join("module.yaml").exists() {
                    module_path.join("module.yaml")
                } else {
                    module_path.clone()
                }
            } else {
                module_path.clone()
            };

            locations.push(PackageLocation {
                source: "module".to_string(),
                file: module_display.display().to_string(),
                module: Some(module_name.clone()),
            });
        }
    }

    // 4. Search in additional_packages in the config file
    for entry in &config.additional_packages {
        if entry.name() == package_name {
            locations.push(PackageLocation {
                source: "additional_packages".to_string(),
                file: config_path.display().to_string(),
                module: None,
            });
            break;
        }
    }

    // Output results
    if json {
        let output = FindOutput {
            package: package_name.to_string(),
            found: !locations.is_empty(),
            installed: is_installed,
            locations,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if locations.is_empty() {
            println!(
                "{} Package '{}' not found in arch-config",
                "✗".red(),
                package_name.yellow()
            );
            println!();
            println!("The package is not declared in:");
            println!("  • Base packages");
            println!("  • Host-specific packages");
            println!("  • Declared packages (from dcli install/search)");
            println!("  • Enabled modules");
            println!("  • Additional packages");
        } else {
            let status = if is_installed {
                format!("{} Installed", "✓".green())
            } else {
                format!("{} Not Installed", "○".yellow())
            };

            println!(
                "{} Found '{}' in {} location(s): {}",
                "✓".green(),
                package_name.yellow(),
                locations.len(),
                status
            );
            println!();

            for loc in &locations {
                match &loc.module {
                    Some(module) => {
                        println!("  {} Module: {}", "→".blue(), module.cyan());
                    }
                    None => {
                        println!("  {} Source: {}", "→".blue(), loc.source.cyan());
                    }
                }
                println!("    File: {}", loc.file.dimmed());
                println!();
            }
        }
    }

    Ok(())
}

/// Check if a package is defined in a file
fn package_in_file(file_path: &PathBuf, package_name: &str) -> Result<bool> {
    let pkg_list = load_package_list_any(file_path)?;

    for entry in &pkg_list.packages {
        if entry.name() == package_name {
            return Ok(true);
        }
    }

    Ok(false)
}

fn package_in_module(module_path: &PathBuf, package_name: &str) -> Result<bool> {
    let module = crate::config::load_module(module_path)?;
    Ok(module.packages().iter().any(|p| p.name() == package_name))
}
