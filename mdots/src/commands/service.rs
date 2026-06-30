//! Service profile management commands
//!
//! Commands for listing, enabling, and disabling service profiles.

use anyhow::{Context, Result};
use colored::*;
use serde::Serialize;
use std::io::{self, Write};

use crate::config::{load_config, ConfigPaths};
use crate::service_profile::ServiceProfileManager;

#[derive(Serialize)]
struct ServiceListOutput {
    profiles: Vec<ServiceProfileJson>,
}

#[derive(Serialize)]
struct ServiceProfileJson {
    name: String,
    description: String,
    enabled_services: Vec<String>,
    disabled_services: Vec<String>,
    conflicts: Vec<String>,
    is_enabled: bool,
}

#[derive(Serialize)]
struct ServiceActionOutput {
    success: bool,
    message: String,
    profile: String,
    action: String,
}

/// List all available service profiles
pub fn list(paths: &ConfigPaths, json: bool) -> Result<()> {
    let config = load_config(paths)?;
    let manager = ServiceProfileManager::new(paths.clone());
    let profiles = manager.list_profiles(&config.enabled_service_profiles)?;

    if profiles.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&ServiceListOutput { profiles: vec![] })?
            );
        } else {
            println!("{}", "No service profiles found".yellow());
            println!();
            println!(
                "Create service profiles in: {}",
                paths.services_dir().display()
            );
            println!();
            println!("Example service profile (services/gaming.lua):");
            println!(
                "{}",
                r#"
return {
    description = "Gaming services",
    services = {
        enabled = { "sunshine.service" },
        disabled = {},
    },
}
"#
                .dimmed()
            );
        }
        return Ok(());
    }

    if json {
        let profile_infos: Vec<ServiceProfileJson> = profiles
            .iter()
            .map(|p| ServiceProfileJson {
                name: p.name.clone(),
                description: p.description.clone(),
                enabled_services: p.enabled_services.clone(),
                disabled_services: p.disabled_services.clone(),
                conflicts: p.conflicts.clone(),
                is_enabled: p.is_enabled,
            })
            .collect();

        let output = ServiceListOutput {
            profiles: profile_infos,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", "=== Service Profiles ===".blue().bold());
        println!();

        for profile in profiles {
            let status = if profile.is_enabled {
                "enabled".green()
            } else {
                "disabled".yellow()
            };

            println!("  {} [{}]", profile.name.blue(), status);

            if !profile.description.is_empty() {
                println!("    {}", profile.description);
            }

            let svc_count = profile.enabled_services.len() + profile.disabled_services.len();
            println!("    Services: {}", svc_count);

            if !profile.conflicts.is_empty() {
                println!(
                    "    {}: {}",
                    "Conflicts with".red(),
                    profile.conflicts.join(", ")
                );
            }

            println!();
        }
    }

    Ok(())
}

/// Enable a service profile
pub fn enable(paths: &ConfigPaths, profile_name: &str, json: bool) -> Result<()> {
    let mut config = load_config(paths)?;
    let manager = ServiceProfileManager::new(paths.clone());

    // Check if profile exists
    let _profile = manager.load_profile(profile_name)?;

    // Check if already enabled
    if config
        .enabled_service_profiles
        .contains(&profile_name.to_string())
    {
        let message = format!("Service profile '{}' is already enabled", profile_name);
        if json {
            let output = ServiceActionOutput {
                success: false,
                message,
                profile: profile_name.to_string(),
                action: "enable".to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("{}", message.yellow());
        }
        return Ok(());
    }

    // Check for conflicts
    let conflicts = manager.check_conflicts(profile_name, &config.enabled_service_profiles)?;

    if !conflicts.is_empty() {
        if json {
            let output = ServiceActionOutput {
                success: false,
                message: format!(
                    "Service profile '{}' conflicts with enabled profile(s): {}",
                    profile_name,
                    conflicts.join(", ")
                ),
                profile: profile_name.to_string(),
                action: "enable".to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
            return Ok(());
        } else {
            println!(
                "{}",
                format!(
                    "Service profile '{}' conflicts with enabled profile(s): {}",
                    profile_name,
                    conflicts.join(", ")
                )
                .red()
            );

            print!("Disable conflicting profile(s)? [y/N] ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            if input.trim().to_lowercase() == "y" {
                for conflict in conflicts {
                    config.enabled_service_profiles.retain(|p| p != &conflict);
                    println!("{}", format!("Disabled profile '{}'", conflict).yellow());
                }
            } else {
                println!("{}", "Cancelled".yellow());
                return Ok(());
            }
        }
    }

    // Enable profile
    config
        .enabled_service_profiles
        .push(profile_name.to_string());

    // Save config
    save_config(paths, &config)?;

    if json {
        let output = ServiceActionOutput {
            success: true,
            message: format!("Enabled service profile '{}'", profile_name),
            profile: profile_name.to_string(),
            action: "enable".to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "{}",
            format!("✓ Enabled service profile '{}'", profile_name).green()
        );
        println!("Run 'mdots sync' to apply service changes");
    }

    Ok(())
}

/// Disable a service profile
pub fn disable(paths: &ConfigPaths, profile_name: &str, json: bool) -> Result<()> {
    let mut config = load_config(paths)?;

    // Check if enabled
    if !config
        .enabled_service_profiles
        .contains(&profile_name.to_string())
    {
        let message = format!("Service profile '{}' is not enabled", profile_name);
        if json {
            let output = ServiceActionOutput {
                success: false,
                message,
                profile: profile_name.to_string(),
                action: "disable".to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("{}", message.yellow());
        }
        return Ok(());
    }

    // Disable profile
    config
        .enabled_service_profiles
        .retain(|p| p != profile_name);

    // Save config
    save_config(paths, &config)?;

    if json {
        let output = ServiceActionOutput {
            success: true,
            message: format!("Disabled service profile '{}'", profile_name),
            profile: profile_name.to_string(),
            action: "disable".to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "{}",
            format!("✓ Disabled service profile '{}'", profile_name).green()
        );
        println!("Run 'mdots sync' to apply service changes");
    }

    Ok(())
}

/// Show details of a service profile
pub fn show(paths: &ConfigPaths, profile_name: &str) -> Result<()> {
    let config = load_config(paths)?;
    let manager = ServiceProfileManager::new(paths.clone());
    let profile = manager.load_profile(profile_name)?;

    let is_enabled = config
        .enabled_service_profiles
        .contains(&profile_name.to_string());

    println!(
        "{}",
        format!("=== Service Profile: {} ===", profile.name)
            .blue()
            .bold()
    );
    println!();

    let status = if is_enabled {
        "enabled".green()
    } else {
        "disabled".yellow()
    };
    println!("Status: {}", status);

    if !profile.description.is_empty() {
        println!("Description: {}", profile.description);
    }

    println!("Path: {}", profile.path.display());

    if !profile.services.enabled.is_empty() {
        println!();
        println!("{}", "Services to enable:".green());
        for svc in &profile.services.enabled {
            println!("  + {}", svc);
        }
    }

    if !profile.services.disabled.is_empty() {
        println!();
        println!("{}", "Services to disable:".red());
        for svc in &profile.services.disabled {
            println!("  - {}", svc);
        }
    }

    if !profile.conflicts.is_empty() {
        println!();
        println!("{}", "Conflicts with:".yellow());
        for conflict in &profile.conflicts {
            println!("  ! {}", conflict);
        }
    }

    Ok(())
}

/// Interactive enable with fzf
pub fn enable_interactive(paths: &ConfigPaths) -> Result<()> {
    use std::process::{Command, Stdio};

    // Check if fzf is installed
    if which::which("fzf").is_err() {
        anyhow::bail!(
            "fzf is not installed. Please install fzf to use interactive service profile selection."
        );
    }

    let config = load_config(paths)?;
    let manager = ServiceProfileManager::new(paths.clone());
    let profiles = manager.list_profiles(&config.enabled_service_profiles)?;

    // Filter to disabled profiles
    let disabled: Vec<_> = profiles.iter().filter(|p| !p.is_enabled).collect();

    if disabled.is_empty() {
        println!("{}", "All service profiles are already enabled".yellow());
        return Ok(());
    }

    // Run fzf
    let mut fzf = Command::new("fzf")
        .args([
            "--multi",
            "--header=→ Select service profiles to enable\nℹ Use TAB to select multiple, ENTER to confirm",
            "--prompt=Select profiles > ",
            "--height=100%",
            "--border=rounded",
            "--border-label= mdots service enable ",
            "--border-label-pos=2",
            "--color=border:blue,label:cyan",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to run fzf")?;

    // Write profile list to fzf
    {
        let stdin = fzf.stdin.as_mut().context("Failed to open fzf stdin")?;
        for profile in &disabled {
            writeln!(stdin, "{}", profile.name)?;
        }
    }

    let output = fzf.wait_with_output().context("Failed to wait for fzf")?;

    if !output.status.success() {
        println!("{} Selection cancelled", "✗".yellow());
        return Ok(());
    }

    let selected = String::from_utf8(output.stdout)
        .context("Failed to parse fzf output")?
        .trim()
        .to_string();

    if selected.is_empty() {
        println!("{} No profiles selected", "✗".yellow());
        return Ok(());
    }

    let profile_names: Vec<&str> = selected.lines().collect();

    println!();
    println!(
        "{} Enabling {} service profile(s)...",
        "→".blue(),
        profile_names.len()
    );
    println!();

    for profile_name in profile_names {
        if let Err(e) = enable(paths, profile_name, false) {
            eprintln!("{} Failed to enable: {} - {}", "✗".red(), profile_name, e);
        }
    }

    Ok(())
}

/// Interactive disable with fzf
pub fn disable_interactive(paths: &ConfigPaths) -> Result<()> {
    use std::process::{Command, Stdio};

    // Check if fzf is installed
    if which::which("fzf").is_err() {
        anyhow::bail!(
            "fzf is not installed. Please install fzf to use interactive service profile selection."
        );
    }

    let config = load_config(paths)?;

    if config.enabled_service_profiles.is_empty() {
        println!("{}", "No service profiles are currently enabled".yellow());
        return Ok(());
    }

    // Run fzf
    let mut fzf = Command::new("fzf")
        .args([
            "--multi",
            "--header=→ Select service profiles to disable\nℹ Use TAB to select multiple, ENTER to confirm",
            "--prompt=Select profiles > ",
            "--height=100%",
            "--border=rounded",
            "--border-label= mdots service disable ",
            "--border-label-pos=2",
            "--color=border:blue,label:cyan",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to run fzf")?;

    // Write enabled profiles to fzf
    {
        let stdin = fzf.stdin.as_mut().context("Failed to open fzf stdin")?;
        for profile_name in &config.enabled_service_profiles {
            writeln!(stdin, "{}", profile_name)?;
        }
    }

    let output = fzf.wait_with_output().context("Failed to wait for fzf")?;

    if !output.status.success() {
        println!("{} Selection cancelled", "✗".yellow());
        return Ok(());
    }

    let selected = String::from_utf8(output.stdout)
        .context("Failed to parse fzf output")?
        .trim()
        .to_string();

    if selected.is_empty() {
        println!("{} No profiles selected", "✗".yellow());
        return Ok(());
    }

    let profile_names: Vec<&str> = selected.lines().collect();

    println!();
    println!(
        "{} Disabling {} service profile(s)...",
        "→".blue(),
        profile_names.len()
    );
    println!();

    for profile_name in profile_names {
        if let Err(e) = disable(paths, profile_name, false) {
            eprintln!("{} Failed to disable: {} - {}", "✗".red(), profile_name, e);
        }
    }

    Ok(())
}

/// Save config - reuses logic from module.rs but handles service profiles
fn save_config(paths: &ConfigPaths, config: &crate::config::Config) -> Result<()> {
    // Determine the correct file to save to
    let save_path = if let Ok(pointer_content) = std::fs::read_to_string(&paths.config_file) {
        if let Ok(pointer_config) = serde_yaml::from_str::<crate::config::Config>(&pointer_content)
        {
            // Check if it's a pointer config
            #[allow(deprecated)]
            let is_pointer = pointer_config.enabled_modules.is_empty()
                && pointer_config.packages.is_empty()
                && pointer_config.exclude.is_empty()
                && pointer_config.additional_packages.is_empty()
                && pointer_config.backup_tool.is_none()
                && pointer_config.description.is_empty()
                && pointer_config.import.is_empty();

            if is_pointer {
                paths.host_packages_file(&config.host)
            } else {
                paths.config_file.clone()
            }
        } else {
            paths.config_file.clone()
        }
    } else {
        paths.config_file.clone()
    };

    // Check if target is a Lua file - we cannot auto-modify Lua configs
    if save_path.extension().and_then(|e| e.to_str()) == Some("lua") {
        anyhow::bail!(
            "Cannot automatically save config changes to Lua file: {}\n\n\
             Lua configs contain code and cannot be auto-modified.\n\
             Please edit the file manually to update enabled_service_profiles.\n\n\
             Alternatively, rename to .yaml to use automatic management:\n\
             mv {} {}",
            save_path.display(),
            save_path.display(),
            save_path.with_extension("yaml").display()
        );
    }

    let yaml = serde_yaml::to_string(config).context("Failed to serialize config")?;
    std::fs::write(&save_path, yaml)
        .context(format!("Failed to write config file: {:?}", save_path))?;
    Ok(())
}
