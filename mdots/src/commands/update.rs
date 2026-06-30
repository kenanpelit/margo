use anyhow::{Context, Result};
use colored::*;
use std::process::Command;

use crate::config::{load_config, ConfigPaths};

pub fn run(paths: &ConfigPaths, no_backup: bool, no_hooks: bool, devel: bool) -> Result<()> {
    // Load config and create backend
    let config = load_config(paths)?;
    let backend = crate::backend::create_backend(&config)?;

    println!();
    println!("{}", "=== System Update ===".blue().bold());

    // Run pre-update hook
    if !no_hooks {
        run_pre_update_hook(paths, &config)?;
    }

    // Create backup unless --no-backup
    if !no_backup {
        match crate::commands::backup::create_backup_if_enabled(
            paths,
            "update",
            "dcli update autobackup",
        ) {
            Ok(true) => {
                println!();
            }
            Ok(false) => {
                // Backup disabled in config
                println!("{}", "System backup disabled in configuration".dimmed());
                println!();
            }
            Err(e) => {
                println!(
                    "{}",
                    format!("⚠ Warning: Failed to create system backup: {}", e).yellow()
                );
                println!("{}", "  Continuing with update...".yellow());
                println!();
            }
        }
    }

    // Run system update
    // Check if devel flag should be used (from CLI flag or config)
    // Note: devel flag only applies to pacman/AUR helpers
    let use_devel = devel || config.update_hooks.devel;

    if use_devel && config.package_manager == Some(crate::config::PackageManagerType::Pacman) {
        println!("{}", "  Using --devel flag to update VCS packages".dimmed());
        println!();
    }

    // Execute system update via backend
    let success = backend.system_update(use_devel)?;

    // Show completion message
    println!();
    if success {
        println!("{}", "✓ System update complete!".green());
    } else {
        println!("{}", "⚠ System update completed with errors".yellow());
        anyhow::bail!("Update failed");
    }

    // Update flatpak if installed
    if is_flatpak_installed() {
        println!();
        println!("{}", "=== Flatpak Update ===".blue().bold());

        let flatpak_status = Command::new("flatpak")
            .args(["update", "-y"])
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .context("Failed to run flatpak update")?;

        println!();
        if flatpak_status.success() {
            println!("{}", "✓ Flatpak update complete!".green());
        } else {
            println!("{}", "⚠ Flatpak update completed with errors".yellow());
        }
    }

    // Run post-update hook
    if !no_hooks {
        run_post_update_hook(paths, &config)?;
    }

    Ok(())
}

fn is_flatpak_installed() -> bool {
    which::which("flatpak").is_ok()
}

fn run_pre_update_hook(paths: &ConfigPaths, config: &crate::config::Config) -> Result<()> {
    if config.update_hooks.pre_update.is_none() {
        return Ok(());
    }

    let hook_script = config.update_hooks.pre_update.as_ref().unwrap();
    let hook_path = if std::path::Path::new(hook_script).is_absolute() {
        std::path::PathBuf::from(hook_script)
    } else {
        paths.config_dir.join(hook_script)
    };

    if !hook_path.exists() || hook_path.is_dir() {
        return Ok(());
    }

    execute_update_hook(
        paths,
        "update_pre",
        &hook_path,
        &config.update_hooks.behavior,
        "Pre-update",
    )
}

fn run_post_update_hook(paths: &ConfigPaths, config: &crate::config::Config) -> Result<()> {
    if config.update_hooks.post_update.is_none() {
        return Ok(());
    }

    let hook_script = config.update_hooks.post_update.as_ref().unwrap();
    let hook_path = if std::path::Path::new(hook_script).is_absolute() {
        std::path::PathBuf::from(hook_script)
    } else {
        paths.config_dir.join(hook_script)
    };

    if !hook_path.exists() || hook_path.is_dir() {
        return Ok(());
    }

    execute_update_hook(
        paths,
        "update_post",
        &hook_path,
        &config.update_hooks.behavior,
        "Post-update",
    )
}

fn execute_update_hook(
    paths: &ConfigPaths,
    state_key: &str,
    hook_path: &std::path::Path,
    behavior: &str,
    hook_type: &str,
) -> Result<()> {
    use crate::commands::sync::{
        check_hook_status, mark_hook_executed, mark_hook_skipped, HookStatus,
    };
    use std::io::{self, Write};

    // Check hook status
    let status = check_hook_status(paths, state_key, &hook_path.to_path_buf())?;

    let should_run = match status {
        HookStatus::Executed => behavior == "always",
        HookStatus::Skipped => behavior == "always",
        HookStatus::Modified | HookStatus::NotRun => {
            if behavior == "skip" {
                false
            } else if behavior == "once" || behavior == "always" {
                true
            } else {
                // behavior == "ask"
                println!();
                println!("{}", format!("{} hook", hook_type).blue().bold());
                println!("  Script: {}", hook_path.display().to_string().dimmed());
                println!();
                print!("Run this hook? [Y/n/s] (Y=yes, n=no this time, s=skip permanently): ");
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let choice = input.trim().to_lowercase();

                match choice.as_str() {
                    "n" => {
                        println!("{}", "Skipping this time".yellow());
                        false
                    }
                    "s" => {
                        println!("{}", "Marking as permanently skipped".yellow());
                        mark_hook_skipped(paths, state_key)?;
                        false
                    }
                    "" | "y" => true,
                    _ => {
                        println!("{}", "Invalid choice, skipping".yellow());
                        false
                    }
                }
            }
        }
    };

    if !should_run {
        return Ok(());
    }

    println!();
    println!(
        "{}",
        format!("Executing {} hook...", hook_type.to_lowercase()).blue()
    );

    let status = Command::new("sudo")
        .arg("bash")
        .arg(hook_path)
        .current_dir(&paths.config_dir)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .context("Failed to execute hook script")?;

    if !status.success() {
        anyhow::bail!("Hook script failed");
    }

    mark_hook_executed(paths, state_key, &hook_path.to_path_buf())?;
    println!("{}", "✓ Hook completed successfully".green());

    Ok(())
}
