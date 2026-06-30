use anyhow::{Context, Result};
use colored::*;
use std::collections::HashSet;
use std::fs;

use crate::config::{
    declared_packages_paths, load_config, load_package_list_any, write_package_list_any,
    ConfigPaths, PackageList, RunHooksAsUser,
};
use crate::package::PackageManager;

/// Install a package using the configured package manager and add to host config
pub fn install(package: &str, paths: &ConfigPaths) -> Result<()> {
    println!("{} Installing package: {}", "→".blue(), package.green());

    // Load config and create backend
    let config = load_config(paths)?;
    let backend = crate::backend::create_backend(&config)?;

    let success = backend.install_interactive(package)?;

    if success {
        println!("{} Package installed successfully", "✓".green());

        // Add package to host config
        add_package_to_host_config(package, paths)?;

        println!();
        println!(
            "{}",
            "Package installed and added to dcli management".green()
        );
        Ok(())
    } else {
        anyhow::bail!("Failed to install package: {}", package);
    }
}

/// Add a package to the declared-packages file
fn add_package_to_host_config(package: &str, paths: &ConfigPaths) -> Result<()> {
    let (preferred_declared, fallback_declared) = declared_packages_paths(paths)?;
    let declared_packages_file = preferred_declared;

    // Load config to check if package is already declared
    let config = load_config(paths)?;

    println!();
    println!("{} Adding package to configuration...", "→".blue());

    // Check if package is already in declared packages
    let pkg_manager = PackageManager::new(paths.clone());
    let declared_packages = pkg_manager.get_declared_packages(&config)?;
    let declared: HashSet<String> = declared_packages.iter().map(|p| p.name.clone()).collect();

    if declared.contains(package) {
        println!("  {} {} already managed by dcli", "✓".green(), package);
        return Ok(());
    }

    let mut pkg_list = if declared_packages_file.exists() {
        load_package_list_any(&declared_packages_file)?
    } else if fallback_declared.exists() {
        load_package_list_any(&fallback_declared)?
    } else {
        PackageList {
            description: "Packages installed via dcli install or dcli search commands".to_string(),
            packages: Vec::new(),
            exclude: Vec::new(),
            conflicts: Vec::new(),
            pre_install_hook: None,
            post_install_hook: None,
            hook_behavior: "ask".to_string(),
            pre_hook_behavior: None,
            post_hook_behavior: None,
            run_hooks_as_user: RunHooksAsUser::Bool(false),
            post_disable_hook: None,
            post_disable_behavior: None,
        }
    };

    if pkg_list.description.is_empty() {
        pkg_list.description =
            "Packages installed via dcli install or dcli search commands".to_string();
    }

    // Add package to packages array
    pkg_list
        .packages
        .push(crate::config::PackageEntry::Simple(package.to_string()));

    if !declared_packages_file.exists() {
        println!(
            "{} Creating declared-packages file: {}",
            "→".blue(),
            declared_packages_file.display()
        );
        fs::create_dir_all(declared_packages_file.parent().unwrap())
            .context("Failed to create parent directory")?;
    }

    write_package_list_any(&declared_packages_file, &pkg_list).with_context(|| {
        format!(
            "Failed to write declared-packages file: {}",
            declared_packages_file.display()
        )
    })?;

    println!(
        "  {} Added {} to {}",
        "✓".green(),
        package,
        declared_packages_file.display()
    );

    Ok(())
}

/// Remove a package using the configured package manager and remove from host config
pub fn remove(package: &str, paths: &ConfigPaths) -> Result<()> {
    println!("{} Removing package: {}", "→".blue(), package.yellow());

    // Load config and create backend
    let config = load_config(paths)?;
    let backend = crate::backend::create_backend(&config)?;

    let success = backend.remove_interactive(package)?;

    if success {
        println!("{} Package removed successfully", "✓".green());

        // Remove package from host config
        remove_package_from_host_config(package, paths)?;

        println!();
        println!(
            "{}",
            "Package removed and removed from dcli management".green()
        );
        Ok(())
    } else {
        anyhow::bail!("Failed to remove package: {}", package);
    }
}

/// Remove a package from the declared-packages file
fn remove_package_from_host_config(package: &str, paths: &ConfigPaths) -> Result<()> {
    let (preferred_declared, fallback_declared) = declared_packages_paths(paths)?;
    let declared_packages_file = preferred_declared;
    let source_file = if declared_packages_file.exists() {
        declared_packages_file.clone()
    } else {
        fallback_declared
    };

    // Check if declared-packages file exists
    if !source_file.exists() {
        println!(
            "  {} No declared-packages file found, nothing to remove",
            "→".blue()
        );
        return Ok(());
    }

    println!();
    println!("{} Removing package from configuration...", "→".blue());

    let mut pkg_list = load_package_list_any(&source_file)?;

    // Remove package from packages array
    let mut removed = false;
    if let Some(pos) = pkg_list.packages.iter().position(|p| p.name() == package) {
        pkg_list.packages.remove(pos);
        removed = true;
    }

    if !removed {
        println!(
            "  {} Package {} not found in {}",
            "→".blue(),
            package,
            declared_packages_file.display()
        );
        return Ok(());
    }

    if !declared_packages_file.exists() {
        fs::create_dir_all(declared_packages_file.parent().unwrap())
            .context("Failed to create parent directory")?;
    }

    write_package_list_any(&declared_packages_file, &pkg_list).with_context(|| {
        format!(
            "Failed to write declared-packages file: {}",
            declared_packages_file.display()
        )
    })?;

    println!(
        "  {} Removed {} from {}",
        "✓".green(),
        package,
        declared_packages_file.display()
    );

    Ok(())
}
