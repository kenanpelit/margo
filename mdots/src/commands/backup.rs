use anyhow::Result;
use colored::*;
use std::process::Command;

use crate::config::ConfigPaths;

/// Get the configured or detected backup tool
fn get_backup_tool(paths: &ConfigPaths) -> Result<String> {
    // Check config file for backup_tool setting
    if let Ok(config) = crate::config::load_config(paths) {
        // Try new structure first
        if let Some(tool) = &config.system_backups.tool {
            if which::which(tool).is_ok() {
                return Ok(tool.clone());
            } else {
                eprintln!(
                    "{}",
                    format!("Warning: Configured backup tool '{}' not found", tool).yellow()
                );
            }
        }

        // Fallback to old structure (for backwards compatibility)
        #[allow(deprecated)]
        if let Some(tool) = &config.backup_tool {
            if which::which(tool).is_ok() {
                return Ok(tool.clone());
            } else {
                eprintln!(
                    "{}",
                    format!("Warning: Configured backup tool '{}' not found", tool).yellow()
                );
            }
        }
    }

    // Auto-detect
    if which::which("timeshift").is_ok() {
        Ok("timeshift".to_string())
    } else if which::which("snapper").is_ok() {
        Ok("snapper".to_string())
    } else {
        anyhow::bail!("No backup tool found. Please install timeshift or snapper.");
    }
}

/// Get snapper configuration to use
fn get_snapper_config(paths: &ConfigPaths) -> String {
    // Try to read from config first
    if let Ok(config) = crate::config::load_config(paths) {
        // Check new structure first
        if !config.system_backups.snapper_config.is_empty()
            && config.system_backups.snapper_config != "root"
        {
            return config.system_backups.snapper_config;
        }

        // Fallback to old structure (for backwards compatibility)
        #[allow(deprecated)]
        if !config.snapper_config.is_empty() && config.snapper_config != "root" {
            return config.snapper_config;
        }
    }

    // Try to read from snapper configs (auto-detection)
    let output = Command::new("snapper").args(["list-configs"]).output();

    if let Ok(output) = output {
        if output.status.success() {
            let configs = String::from_utf8_lossy(&output.stdout);
            // Look for "root" config first, otherwise use first available
            for line in configs.lines().skip(2) {
                // Skip header lines
                let parts: Vec<&str> = line.split_whitespace().collect();
                if !parts.is_empty() {
                    let config_name = parts[0];
                    if config_name == "root" {
                        return config_name.to_string();
                    }
                }
            }
            // If no "root" config, use first available
            for line in configs.lines().skip(2) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if !parts.is_empty() {
                    return parts[0].to_string();
                }
            }
        }
    }

    // Default to "root"
    "root".to_string()
}

/// Create a backup snapshot if enabled by config
/// Returns Ok(true) if backup was created, Ok(false) if skipped, Err if failed
pub fn create_backup_if_enabled(
    paths: &ConfigPaths,
    operation: &str, // "sync" or "update"
    comment: &str,
) -> Result<bool> {
    let config = crate::config::load_config(paths)?;

    // Check if system backups are enabled globally
    if !config.system_backups.enabled {
        return Ok(false);
    }

    // Check operation-specific setting
    let should_backup = match operation {
        "sync" => config.system_backups.backup_on_sync,
        "update" => config.system_backups.backup_on_update,
        _ => false,
    };

    if !should_backup {
        return Ok(false);
    }

    // Create the backup
    let backup_tool = get_backup_tool(paths)?;

    // Prompt for sudo password first (if not cached) so the spinner only
    // shows during the actual backup, not during the sudo password prompt
    let sudo_status = Command::new("sudo").arg("-v").status()?;
    if !sudo_status.success() {
        anyhow::bail!("sudo authentication failed");
    }

    let spinner = crate::progress::create_spinner(&format!("Creating {} snapshot...", backup_tool));

    match backup_tool.as_str() {
        "timeshift" => {
            let status = Command::new("sudo")
                .args(["timeshift", "--create", "--comments", comment, "--scripted"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?;

            if status.success() {
                // Rotate old backups if max_backups is set
                rotate_system_backups(paths, &backup_tool, config.system_backups.max_backups)?;
                spinner.finish_with_message("✓ Timeshift snapshot created");
                Ok(true)
            } else {
                spinner.finish_with_message("✗ Failed to create timeshift snapshot");
                anyhow::bail!("Failed to create timeshift snapshot");
            }
        }
        "snapper" => {
            let snapper_cfg = get_snapper_config(paths);
            let status = Command::new("sudo")
                .args(["snapper", "-c", &snapper_cfg, "create", "-d", comment])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?;

            if status.success() {
                // Rotate old backups if max_backups is set
                rotate_system_backups(paths, &backup_tool, config.system_backups.max_backups)?;
                spinner.finish_with_message("✓ Snapper snapshot created");
                Ok(true)
            } else {
                spinner.finish_with_message("✗ Failed to create snapper snapshot");
                anyhow::bail!("Failed to create snapper snapshot");
            }
        }
        _ => {
            spinner.finish_with_message(format!("✗ Unknown backup tool: {}", backup_tool));
            anyhow::bail!("Unknown backup tool: {}", backup_tool)
        }
    }
}

/// Rotate system backups according to max_backups setting
/// Only deletes dcli-created snapshots (identified by comment containing "dcli")
fn rotate_system_backups(paths: &ConfigPaths, backup_tool: &str, max_backups: u32) -> Result<()> {
    if max_backups == 0 {
        return Ok(()); // Unlimited backups
    }

    match backup_tool {
        "timeshift" => rotate_timeshift_backups(max_backups),
        "snapper" => rotate_snapper_backups(paths, max_backups),
        _ => Ok(()),
    }
}

/// Rotate timeshift backups - keeps only max_backups dcli snapshots
fn rotate_timeshift_backups(max_backups: u32) -> Result<()> {
    // Get list of timeshift snapshots
    let output = Command::new("sudo")
        .args(["timeshift", "--list", "--scripted"])
        .output()?;

    if !output.status.success() {
        return Ok(()); // Can't list, skip rotation
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse timeshift output to find dcli snapshots
    // Format: snapshot_name | date | tags | description
    let mut dcli_snapshots: Vec<String> = Vec::new();

    for line in stdout.lines() {
        // Skip header lines and empty lines
        if line.trim().is_empty() || line.contains("Snapshot") || line.starts_with('-') {
            continue;
        }

        // Check if this is a dcli snapshot (comment contains "dcli")
        if line.to_lowercase().contains("dcli") {
            // Extract snapshot name (first field)
            if let Some(snapshot_name) = line.split_whitespace().next() {
                dcli_snapshots.push(snapshot_name.to_string());
            }
        }
    }

    // Sort by name (which includes timestamp)
    dcli_snapshots.sort();

    // Delete oldest snapshots if we exceed max_backups
    while dcli_snapshots.len() > max_backups as usize {
        if let Some(snapshot) = dcli_snapshots.first() {
            let _ = Command::new("sudo")
                .args([
                    "timeshift",
                    "--delete",
                    "--snapshot",
                    snapshot,
                    "--scripted",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            dcli_snapshots.remove(0);
        }
    }

    Ok(())
}

/// Rotate snapper backups - keeps only max_backups dcli snapshots
fn rotate_snapper_backups(paths: &ConfigPaths, max_backups: u32) -> Result<()> {
    let snapper_cfg = get_snapper_config(paths);

    // Get list of snapper snapshots
    let output = Command::new("sudo")
        .args([
            "snapper",
            "-c",
            &snapper_cfg,
            "list",
            "--columns",
            "number,description",
        ])
        .output()?;

    if !output.status.success() {
        return Ok(()); // Can't list, skip rotation
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse snapper output to find dcli snapshots
    // Format: number | description
    let mut dcli_snapshots: Vec<u32> = Vec::new();

    for line in stdout.lines() {
        // Skip header lines
        if line.contains("Number") || line.starts_with('-') || line.trim().is_empty() {
            continue;
        }

        // Check if this is a dcli snapshot (description contains "dcli")
        if line.to_lowercase().contains("dcli") {
            // Extract snapshot number (first field)
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(num_str) = parts.first() {
                if let Ok(num) = num_str.parse::<u32>() {
                    dcli_snapshots.push(num);
                }
            }
        }
    }

    // Sort by number (older snapshots have lower numbers)
    dcli_snapshots.sort();

    // Delete oldest snapshots if we exceed max_backups
    while dcli_snapshots.len() > max_backups as usize {
        if let Some(snapshot_num) = dcli_snapshots.first() {
            let _ = Command::new("sudo")
                .args([
                    "snapper",
                    "-c",
                    &snapper_cfg,
                    "delete",
                    &snapshot_num.to_string(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            dcli_snapshots.remove(0);
        }
    }

    Ok(())
}

/// Create a backup snapshot
pub fn create(paths: &ConfigPaths) -> Result<()> {
    let backup_tool = get_backup_tool(paths)?;

    let spinner = crate::progress::create_spinner(&format!("Creating {} snapshot...", backup_tool));

    match backup_tool.as_str() {
        "timeshift" => {
            let status = Command::new("sudo")
                .args([
                    "timeshift",
                    "--create",
                    "--comments",
                    "dcli backup (manual)",
                ])
                .status()?;

            if status.success() {
                spinner.finish_with_message("✓ Snapshot created successfully");
                Ok(())
            } else {
                spinner.finish_with_message("✗ Failed to create snapshot");
                anyhow::bail!("Failed to create timeshift snapshot");
            }
        }
        "snapper" => {
            let config = get_snapper_config(paths);
            let status = Command::new("sudo")
                .args([
                    "snapper",
                    "-c",
                    &config,
                    "create",
                    "-d",
                    "dcli backup (manual)",
                ])
                .status()?;

            if status.success() {
                spinner.finish_with_message("✓ Snapshot created successfully");
                Ok(())
            } else {
                spinner.finish_with_message("✗ Failed to create snapshot");
                anyhow::bail!("Failed to create snapper snapshot");
            }
        }
        _ => anyhow::bail!("Unknown backup tool: {}", backup_tool),
    }
}

/// List backup snapshots
pub fn list(paths: &ConfigPaths) -> Result<()> {
    let backup_tool = get_backup_tool(paths)?;

    match backup_tool.as_str() {
        "timeshift" => {
            let status = Command::new("sudo")
                .args(["timeshift", "--list"])
                .status()?;

            if !status.success() {
                anyhow::bail!("Failed to list timeshift snapshots");
            }
        }
        "snapper" => {
            let config = get_snapper_config(paths);
            let status = Command::new("sudo")
                .args(["snapper", "-c", &config, "list"])
                .status()?;

            if !status.success() {
                anyhow::bail!("Failed to list snapper snapshots");
            }
        }
        _ => anyhow::bail!("Unknown backup tool: {}", backup_tool),
    }

    Ok(())
}

/// Restore from a backup snapshot
pub fn restore(paths: &ConfigPaths, snapshot: Option<String>) -> Result<()> {
    let backup_tool = get_backup_tool(paths)?;

    // If no snapshot provided, show interactive selection with fzf
    let snapshot = if snapshot.is_none() {
        Some(select_snapshot_interactive(&backup_tool, paths)?)
    } else {
        snapshot
    };

    match backup_tool.as_str() {
        "timeshift" => {
            let mut args = vec!["timeshift", "--restore"];

            if let Some(snap) = snapshot.as_ref() {
                args.push("--snapshot");
                args.push(snap);
            }

            let status = Command::new("sudo").args(&args).status()?;

            if !status.success() {
                anyhow::bail!("Failed to restore timeshift snapshot");
            }
        }
        "snapper" => {
            let config = get_snapper_config(paths);

            if let Some(snap) = snapshot {
                let spinner = crate::progress::create_spinner(&format!(
                    "Restoring snapshot {} using snapper...",
                    snap
                ));

                let range = format!("{}..0", snap);
                let status = Command::new("sudo")
                    .args(["snapper", "-c", &config, "undochange", &range])
                    .status()?;

                if status.success() {
                    spinner.finish_with_message("✓ Snapshot restored successfully");
                } else {
                    spinner.finish_with_message("✗ Failed to restore snapshot");
                    anyhow::bail!("Failed to restore snapper snapshot");
                }
            } else {
                println!(
                    "{}",
                    "Snapper requires a snapshot number to restore.".yellow()
                );
                println!("Usage: dcli restore <snapshot-number>");
                println!();
                println!("Available snapshots:");

                Command::new("sudo")
                    .args(["snapper", "-c", &config, "list"])
                    .status()?;

                anyhow::bail!("No snapshot specified");
            }
        }
        _ => anyhow::bail!("Unknown backup tool: {}", backup_tool),
    }

    Ok(())
}

/// Interactive snapshot selection with fzf
fn select_snapshot_interactive(backup_tool: &str, paths: &ConfigPaths) -> Result<String> {
    use anyhow::Context;
    use std::process::Stdio;

    // Check if fzf is installed
    if which::which("fzf").is_err() {
        anyhow::bail!("fzf is not installed. Please install fzf or provide a snapshot ID.");
    }

    // Get snapshot list based on backup tool
    let list_output = match backup_tool {
        "timeshift" => {
            let output = Command::new("sudo")
                .args(["timeshift", "--list", "--scripted"])
                .output()?;
            String::from_utf8(output.stdout)?
        }
        "snapper" => {
            let config = get_snapper_config(paths);
            let output = Command::new("sudo")
                .args(["snapper", "-c", &config, "list"])
                .output()?;
            String::from_utf8(output.stdout)?
        }
        _ => anyhow::bail!("Unknown backup tool: {}", backup_tool),
    };

    if list_output.trim().is_empty() {
        anyhow::bail!("No snapshots found");
    }

    // Run fzf with snapshot list
    let mut fzf = Command::new("fzf")
        .args([
            "--header=→ Select snapshot to restore\nℹ Use arrow keys to select, ENTER to confirm",
            "--prompt=Select snapshot > ",
            "--height=100%",
            "--border=rounded",
            "--border-label= dcli restore ",
            "--border-label-pos=2",
            "--color=border:blue,label:cyan",
            "--no-multi",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    {
        let stdin = fzf.stdin.as_mut().context("Failed to open fzf stdin")?;
        use std::io::Write;
        write!(stdin, "{}", list_output)?;
    }

    let output = fzf.wait_with_output()?;

    if !output.status.success() {
        anyhow::bail!("Snapshot selection cancelled");
    }

    let selected = String::from_utf8(output.stdout)?.trim().to_string();

    if selected.is_empty() {
        anyhow::bail!("No snapshot selected");
    }

    // Extract snapshot ID from the selected line
    let snapshot_id = match backup_tool {
        "timeshift" => {
            // Timeshift format: snapshot name is the first field
            selected.split_whitespace().next().unwrap_or("").to_string()
        }
        "snapper" => {
            // Snapper format: ID is the first column (after skipping header)
            selected.split_whitespace().next().unwrap_or("").to_string()
        }
        _ => selected,
    };

    if snapshot_id.is_empty() {
        anyhow::bail!("Failed to parse snapshot ID");
    }

    Ok(snapshot_id)
}

/// Delete a backup snapshot
pub fn delete(paths: &ConfigPaths, snapshot: String) -> Result<()> {
    let backup_tool = get_backup_tool(paths)?;

    println!(
        "{} Deleting {} snapshot {}...",
        "→".blue(),
        backup_tool,
        snapshot.yellow()
    );

    match backup_tool.as_str() {
        "timeshift" => {
            let status = Command::new("sudo")
                .args(["timeshift", "--delete", "--snapshot", &snapshot])
                .status()?;

            if status.success() {
                println!("{} Snapshot deleted successfully", "✓".green());
                Ok(())
            } else {
                anyhow::bail!("Failed to delete timeshift snapshot");
            }
        }
        "snapper" => {
            let config = get_snapper_config(paths);
            let status = Command::new("sudo")
                .args(["snapper", "-c", &config, "delete", &snapshot])
                .status()?;

            if status.success() {
                println!("{} Snapshot deleted successfully", "✓".green());
                Ok(())
            } else {
                anyhow::bail!("Failed to delete snapper snapshot");
            }
        }
        _ => anyhow::bail!("Unknown backup tool: {}", backup_tool),
    }
}

/// Check backup configuration
pub fn check_config(paths: &ConfigPaths) -> Result<()> {
    println!("{}", "=== Backup Configuration ===".blue().bold());
    println!();

    // Load config to show settings
    if let Ok(config) = crate::config::load_config(paths) {
        println!("{}", "System Backup Settings:".bold());
        println!(
            "  Global enabled: {}",
            if config.system_backups.enabled {
                "true".green()
            } else {
                "false".red()
            }
        );
        println!(
            "  Backup on sync: {}",
            if config.system_backups.backup_on_sync {
                "true".green()
            } else {
                "false".yellow()
            }
        );
        println!(
            "  Backup on update: {}",
            if config.system_backups.backup_on_update {
                "true".green()
            } else {
                "false".yellow()
            }
        );

        if let Some(tool) = &config.system_backups.tool {
            println!("  Configured tool: {}", tool.cyan());
        }

        println!(
            "  Snapper config: {}",
            config.system_backups.snapper_config.cyan()
        );
        println!(
            "  Max backups: {}",
            if config.system_backups.max_backups == 0 {
                "unlimited".cyan()
            } else {
                config.system_backups.max_backups.to_string().cyan()
            }
        );
        println!();
    }

    match get_backup_tool(paths) {
        Ok(tool) => {
            println!("{} Detected backup tool: {}", "✓".green(), tool.green());

            match tool.as_str() {
                "timeshift" => {
                    println!();
                    println!("Timeshift status:");
                    let _ = Command::new("sudo").args(["timeshift", "--list"]).status();
                }
                "snapper" => {
                    let config = get_snapper_config(paths);
                    println!("{} Active Snapper config: {}", "→".blue(), config.cyan());
                    println!();
                    println!("Snapper configurations:");
                    let _ = Command::new("snapper").args(["list-configs"]).status();
                }
                _ => {}
            }
        }
        Err(e) => {
            println!("{} {}", "✗".red(), e);
            println!();
            println!("Please install either timeshift or snapper:");
            println!("  sudo pacman -S timeshift");
            println!("  sudo pacman -S snapper");
        }
    }

    Ok(())
}
