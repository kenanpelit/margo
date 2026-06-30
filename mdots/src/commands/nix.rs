use anyhow::Result;
use colored::*;
use serde_json::json;

use crate::config::{load_config, ConfigPaths};

/// Install nix and home-manager
pub fn install(paths: &ConfigPaths) -> Result<()> {
    println!("{}", "=== Installing Nix & Home Manager ===".blue().bold());
    println!();

    let config = load_config(paths)?;
    let pm_type = crate::config::resolve_package_manager(&config)?;

    if crate::nix::is_nix_installed() {
        println!("{} Nix is already installed", "✓".green());
    } else {
        crate::nix::install_nix(&pm_type)?;
        println!("{} Nix installed successfully", "✓".green());
    }

    if crate::nix::is_home_manager_installed() {
        println!("{} Home Manager is already installed", "✓".green());
    } else {
        crate::nix::setup_channels(
            &config.nix.nixpkgs_channel,
            &config.nix.home_manager_channel,
        )?;
        crate::nix::install_home_manager()?;
        println!("{} Home Manager installed successfully", "✓".green());
    }

    println!();
    println!("{}", "✓ Nix & Home Manager setup complete!".green());
    println!();
    println!("Next steps:");
    println!(
        "  1. Run {} to set up mdots home-manager config",
        "mdots init --nix-init".cyan()
    );
    println!(
        "  2. Or manually create {}",
        "~/.config/mdots/home-manager/home.nix".cyan()
    );

    Ok(())
}

/// Run home-manager switch
pub fn switch(paths: &ConfigPaths) -> Result<()> {
    let config = load_config(paths)?;

    if !config.nix.home_manager_enabled {
        anyhow::bail!("Home Manager is not enabled in your config. Set nix.home_manager_enabled: true in your host config.");
    }

    if !crate::nix::is_home_manager_installed() {
        anyhow::bail!("Home Manager is not installed. Run 'mdots nix install' first.");
    }

    crate::nix::home_manager_switch(paths, &config)
}

/// Update nix channels/flake inputs and run home-manager switch
pub fn update(paths: &ConfigPaths) -> Result<()> {
    let config = load_config(paths)?;

    if !config.nix.home_manager_enabled {
        anyhow::bail!("Home Manager is not enabled in your config. Set nix.home_manager_enabled: true in your host config.");
    }

    if !crate::nix::is_home_manager_installed() {
        anyhow::bail!("Home Manager is not installed. Run 'mdots nix install' first.");
    }

    crate::nix::home_manager_update(paths, &config)
}

/// Search nixpkgs for a package
pub fn search(query: &str) -> Result<()> {
    if !crate::nix::is_nix_installed() {
        anyhow::bail!("Nix is not installed. Run 'mdots nix install' first.");
    }

    crate::nix::nix_search(query)
}

/// Show nix/home-manager status
pub fn status(paths: &ConfigPaths, json: bool) -> Result<()> {
    let config = load_config(paths)?;
    let nix_status = crate::nix::nix_status(paths, &config)?;

    if json {
        let output = json!({
            "nix": {
                "installed": nix_status.nix_installed,
                "version": nix_status.nix_version,
                "daemon_running": nix_status.daemon_running,
            },
            "home_manager": {
                "installed": nix_status.hm_installed,
                "version": nix_status.hm_version,
                "enabled_in_config": config.nix.home_manager_enabled,
                "home_nix_exists": nix_status.home_nix_exists,
                "mdots_packages_exists": nix_status.mdots_packages_exists,
                "flake_enabled": nix_status.flake_enabled,
            },
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("{}", "=== Nix & Home Manager Status ===".blue().bold());
    println!();

    println!("{}", "Nix:".cyan().bold());
    if nix_status.nix_installed {
        println!(
            "  {} Installed: {}",
            "✓".green(),
            nix_status.nix_version.as_deref().unwrap_or("unknown")
        );
        println!(
            "  {} Daemon: {}",
            "✓".green(),
            if nix_status.daemon_running {
                "running".green().to_string()
            } else {
                "not running".yellow().to_string()
            }
        );
    } else {
        println!("  {} Not installed", "✗".red());
    }
    println!();

    println!("{}", "Home Manager:".cyan().bold());
    if nix_status.hm_installed {
        println!(
            "  {} Installed: {}",
            "✓".green(),
            nix_status.hm_version.as_deref().unwrap_or("unknown")
        );
        println!(
            "  {} Enabled in config: {}",
            "✓".green(),
            if config.nix.home_manager_enabled {
                "yes".green().to_string()
            } else {
                "no".yellow().to_string()
            }
        );
        println!(
            "  {} Flake enabled: {}",
            "✓".green(),
            if nix_status.flake_enabled {
                "yes".green().to_string()
            } else {
                "no".to_string()
            }
        );
        if nix_status.flake_enabled {
            println!(
                "  {} flake.nix: {}",
                "✓".green(),
                if nix_status.flake_nix_exists {
                    "exists".green().to_string()
                } else {
                    "not found".yellow().to_string()
                }
            );
        }
        println!(
            "  {} home.nix: {}",
            "✓".green(),
            if nix_status.home_nix_exists {
                "exists".green().to_string()
            } else {
                "not found".yellow().to_string()
            }
        );
        println!(
            "  {} mdots-packages.nix: {}",
            "✓".green(),
            if nix_status.mdots_packages_exists {
                "exists".green().to_string()
            } else {
                "not found".yellow().to_string()
            }
        );
    } else {
        println!("  {} Not installed", "✗".red());
    }

    Ok(())
}
