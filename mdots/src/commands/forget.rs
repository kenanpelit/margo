use anyhow::{Context, Result};
use colored::*;
use std::fs;
use std::path::PathBuf;

use crate::config::{
    declared_packages_paths, load_config, load_package_list, load_package_list_any,
    write_package_list_any, ConfigPaths,
};

/// Represents where a package was found and removed from
struct RemovalLocation {
    source: String,
    file: PathBuf,
}

/// Remove a package from mdots tracking without uninstalling it
pub fn run(paths: &ConfigPaths, package_name: &str) -> Result<()> {
    println!("{}", "=== Forget Package ===".blue());
    println!();
    println!(
        "Removing '{}' from mdots tracking (package will remain installed)",
        package_name.yellow()
    );
    println!();

    let config = load_config(paths)?;
    let mut removed_from: Vec<RemovalLocation> = Vec::new();
    let mut found_in_readonly: Vec<(String, PathBuf)> = Vec::new();

    // 1. Check and remove from declared-packages
    let (preferred_declared, fallback_declared) = declared_packages_paths(paths)?;
    let declared_file = if preferred_declared.exists() {
        preferred_declared.clone()
    } else if fallback_declared.exists() {
        fallback_declared.clone()
    } else {
        PathBuf::new()
    };

    if declared_file.exists() {
        if let Ok(mut pkg_list) = load_package_list_any(&declared_file) {
            if let Some(pos) = pkg_list
                .packages
                .iter()
                .position(|p| p.name() == package_name)
            {
                pkg_list.packages.remove(pos);
                write_package_list_any(&preferred_declared, &pkg_list)
                    .context("Failed to write declared-packages file")?;
                removed_from.push(RemovalLocation {
                    source: "declared-packages".to_string(),
                    file: preferred_declared.clone(),
                });
            }
        }
    }

    // 2. Check and remove from system-packages-{host}
    let module_name = format!("system-packages-{}", config.host);
    let module_dir = paths.config_dir.join("modules").join(&module_name);
    let system_packages_file = module_dir.join("packages.yaml");

    if system_packages_file.exists() {
        if let Ok(mut pkg_list) = load_package_list(&system_packages_file) {
            if let Some(pos) = pkg_list
                .packages
                .iter()
                .position(|p| p.name() == package_name)
            {
                pkg_list.packages.remove(pos);
                let yaml = serde_yaml::to_string(&pkg_list)
                    .context("Failed to serialize system-packages")?;
                fs::write(&system_packages_file, yaml)
                    .context("Failed to write system-packages file")?;
                removed_from.push(RemovalLocation {
                    source: format!("system-packages-{}", config.host),
                    file: system_packages_file.clone(),
                });
            }
        }
    }

    // 3. Check base.yaml (read-only warning)
    let base_file = paths.base_packages_file();
    if base_file.exists() {
        if let Ok(pkg_list) = load_package_list_any(&base_file) {
            if pkg_list.packages.iter().any(|p| p.name() == package_name) {
                found_in_readonly.push(("base".to_string(), base_file));
            }
        }
    }

    // 4. Check host config packages (read-only warning)
    if config.packages.iter().any(|p| p.name() == package_name) {
        let config_file = paths.host_packages_file(&config.host);
        found_in_readonly.push((format!("host ({})", config.host), config_file));
    }

    // 5. Check enabled modules (read-only warning)
    for module_name in &config.enabled_modules {
        let modules_dir = paths.modules_dir();
        let module_file = modules_dir.join(format!("{}.yaml", module_name));
        let module_lua = modules_dir.join(format!("{}.lua", module_name));
        let module_dir = modules_dir.join(module_name);

        let module_path = if module_dir.exists()
            && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
        {
            if module_dir.join("module.lua").exists() {
                module_dir.join("module.lua")
            } else {
                module_dir.join("module.yaml")
            }
        } else if module_lua.exists() {
            module_lua
        } else if module_file.exists() {
            module_file
        } else {
            continue;
        };

        if let Ok(module) = crate::config::load_module(modules_dir.join(module_name)) {
            if module.packages().iter().any(|p| p.name() == package_name) {
                found_in_readonly.push((format!("module ({})", module_name), module_path));
            }
        } else if let Ok(module) = crate::config::load_module(&module_path) {
            if module.packages().iter().any(|p| p.name() == package_name) {
                found_in_readonly.push((format!("module ({})", module_name), module_path));
            }
        }
    }

    // 6. Check additional_packages (read-only warning)
    if config
        .additional_packages
        .iter()
        .any(|p| p.name() == package_name)
    {
        let config_file = paths.host_packages_file(&config.host);
        found_in_readonly.push(("additional_packages".to_string(), config_file));
    }

    // Output results
    if removed_from.is_empty() && found_in_readonly.is_empty() {
        println!(
            "{} Package '{}' is not tracked by mdots",
            "!".yellow(),
            package_name
        );
        println!();
        println!("Nothing to forget.");
        return Ok(());
    }

    if !removed_from.is_empty() {
        println!("{}", "Removed from:".green());
        for loc in &removed_from {
            println!("  {} {} ({})", "✓".green(), loc.source, loc.file.display());
        }
        println!();
    }

    if !found_in_readonly.is_empty() {
        println!(
            "{}",
            "Package also found in these locations (manual removal required):".yellow()
        );
        for (source, file) in &found_in_readonly {
            println!("  {} {} ({})", "!".yellow(), source, file.display());
        }
        println!();
        println!("To fully forget this package, manually remove it from the files above.");
        println!();
    }

    if !removed_from.is_empty() {
        println!(
            "{} Package '{}' forgotten from mdots tracking",
            "✓".green(),
            package_name
        );
        println!();
        println!("The package remains installed on your system.");
        println!("It will no longer be managed by mdots sync or mdots merge.");
    }

    Ok(())
}
