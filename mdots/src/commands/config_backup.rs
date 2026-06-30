use anyhow::{Context, Result};
use colored::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{load_config, ConfigPaths};

#[derive(Debug, Serialize, Deserialize)]
struct BackupMetadata {
    timestamp: String,
    timestamp_display: String,
    hostname: String,
    active_host_file: String,
    enabled_modules: Vec<String>,
    backup_type: String,
    dcli_version: String,
    package_count: usize,
    validation_passed: bool,
}

/// Create a config backup
pub fn save_config(paths: &ConfigPaths, backup_type: &str, json: bool) -> Result<()> {
    // Step 1: Run validation (quiet mode - no verbose output)
    // Run validation (use the validate command logic)
    // If validation fails, it will return an error
    if !json {
        print!("Validating configuration... ");
    }

    let validation_result = crate::commands::validate::run_quiet(paths, false, false);

    // Check if validation passed
    if let Err(e) = validation_result {
        if !json {
            println!();
            println!(
                "{}",
                "✗ Validation failed - refusing to create backup"
                    .red()
                    .bold()
            );
            println!();
            println!("Run 'dcli validate' to see detailed validation results.");
        }
        anyhow::bail!("Validation failed: {}", e);
    }

    if !json {
        println!();
    }

    // Step 2: Load config and metadata
    let config = load_config(paths)?;
    let hostname = config.host.clone();

    // Step 3: Create backup directory
    fs::create_dir_all(&paths.config_backups_dir)
        .context("Failed to create config-backups directory")?;

    // Step 4: Generate timestamp and filenames
    let timestamp = chrono::Utc::now();
    let timestamp_str = timestamp.format("%Y%m%d_%H%M%S").to_string();
    let timestamp_display = timestamp.format("%Y-%m-%d %H:%M").to_string();

    let backup_name = format!("{}-{}", hostname, timestamp_str);
    let backup_file = paths
        .config_backups_dir
        .join(format!("{}.tar.gz", backup_name));
    let metadata_file = paths
        .config_backups_dir
        .join(format!("{}.metadata.json", backup_name));

    if !json {
        println!("{} Creating backup: {}", "→".blue(), backup_name);
    }

    // Step 5: Create temporary directory for staging
    let temp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;
    let staging_dir = temp_dir.path().join("backup");
    fs::create_dir(&staging_dir)?;

    // Step 6: Copy files to staging (excluding non-active host files)
    if !json {
        println!("  {} Copying configuration files...", "→".blue());
    }

    // Copy config.yaml (pointer)
    if paths.config_file.exists() {
        fs::copy(&paths.config_file, staging_dir.join("config.yaml"))?;
    }

    // Copy active host file(s) — supports both flat files and directory structure
    let hosts_dir = paths.hosts_dir();
    let host_dir = hosts_dir.join(&hostname);
    if host_dir.is_dir() {
        let hosts_staging = staging_dir.join("hosts");
        copy_dir_recursive(&host_dir, &hosts_staging.join(&hostname))?;
    } else {
        let active_host_yaml = hosts_dir.join(format!("{}.yaml", hostname));
        let active_host_lua = hosts_dir.join(format!("{}.lua", hostname));
        let active_host_file = if active_host_lua.exists() {
            active_host_lua
        } else {
            active_host_yaml
        };
        if active_host_file.exists() {
            let hosts_staging = staging_dir.join("hosts");
            fs::create_dir_all(&hosts_staging)?;
            fs::copy(
                &active_host_file,
                hosts_staging.join(active_host_file.file_name().unwrap()),
            )?;
        }
    }

    // Copy entire modules directory
    let modules_dir = paths.modules_dir();
    if modules_dir.exists() {
        copy_dir_recursive(&modules_dir, &staging_dir.join("modules"))?;
    }

    // Copy entire scripts directory
    let scripts_dir = paths.config_dir.join("scripts");
    if scripts_dir.exists() {
        copy_dir_recursive(&scripts_dir, &staging_dir.join("scripts"))?;
    }

    // Copy entire dotfiles directory (if at root level)
    let dotfiles_dir = paths.config_dir.join("dotfiles");
    if dotfiles_dir.exists() {
        copy_dir_recursive(&dotfiles_dir, &staging_dir.join("dotfiles"))?;
    }

    // Copy state directory
    if paths.state_dir.exists() {
        copy_dir_recursive(&paths.state_dir, &staging_dir.join("state"))?;
    }

    // Determine the active host file path for metadata
    let active_host_file = paths.host_packages_file(&hostname);

    // Step 7: Create metadata
    let package_count = if let Ok(state_content) = fs::read_to_string(&paths.state_file) {
        // Count packages in state file
        state_content
            .lines()
            .filter(|l| l.trim().starts_with("- name:"))
            .count()
    } else {
        0
    };

    let metadata = BackupMetadata {
        timestamp: timestamp.to_rfc3339(),
        timestamp_display,
        hostname: hostname.clone(),
        active_host_file: active_host_file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        enabled_modules: config.enabled_modules.clone(),
        backup_type: backup_type.to_string(),
        dcli_version: env!("CARGO_PKG_VERSION").to_string(),
        package_count,
        validation_passed: true,
    };

    // Step 8: Create tar.gz archive
    if !json {
        println!("  {} Compressing archive...", "→".blue());
    }

    let tar_status = Command::new("tar")
        .args([
            "-czf",
            backup_file.to_str().unwrap(),
            "-C",
            staging_dir.to_str().unwrap(),
            ".",
        ])
        .status()
        .context("Failed to create tar archive")?;

    if !tar_status.success() {
        anyhow::bail!("tar command failed");
    }

    // Verify the new archive before trusting it: rotation below may delete older
    // backups, so a corrupt/truncated new backup must never push out good ones.
    if !verify_archive(&backup_file) {
        let _ = fs::remove_file(&backup_file);
        anyhow::bail!(
            "Backup verification failed (archive unreadable); existing backups were left intact"
        );
    }

    // Write metadata file only after successful, verified tar creation
    let metadata_json = serde_json::to_string_pretty(&metadata)?;
    fs::write(&metadata_file, metadata_json)?;

    // Step 9: Backup rotation
    rotate_backups(paths, &hostname, &config)?;

    if !json {
        println!();
        println!("{} Backup created successfully", "✓".green());
        println!("  Location: {}", backup_file.display());
        println!(
            "  Size: {}",
            format_file_size(fs::metadata(&backup_file)?.len())
        );
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "success": true,
                "backup_name": backup_name,
                "backup_file": backup_file,
                "metadata": metadata,
            }))?
        );
    }

    Ok(())
}

/// Rotate backups according to max_backups setting
fn rotate_backups(
    paths: &ConfigPaths,
    hostname: &str,
    config: &crate::config::Config,
) -> Result<()> {
    let max_backups = config.config_backups.max_backups;

    if max_backups == 0 {
        return Ok(()); // Unlimited backups
    }

    // Find all backups for this host
    let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();

    if let Ok(entries) = fs::read_dir(&paths.config_backups_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map(|e| e == "gz").unwrap_or(false) {
                let filename = path.file_stem().unwrap().to_string_lossy();
                if filename.starts_with(&format!("{}-", hostname)) {
                    let metadata_file = paths
                        .config_backups_dir
                        .join(format!("{}.metadata.json", filename));
                    backups.push((path, metadata_file));
                }
            }
        }
    }

    // Sort by filename (timestamp is in filename)
    backups.sort_by(|a, b| a.0.cmp(&b.0));

    // Delete oldest backups if we exceed max_backups
    while backups.len() > max_backups as usize {
        if let Some((backup_file, metadata_file)) = backups.first() {
            let _ = fs::remove_file(backup_file);
            let _ = fs::remove_file(metadata_file);
            backups.remove(0);
        }
    }

    Ok(())
}

/// Format file size for display
fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Recursively copy a directory
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            // Skip config-backups directory to avoid recursive backup
            if src_path.file_name() == Some(std::ffi::OsStr::new("config-backups")) {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if src_path.is_symlink() {
            // Copy symlink as-is
            if let Ok(link_target) = fs::read_link(&src_path) {
                let _ = std::os::unix::fs::symlink(&link_target, &dst_path);
            }
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// Restore from a config backup
pub fn restore_config(paths: &ConfigPaths, backup_name: Option<String>, json: bool) -> Result<()> {
    let config = load_config(paths)?;
    let hostname = config.host;

    // Step 1: Select backup (interactive if not specified)
    let backup_name = if let Some(name) = backup_name {
        name
    } else {
        select_backup_interactive(paths, &hostname)?
    };

    // Step 2: Load backup metadata
    let metadata_file = paths
        .config_backups_dir
        .join(format!("{}.metadata.json", backup_name));
    if !metadata_file.exists() {
        anyhow::bail!("Backup metadata not found: {}", backup_name);
    }

    let metadata_content = fs::read_to_string(&metadata_file)?;
    let metadata: BackupMetadata = serde_json::from_str(&metadata_content)?;

    if !json {
        println!("{}", "=== Restore Configuration ===".blue().bold());
        println!();
        println!("Backup: {}", backup_name.cyan());
        println!("Date: {}", metadata.timestamp_display);
        println!("Type: {}", metadata.backup_type);
        println!("Modules: {}", metadata.enabled_modules.join(", "));
        println!();
    }

    // Step 3: Create automatic backup of current state
    if !json {
        println!("{}", "Creating backup of current configuration...".blue());
    }

    save_config(paths, "pre-restore", json)?;

    if !json {
        println!("{}", "✓ Current configuration backed up".green());
        println!();
    }

    // Step 4: Extract backup to temporary directory
    if !json {
        println!("{}", "Extracting backup...".blue());
    }

    let backup_file = paths
        .config_backups_dir
        .join(format!("{}.tar.gz", backup_name));
    if !backup_file.exists() {
        anyhow::bail!("Backup file not found: {}", backup_name);
    }

    let temp_dir = tempfile::tempdir()?;
    let extract_dir = temp_dir.path();

    let tar_status = Command::new("tar")
        .args([
            "-xzf",
            backup_file.to_str().unwrap(),
            "-C",
            extract_dir.to_str().unwrap(),
        ])
        .status()
        .context("Failed to extract tar archive")?;

    if !tar_status.success() {
        anyhow::bail!("tar extraction failed");
    }

    // Step 5: Restore files
    if !json {
        println!("{}", "Restoring configuration files...".blue());
    }

    // Restore config.yaml
    let config_yaml_src = extract_dir.join("config.yaml");
    if config_yaml_src.exists() {
        fs::copy(&config_yaml_src, &paths.config_file)?;
        if !json {
            println!("  {} config.yaml", "✓".green());
        }
    }

    // Restore host file
    let hosts_src = extract_dir.join("hosts");
    if hosts_src.exists() {
        let hosts_dst = paths.hosts_dir();
        fs::create_dir_all(&hosts_dst)?;
        // Only restore the active host file (don't touch other hosts)
        if let Ok(entries) = fs::read_dir(&hosts_src) {
            for entry in entries.flatten() {
                let filename = entry.file_name();
                if filename.to_string_lossy().contains(&hostname) {
                    fs::copy(entry.path(), hosts_dst.join(&filename))?;
                    if !json {
                        println!("  {} hosts/{}", "✓".green(), filename.to_string_lossy());
                    }
                }
            }
        }
    }

    // Restore modules (replace entire directory)
    let modules_src = extract_dir.join("modules");
    if modules_src.exists() {
        let modules_dst = paths.modules_dir();
        if modules_dst.exists() {
            fs::remove_dir_all(&modules_dst)?;
        }
        copy_dir_recursive(&modules_src, &modules_dst)?;
        if !json {
            println!("  {} modules/", "✓".green());
        }
    }

    // Restore scripts (replace entire directory)
    let scripts_src = extract_dir.join("scripts");
    if scripts_src.exists() {
        let scripts_dst = paths.config_dir.join("scripts");
        if scripts_dst.exists() {
            fs::remove_dir_all(&scripts_dst)?;
        }
        copy_dir_recursive(&scripts_src, &scripts_dst)?;
        if !json {
            println!("  {} scripts/", "✓".green());
        }
    }

    // Restore dotfiles (replace entire directory)
    let dotfiles_src = extract_dir.join("dotfiles");
    if dotfiles_src.exists() {
        let dotfiles_dst = paths.config_dir.join("dotfiles");
        if dotfiles_dst.exists() {
            fs::remove_dir_all(&dotfiles_dst)?;
        }
        copy_dir_recursive(&dotfiles_src, &dotfiles_dst)?;
        if !json {
            println!("  {} dotfiles/", "✓".green());
        }
    }

    // Restore state (replace entire directory except config-backups)
    let state_src = extract_dir.join("state");
    if state_src.exists() {
        // Copy each state file individually to avoid overwriting config-backups
        for entry in fs::read_dir(&state_src)? {
            let entry = entry?;
            let filename = entry.file_name();

            // Skip config-backups directory
            if filename == "config-backups" {
                continue;
            }

            let dst_path = paths.state_dir.join(&filename);
            if entry.path().is_dir() {
                if dst_path.exists() {
                    fs::remove_dir_all(&dst_path)?;
                }
                copy_dir_recursive(&entry.path(), &dst_path)?;
            } else {
                fs::copy(entry.path(), &dst_path)?;
            }
        }
        if !json {
            println!("  {} state/", "✓".green());
        }
    }

    // Step 6: Re-symlink dotfiles
    if !json {
        println!();
        println!("{}", "Re-symlinking dotfiles...".blue());
    }

    let restored_config = load_config(paths)?;
    crate::dotfiles::sync_dotfiles(paths, &restored_config, true, json)?;

    if !json {
        println!("{}", "✓ Dotfiles re-symlinked".green());
    }

    // Step 7: Success message and next steps
    if !json {
        println!();
        println!("{}", "✓ Configuration restored successfully!".green());
        println!();
        println!("{}", "Next steps:".bold());
        println!("  1. Review restored configuration: dcli status");
        println!("  2. Run validation: dcli validate");
        println!("  3. Apply package changes: dcli sync");
        println!();
        println!(
            "{}",
            "Note: Packages are NOT automatically installed/removed.".yellow()
        );
        println!("Run 'dcli sync' to apply the restored package configuration.");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "success": true,
                "backup_name": backup_name,
                "metadata": metadata,
            }))?
        );
    }

    Ok(())
}

/// Interactive backup selection with fzf
fn select_backup_interactive(paths: &ConfigPaths, hostname: &str) -> Result<String> {
    use std::process::Stdio;

    // Check if fzf is installed
    if which::which("fzf").is_err() {
        anyhow::bail!("fzf is not installed. Please install fzf or provide a backup name.");
    }

    // Collect all backups for this host
    let mut backups: Vec<(String, BackupMetadata)> = Vec::new();

    if !paths.config_backups_dir.exists() {
        anyhow::bail!("No backups found");
    }

    for entry in fs::read_dir(&paths.config_backups_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().map(|e| e == "json").unwrap_or(false) {
            if let Ok(metadata_content) = fs::read_to_string(&path) {
                if let Ok(metadata) = serde_json::from_str::<BackupMetadata>(&metadata_content) {
                    if metadata.hostname == hostname {
                        let backup_name = path
                            .file_stem()
                            .unwrap()
                            .to_string_lossy()
                            .trim_end_matches(".metadata")
                            .to_string();
                        backups.push((backup_name, metadata));
                    }
                }
            }
        }
    }

    if backups.is_empty() {
        anyhow::bail!("No backups found for host: {}", hostname);
    }

    // Sort by timestamp (newest first)
    backups.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));

    // Build fzf input (format: "date | type | modules")
    let mut fzf_input = String::new();
    for (_backup_name, metadata) in &backups {
        fzf_input.push_str(&format!(
            "{} | {} | {} modules | {} packages\n",
            metadata.timestamp_display,
            metadata.backup_type,
            metadata.enabled_modules.len(),
            metadata.package_count
        ));
    }

    // Run fzf
    let mut fzf = Command::new("fzf")
        .args([
            "--header=→ Select backup to restore (ESC to cancel)\nℹ Use arrow keys to select, ENTER to confirm",
            "--prompt=Select backup > ",
            "--height=100%",
            "--border=rounded",
            "--border-label= dcli restore-config ",
            "--border-label-pos=2",
            "--color=border:blue,label:cyan",
            "--no-multi",
            "--reverse",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    {
        let stdin = fzf.stdin.as_mut().context("Failed to open fzf stdin")?;
        stdin.write_all(fzf_input.as_bytes())?;
    }

    let output = fzf.wait_with_output()?;

    if !output.status.success() {
        anyhow::bail!("Backup selection cancelled");
    }

    let selected = String::from_utf8(output.stdout)?.trim().to_string();

    if selected.is_empty() {
        anyhow::bail!("No backup selected");
    }

    // Extract timestamp from selected line (first field before |)
    let timestamp_display = selected.split('|').next().unwrap().trim();

    // Find matching backup by timestamp_display
    for (backup_name, metadata) in backups {
        if metadata.timestamp_display == timestamp_display {
            return Ok(backup_name);
        }
    }

    anyhow::bail!("Failed to find selected backup");
}

/// Verify that a `.tar.gz` backup archive is actually readable. A successful
/// `tar -czf` exit does not guarantee a non-truncated, valid stream, so this is
/// checked before rotation is allowed to delete older backups.
fn verify_archive(path: &Path) -> bool {
    Command::new("tar")
        .args(["-tzf", &path.to_string_lossy()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_archive_distinguishes_valid_and_corrupt() {
        let tmp = tempfile::tempdir().unwrap();

        // A real archive built with tar verifies OK.
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.txt"), "hello").unwrap();
        let good = tmp.path().join("good.tar.gz");
        let created = Command::new("tar")
            .args([
                "-czf",
                good.to_str().unwrap(),
                "-C",
                src.to_str().unwrap(),
                ".",
            ])
            .status()
            .unwrap()
            .success();
        assert!(created, "tar should create the archive");
        assert!(verify_archive(&good), "valid archive must verify");

        // Garbage that merely has the .tar.gz name must NOT verify.
        let bad = tmp.path().join("bad.tar.gz");
        std::fs::write(&bad, b"this is not a gzip archive").unwrap();
        assert!(
            !verify_archive(&bad),
            "corrupt archive must fail verification"
        );
    }
}
