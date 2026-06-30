use anyhow::Result;
use colored::*;
use serde::Serialize;

use crate::config::{load_config, resolve_config_path, ConfigPaths, PackageManagerType};
use crate::package::PackageManager;

/// Parse /etc/os-release and get the NAME field
fn get_distro_name() -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(name) = line.strip_prefix("NAME=") {
                return name.trim_matches('"').to_string();
            }
        }
    }
    "Unknown".to_string()
}

#[derive(Serialize)]
struct StatusOutput {
    configuration: ConfigInfo,
    enabled_modules: Vec<String>,
    declared_packages: PackageStats,
    installed_packages: InstalledStats,
    /// True if a previous `dcli sync` did not finish (system may be partial).
    sync_interrupted: bool,
}

#[derive(Serialize)]
struct ConfigInfo {
    host: String,
    distro: String,
    config_dir: String,
    config_file: String,
    config_type: String,
    backup_tool: String,
    flatpak_scope: String,
    module_processing: String,
    auto_prune: bool,
    services_enabled: usize,
    services_disabled: usize,
    default_apps_configured: bool,
}

#[derive(Serialize)]
struct PackageStats {
    pacman: usize,
    flatpak: usize,
    nix: usize,
    total: usize,
}

#[derive(Serialize)]
struct InstalledStats {
    native: usize,
    flatpak: usize,
    nix: usize,
}

pub fn run(paths: &ConfigPaths, json: bool) -> Result<()> {
    // Load configuration
    let config = load_config(paths)?;
    let config_path = resolve_config_path(paths)?;
    let config_type = match config_path.extension().and_then(|e| e.to_str()) {
        Some("lua") => "lua",
        _ => "yaml",
    };

    // Get declared packages
    let pkg_manager = PackageManager::new(paths.clone());
    let declared = pkg_manager.get_declared_packages(&config)?;

    let pacman_count = declared
        .iter()
        .filter(|p| matches!(p.package_type, crate::config::PackageType::Native))
        .count();
    let flatpak_count = declared
        .iter()
        .filter(|p| matches!(p.package_type, crate::config::PackageType::Flatpak))
        .count();
    let nix_declared_count = declared
        .iter()
        .filter(|p| matches!(p.package_type, crate::config::PackageType::Nix))
        .count();

    // Check installed packages
    let installed = pkg_manager.get_installed_native_packages(&config)?;

    // Check installed flatpaks (both scopes)
    let mut installed_flatpaks: Vec<String> = pkg_manager
        .get_installed_flatpaks("--user")
        .unwrap_or_default();
    let system_flatpaks = pkg_manager
        .get_installed_flatpaks("--system")
        .unwrap_or_default();
    for pkg in system_flatpaks {
        if !installed_flatpaks.contains(&pkg) {
            installed_flatpaks.push(pkg);
        }
    }

    // Count installed nix packages from dcli-packages.nix and packages.nix
    let nix_installed_count = if config.nix.enabled {
        let host_dir = if crate::nix::use_per_host_structure(paths) {
            crate::nix::home_manager_host_dir(paths, &config.host)
        } else {
            paths.home_manager_dir().to_path_buf()
        };
        count_nix_packages_in_file(&host_dir.join("dcli-packages.nix"))
            + count_nix_packages_in_file(&host_dir.join("packages.nix"))
    } else {
        0
    };

    // Detect package manager type for display
    let pm_type = crate::config::resolve_package_manager(&config)?;
    let pm_label = match pm_type {
        PackageManagerType::Pacman => "Pacman packages",
    };

    // Check if default apps are configured
    let default_apps_configured =
        !config.default_apps.to_apps_map().is_empty() || !config.default_apps.mime_types.is_empty();

    // A leftover marker means the last sync did not finish cleanly.
    let sync_interrupted = crate::commands::sync::sync_was_interrupted(&paths.state_dir);

    if json {
        // Output JSON format
        #[allow(deprecated)]
        let backup_tool_display = config
            .backup_tool
            .as_deref()
            .or(config.system_backups.tool.as_deref())
            .unwrap_or("auto-detect")
            .to_string();

        let output = StatusOutput {
            configuration: ConfigInfo {
                host: config.host.clone(),
                distro: get_distro_name(),
                config_dir: paths.config_dir.display().to_string(),
                config_file: config_path.display().to_string(),
                config_type: config_type.to_string(),
                backup_tool: backup_tool_display,
                flatpak_scope: format!("{:?}", config.flatpak_scope),
                module_processing: format!("{:?}", config.module_processing),
                auto_prune: config.auto_prune,
                services_enabled: config.services.enabled.len(),
                services_disabled: config.services.disabled.len(),
                default_apps_configured,
            },
            enabled_modules: config.enabled_modules.clone(),
            declared_packages: PackageStats {
                pacman: pacman_count,
                flatpak: flatpak_count,
                nix: nix_declared_count,
                total: declared.len(),
            },
            installed_packages: InstalledStats {
                native: installed.len(),
                flatpak: installed_flatpaks.len(),
                nix: nix_installed_count,
            },
            sync_interrupted,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        // Human-readable format
        println!("{}", "=== dcli Status ===".blue().bold());
        println!();

        if sync_interrupted {
            println!(
                "{}",
                "⚠ A previous sync did not finish — your system may be partially applied."
                    .yellow()
                    .bold()
            );
            println!("  Run 'dcli sync' to reconcile it to your configuration.");
            println!();
        }

        // Display configuration
        println!("{}", "Configuration:".cyan().bold());
        println!("  Host: {}", config.host.green());
        println!("  Distro: {}", get_distro_name().green());
        println!("  Config directory: {}", paths.config_dir.display());
        println!("  Config file: {}", config_path.display());
        println!("  Config type: {}", config_type.yellow());

        #[allow(deprecated)]
        let backup_tool_display = config
            .backup_tool
            .as_deref()
            .or(config.system_backups.tool.as_deref())
            .unwrap_or("auto-detect");

        println!("  Backup tool: {}", backup_tool_display.yellow());
        println!(
            "  Flatpak scope: {}",
            format!("{:?}", config.flatpak_scope).yellow()
        );
        println!(
            "  Module processing: {}",
            format!("{:?}", config.module_processing).yellow()
        );
        println!(
            "  Auto-prune: {}",
            if config.auto_prune {
                "enabled".green()
            } else {
                "disabled".yellow()
            }
        );
        println!();

        // Display enabled modules
        if config.enabled_modules.is_empty() {
            println!("{}", "Enabled modules: None".yellow());
        } else {
            println!(
                "{} ({})",
                "Enabled modules:".cyan().bold(),
                config.enabled_modules.len()
            );
            for module in &config.enabled_modules {
                println!("  {} {}", "✓".green(), module.blue());
            }
        }
        println!();

        // Display services configuration
        if !config.services.enabled.is_empty() || !config.services.disabled.is_empty() {
            println!("{}", "Services:".cyan().bold());
            if !config.services.enabled.is_empty() {
                println!(
                    "  Enabled: {}",
                    config.services.enabled.len().to_string().green()
                );
            }
            if !config.services.disabled.is_empty() {
                println!(
                    "  Disabled: {}",
                    config.services.disabled.len().to_string().yellow()
                );
            }
            println!();
        }

        // Display default apps configuration
        if default_apps_configured {
            let apps_map = config.default_apps.to_apps_map();
            let total_defaults = apps_map.len() + config.default_apps.mime_types.len();
            println!("{}", "Default Applications:".cyan().bold());
            println!("  Configured: {}", total_defaults.to_string().green());
            println!(
                "  Scope: {}",
                format!("{:?}", config.default_apps.scope).yellow()
            );
            println!();
        }

        println!("{}", "Declared packages:".cyan().bold());
        println!("  {}: {}", pm_label, pacman_count.to_string().green());
        println!("  Flatpak packages: {}", flatpak_count.to_string().green());
        if nix_declared_count > 0 {
            println!("  Nix packages: {}", nix_declared_count.to_string().green());
        }
        println!("  Total: {}", declared.len().to_string().green().bold());
        println!();

        println!("{}", "Installed packages:".cyan().bold());
        println!("  {}: {}", pm_label, installed.len().to_string().green());
        println!(
            "  Flatpak packages: {}",
            installed_flatpaks.len().to_string().green()
        );
        if nix_installed_count > 0 {
            println!(
                "  Nix packages: {}",
                nix_installed_count.to_string().green()
            );
        }

        // Nix status
        if config.nix.enabled || config.nix.home_manager_enabled {
            println!();
            println!("{}", "Nix & Home Manager:".cyan().bold());
            println!(
                "  Nix enabled: {}",
                if config.nix.enabled {
                    "yes".green().to_string()
                } else {
                    "no".yellow().to_string()
                }
            );
            println!(
                "  Home Manager enabled: {}",
                if config.nix.home_manager_enabled {
                    "yes".green().to_string()
                } else {
                    "no".yellow().to_string()
                }
            );
            println!(
                "  Flake enabled: {}",
                if config.nix.flake_enabled {
                    "yes".green().to_string()
                } else {
                    "no".to_string()
                }
            );
            println!("  Nixpkgs channel/input: {}", config.nix.nixpkgs_channel);
            println!(
                "  Home Manager channel/input: {}",
                config.nix.home_manager_channel
            );

            if crate::nix::is_nix_installed() {
                println!("  Nix installed: {}", "yes".green());
            } else {
                println!("  Nix installed: {}", "no".red());
            }

            if crate::nix::is_home_manager_installed() {
                println!("  Home Manager installed: {}", "yes".green());
            } else {
                println!("  Home Manager installed: {}", "no".red());
            }

            if config.nix.flake_enabled {
                let hn = paths.home_manager_dir();
                println!(
                    "  flake.nix: {}",
                    if hn.join("flake.nix").exists() {
                        "exists".green().to_string()
                    } else {
                        "not found".yellow().to_string()
                    }
                );
                println!(
                    "  flake.lock: {}",
                    if hn.join("flake.lock").exists() {
                        "exists".green().to_string()
                    } else {
                        "not found".yellow().to_string()
                    }
                );
            }
        }
    }

    Ok(())
}

/// Count nix package entries in a dcli-packages.nix or packages.nix file
fn count_nix_packages_in_file(path: &std::path::Path) -> usize {
    if !path.exists() {
        return 0;
    }
    if let Ok(content) = std::fs::read_to_string(path) {
        let mut inside = false;
        let mut count = 0;
        for line in content.lines() {
            let trimmed = line.trim();

            if !inside && trimmed.contains('[') {
                inside = true;
                // Check for inline packages or bundled ]; on the same line
                if let Some(pos) = trimmed.find('[') {
                    let after_bracket = trimmed[pos + 1..].trim();
                    let after_bracket = after_bracket.trim_end_matches(';');
                    if !after_bracket.is_empty() {
                        for word in after_bracket.split_whitespace() {
                            if !word.starts_with("//") && !word.starts_with('#') && word != "]" {
                                count += 1;
                            }
                        }
                    }
                }
                if trimmed.contains("];") {
                    break;
                }
                continue;
            }

            if inside && trimmed == "];" {
                break;
            }

            if inside
                && !trimmed.is_empty()
                && !trimmed.starts_with("//")
                && !trimmed.starts_with('#')
                && trimmed.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-')
            {
                count += 1;
            }
        }
        count
    } else {
        0
    }
}
