use anyhow::{Context, Result};
use colored::*;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config::ConfigPaths;

/// Migrate from old structure (packages/) to new structure (hosts/, modules/)
pub fn run(paths: &ConfigPaths, dry_run: bool) -> Result<()> {
    println!("{}", "=== mdots Configuration Migration ===".blue());
    println!();

    // Check if already using new structure
    let new_modules_dir = paths.config_dir.join("modules");
    let new_hosts_dir = paths.config_dir.join("hosts");
    let old_packages_dir = paths.config_dir.join("packages");

    if new_modules_dir.exists() && new_hosts_dir.exists() {
        println!("{}", "✓ Already using new structure!".green());
        println!();
        println!("Your configuration is already in the new format:");
        println!("  • hosts/        - Host configurations");
        println!("  • modules/      - Package modules");
        println!("  • config.yaml   - Configuration pointer");
        return Ok(());
    }

    if !old_packages_dir.exists() {
        println!("{}", "✗ No packages/ directory found".red());
        println!();
        println!("Nothing to migrate. Run 'mdots init' to create a new configuration.");
        return Ok(());
    }

    println!(
        "{}",
        "This will migrate your configuration to the new structure:".bold()
    );
    println!();
    println!("{}", "Changes:".bold());
    println!("  {} packages/modules/  →  modules/", "•".blue());
    println!("  {} packages/hosts/    →  hosts/", "•".blue());
    println!("  {} packages/base.yaml →  modules/base.yaml", "•".blue());
    println!(
        "  {} config.yaml        →  pointer to host file",
        "•".blue()
    );
    println!();

    // Detect what needs to be migrated
    let old_modules = old_packages_dir.join("modules");
    let old_hosts = old_packages_dir.join("hosts");
    let old_base = old_packages_dir.join("base.yaml");

    let mut actions = Vec::new();

    if old_modules.exists() {
        actions.push("Move packages/modules/ → modules/".to_string());
    }
    if old_hosts.exists() {
        actions.push("Move packages/hosts/ → hosts/".to_string());
    }
    if old_base.exists() {
        actions.push("Move packages/base.yaml → modules/base.yaml".to_string());
    }

    // Load current config to extract hostname
    let config_content =
        fs::read_to_string(&paths.config_file).context("Failed to read config.yaml")?;
    let config: serde_yaml::Value =
        serde_yaml::from_str(&config_content).context("Failed to parse config.yaml")?;

    let hostname = config
        .get("host")
        .and_then(|h| h.as_str())
        .unwrap_or("localhost");

    actions.push(format!(
        "Convert config.yaml → pointer to hosts/{}.yaml",
        hostname
    ));
    actions.push(format!("Create full config in hosts/{}.yaml", hostname));

    if actions.is_empty() {
        println!("{}", "Nothing to migrate!".yellow());
        return Ok(());
    }

    println!("{}", "Migration plan:".bold());
    for action in &actions {
        println!("  {} {}", "→".blue(), action);
    }
    println!();

    if dry_run {
        println!("{}", "[DRY RUN - No changes will be made]".yellow());
        println!();
        println!("Run without --dry-run to perform migration:");
        println!("  mdots migrate");
        return Ok(());
    }

    // Confirm migration
    print!("{} ", "Proceed with migration? [y/N]".yellow());
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if !input.trim().eq_ignore_ascii_case("y") {
        println!("{}", "Migration cancelled".yellow());
        return Ok(());
    }

    println!();
    println!("{} Starting migration...", "→".blue());
    println!();

    // Create backup
    let backup_dir = create_backup(&paths.config_dir)?;
    println!("  {} Created backup: {}", "✓".green(), backup_dir.display());

    // Perform migration
    migrate_structure(paths, &old_packages_dir, hostname)?;

    println!();
    println!("{}", "✓ Migration complete!".green());
    println!();
    println!("New structure:");
    println!("  config.yaml          → Points to hosts/{}.yaml", hostname);
    println!("  hosts/{}.yaml   → Your full configuration", hostname);
    println!("  modules/base.yaml    → Base packages");
    println!("  modules/             → Package modules");
    println!();
    println!("Backup saved to: {}", backup_dir.display());
    println!();
    println!("Next steps:");
    println!("  1. Run: mdots validate");
    println!("  2. Run: mdots status");
    println!("  3. Run: mdots sync --dry-run");

    Ok(())
}

fn create_backup(config_dir: &Path) -> Result<PathBuf> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let backup_name = format!("arch-config.backup.{}", timestamp);
    let backup_dir = config_dir
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&backup_name);

    // Copy entire directory
    copy_dir_recursive(config_dir, &backup_dir)?;

    Ok(backup_dir)
}

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).context(format!("Failed to create directory: {}", dst.display()))?;

    for entry in
        fs::read_dir(src).context(format!("Failed to read directory: {}", src.display()))?
    {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        // Skip backup directories
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("arch-config.backup")
        {
            continue;
        }

        // Handle symlinks, directories, and files
        let metadata = match src_path.symlink_metadata() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Warning: Skipping {} - {}", src_path.display(), e);
                continue;
            }
        };

        if metadata.is_symlink() {
            // Copy symlink as-is
            #[cfg(unix)]
            {
                use std::os::unix::fs as unix_fs;
                let link_target = match fs::read_link(&src_path) {
                    Ok(target) => target,
                    Err(e) => {
                        eprintln!("Warning: Skipping symlink {} - {}", src_path.display(), e);
                        continue;
                    }
                };
                if let Err(e) = unix_fs::symlink(&link_target, &dst_path) {
                    eprintln!(
                        "Warning: Failed to copy symlink {} - {}",
                        src_path.display(),
                        e
                    );
                }
            }
            #[cfg(not(unix))]
            {
                eprintln!(
                    "Warning: Skipping symlink {} (not supported on this platform)",
                    src_path.display()
                );
            }
        } else if metadata.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            if let Err(e) = fs::copy(&src_path, &dst_path) {
                eprintln!("Warning: Failed to copy {} - {}", src_path.display(), e);
            }
        }
    }

    Ok(())
}

fn migrate_structure(paths: &ConfigPaths, old_packages_dir: &Path, hostname: &str) -> Result<()> {
    // 1. Move modules/ directory
    let old_modules = old_packages_dir.join("modules");
    let new_modules = paths.config_dir.join("modules");

    if old_modules.exists() {
        if new_modules.exists() {
            fs::remove_dir_all(&new_modules)?;
        }
        fs::rename(&old_modules, &new_modules).context("Failed to move modules directory")?;
        println!("  {} Moved modules/", "✓".green());
    }

    // 2. Move hosts/ directory
    let old_hosts = old_packages_dir.join("hosts");
    let new_hosts = paths.config_dir.join("hosts");

    if old_hosts.exists() {
        if new_hosts.exists() {
            fs::remove_dir_all(&new_hosts)?;
        }
        fs::rename(&old_hosts, &new_hosts).context("Failed to move hosts directory")?;
        println!("  {} Moved hosts/", "✓".green());
    } else {
        // Create hosts directory if it doesn't exist
        fs::create_dir_all(&new_hosts)?;
        println!("  {} Created hosts/", "✓".green());
    }

    // 3. Move base.yaml
    let old_base = old_packages_dir.join("base.yaml");
    let new_base = new_modules.join("base.yaml");

    if old_base.exists() {
        fs::rename(&old_base, &new_base).context("Failed to move base.yaml")?;
        println!("  {} Moved base.yaml → modules/", "✓".green());
    }

    // 4. Read current config.yaml
    let config_content = fs::read_to_string(&paths.config_file)?;
    let config: serde_yaml::Value = serde_yaml::from_str(&config_content)?;

    // 5. Create or update host config file
    let host_file = new_hosts.join(format!("{}.yaml", hostname));

    // Check if host file needs to be updated (old format or doesn't exist)
    let needs_update = if host_file.exists() {
        // Check if it's in old format (missing 'host' field)
        match fs::read_to_string(&host_file) {
            Ok(content) => {
                match serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    Ok(yaml) => yaml.get("host").is_none(), // Old format if missing 'host' field
                    Err(_) => true,                         // Invalid YAML, needs update
                }
            }
            Err(_) => true, // Can't read, needs update
        }
    } else {
        true // Doesn't exist, needs creation
    };

    if needs_update {
        // Read existing host file if it exists (to preserve packages)
        let existing_host_yaml = if host_file.exists() {
            fs::read_to_string(&host_file)
                .ok()
                .and_then(|content| serde_yaml::from_str::<serde_yaml::Value>(&content).ok())
        } else {
            None
        };

        // Convert old config.yaml to host config, merging with existing host file
        let default_description = format!("{} configuration", hostname);
        let description = existing_host_yaml
            .as_ref()
            .and_then(|y| y.get("description"))
            .and_then(|d| d.as_str())
            .or_else(|| config.get("description").and_then(|d| d.as_str()))
            .unwrap_or(&default_description);

        // Extract enabled_modules (prefer config.yaml, fallback to existing host file)
        let enabled_modules_raw = config
            .get("enabled_modules")
            .or_else(|| {
                existing_host_yaml
                    .as_ref()
                    .and_then(|y| y.get("enabled_modules"))
            })
            .and_then(|m| m.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| format!("  - {}", s))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        let enabled_modules = if enabled_modules_raw.is_empty() {
            "[]".to_string()
        } else {
            format!("\n{}", enabled_modules_raw)
        };

        // Extract packages (prefer existing host file, fallback to config.yaml)
        let packages_raw = existing_host_yaml
            .as_ref()
            .and_then(|y| y.get("packages"))
            .or_else(|| config.get("additional_packages"))
            .or_else(|| config.get("packages"))
            .and_then(|p| p.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| format!("  - {}", s))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        let packages = if packages_raw.is_empty() {
            "[]".to_string()
        } else {
            format!("\n{}", packages_raw)
        };

        let flatpak_scope = config
            .get("flatpak_scope")
            .and_then(|f| f.as_str())
            .unwrap_or("user");

        let auto_prune = config
            .get("auto_prune")
            .and_then(|a| a.as_bool())
            .unwrap_or(false);

        let backup_tool = config
            .get("backup_tool")
            .and_then(|b| b.as_str())
            .unwrap_or("timeshift");

        let snapper_config = config
            .get("snapper_config")
            .and_then(|s| s.as_str())
            .unwrap_or("root");

        // Build system_backups section
        let system_backups_section = format!(
            r#"
# System backup settings
system_backups:
  enabled: true           # Global toggle for system backups
  backup_on_sync: true    # Create backup during mdots sync
  backup_on_update: true  # Create backup during mdots update
  tool: {}                # Backup tool: timeshift or snapper
  snapper_config: {}      # Snapper config name (if using snapper)
  max_backups: 5          # Keep last N backups (0 = unlimited)"#,
            backup_tool, snapper_config
        );

        let host_content = format!(
            r#"# Host configuration for {}
# Migrated from old config.yaml

host: {}
description: {}

# Import shared configurations (optional)
# Example:
# import:
#   - hosts/shared/laptop-common.yaml

# Enabled modules
enabled_modules:{}

# Host-specific packages
packages:{}

# Exclude packages from base or modules
exclude: []

# Configuration backup settings
config_backups:
  enabled: true      # Auto-backup on sync
  max_backups: 5     # Keep last 5 backups (0 = unlimited)
{}

# Settings
flatpak_scope: {}
auto_prune: {}
"#,
            hostname,
            hostname,
            description,
            enabled_modules,
            packages,
            system_backups_section,
            flatpak_scope,
            auto_prune
        );

        fs::write(&host_file, host_content)?;
        if existing_host_yaml.is_some() {
            println!(
                "  {} Updated hosts/{}.yaml to new format",
                "✓".green(),
                hostname
            );
        } else {
            println!("  {} Created hosts/{}.yaml", "✓".green(), hostname);
        }
    } else {
        println!(
            "  {} hosts/{}.yaml already in new format",
            "✓".green(),
            hostname
        );
    }

    // 6. Convert config.yaml to pointer
    let pointer_content = format!(
        r#"# mdots configuration pointer
# This file points to the active host configuration
# Migrated from old structure

# Active host
host: {}
"#,
        hostname
    );

    fs::write(&paths.config_file, pointer_content)?;
    println!("  {} Updated config.yaml (now a pointer)", "✓".green());

    // 7. Remove empty packages/ directory
    if old_packages_dir.exists() {
        // Check if it's empty
        let is_empty = fs::read_dir(old_packages_dir)?.next().is_none();

        if is_empty {
            fs::remove_dir(old_packages_dir)?;
            println!("  {} Removed empty packages/", "✓".green());
        } else {
            println!("  {} Kept packages/ (contains other files)", "→".yellow());
        }
    }

    Ok(())
}
