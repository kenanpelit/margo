//! `mdots init --blackdon`: bootstrap from BlackDon's config repository.

use super::*;

/// Bootstrap from BlackDon's config repository
pub(super) fn bootstrap_blackdon_config(paths: &ConfigPaths) -> Result<()> {
    println!("{}", "=== Installing from BlackDon's Dotfiles ===".blue());
    println!();

    // Check if arch-config already exists
    if paths.config_dir.exists() {
        println!("{}", "arch-config directory already exists".yellow());
        print!("Do you want to backup and replace it? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }

        // Backup existing directory
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let backup_dir = format!("{}.backup.{}", paths.config_dir.display(), timestamp);
        println!(
            "{} Backing up existing directory to: {}",
            "→".blue(),
            backup_dir
        );
        fs::rename(&paths.config_dir, &backup_dir)
            .context("Failed to backup existing directory")?;
    }

    // Create temporary directory for cloning
    let temp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;
    let temp_path = temp_dir.path();

    println!(
        "{} Cloning BlackDon's arch-config from GitLab...",
        "→".blue()
    );

    // Clone the repository
    let clone_output = std::process::Command::new("git")
        .args(["clone", "https://gitlab.com/theblackdon/arch-config"])
        .arg(temp_path)
        .output()
        .context("Failed to execute git clone")?;

    if !clone_output.status.success() {
        anyhow::bail!(
            "Failed to clone repository: {}",
            String::from_utf8_lossy(&clone_output.stderr)
        );
    }

    println!("{} Repository cloned successfully", "✓".green());

    // Create arch-config directory
    fs::create_dir_all(&paths.config_dir).context("Failed to create arch-config directory")?;

    // Copy all contents except .git, using NEW directory structure
    println!("{} Copying configuration files...", "→".blue());

    // NEW STRUCTURE: Copy modules/ directly to root level
    let modules_src = temp_path.join("modules");
    let packages_modules_src = temp_path.join("packages").join("modules");

    // Try new location first, fall back to old location
    if modules_src.exists() {
        let dest_modules = paths.config_dir.join("modules");
        copy_dir_recursive(&modules_src, &dest_modules)?;
        println!("  {} modules/", "✓".green());
    } else if packages_modules_src.exists() {
        let dest_modules = paths.config_dir.join("modules");
        copy_dir_recursive(&packages_modules_src, &dest_modules)?;
        println!("  {} modules/", "✓".green());
    } else {
        // Create empty modules directory if neither exists
        fs::create_dir_all(paths.config_dir.join("modules"))?;
        println!("  {} modules/ (empty)", "✓".green());
    }

    // NEW STRUCTURE: Create empty hosts/ directory (don't copy from repo)
    let hosts_dir = paths.config_dir.join("hosts");
    fs::create_dir_all(&hosts_dir).context("Failed to create hosts directory")?;
    println!("  {} hosts/", "✓".green());

    // Copy other directories
    for dir_name in &[
        "state",
        "scripts",
        "udev-rules",
        "dotfiles",
        "logos",
        "wallpapers",
    ] {
        let src_dir = temp_path.join(dir_name);
        if src_dir.exists() {
            let dest_dir = paths.config_dir.join(dir_name);
            copy_dir_recursive(&src_dir, &dest_dir)?;
        }
    }

    // Copy user-facing documentation
    println!("{} Copying documentation...", "→".blue());
    copy_user_docs(&paths.config_dir)?;

    // Copy README if it exists
    let readme_src = temp_path.join("README.md");
    if readme_src.exists() {
        let readme_dest = paths.config_dir.join("README.md");
        fs::copy(&readme_src, &readme_dest).context("Failed to copy README.md")?;
    }

    println!("{} Configuration files copied", "✓".green());

    // Get current hostname
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());

    println!(
        "{} Creating config.yaml for host: {}",
        "→".blue(),
        hostname.green()
    );

    // Get machine description
    print!("Describe this machine (e.g., 'Gaming Desktop', 'Work Laptop'): ");
    io::stdout().flush()?;
    let mut machine_desc = String::new();
    io::stdin().read_line(&mut machine_desc)?;
    let machine_desc = machine_description(&machine_desc, &hostname);

    // Create config.yaml as POINTER file (NEW format)
    let config_content = format!(
        r#"# mdots configuration pointer
# This file points to the active host configuration
# The full configuration lives in hosts/{}.yaml
# Bootstrapped from BlackDon's configuration

# Active host
host: {}
"#,
        hostname, hostname
    );

    fs::write(&paths.config_file, config_content).context("Failed to create config.yaml")?;
    println!("{} Created config.yaml (pointer)", "✓".green());

    // Create host-specific FULL configuration file (NEW format)
    println!("{} Creating host configuration...", "→".blue());

    let host_content = format!(
        r#"# Host configuration for {}
# {}
# Bootstrapped from BlackDon's configuration

host: {}
description: {}

# Import shared configurations (optional)
# Example:
# import:
#   - hosts/shared/laptop-common.yaml

# Enabled modules
enabled_modules: []

# Module processing mode
# parallel: Collect and install all modules at once (faster, default)
# sequential: Process modules one-by-one in enabled_modules order (more control)
module_processing: parallel

# Host-specific packages
packages: []

# Exclude packages from base or modules
exclude: []

# Configuration backup settings
config_backups:
  enabled: true      # Auto-backup on sync
  max_backups: 5     # Keep last 5 backups (0 = unlimited)

# System backup settings
system_backups:
  enabled: true           # Global toggle for system backups
  backup_on_sync: true    # Create backup during mdots sync
  backup_on_update: true  # Create backup during mdots update
  tool: timeshift         # Backup tool: timeshift or snapper
  snapper_config: root    # Snapper config name (if using snapper)
  max_backups: 5          # Keep last N backups (0 = unlimited)

# Update hooks (optional - run scripts before/after system updates)
# update_hooks:
#   pre_update: "scripts/pre-update.sh"   # Run before yay -Syu
#   post_update: "scripts/post-update.sh" # Run after flatpak update
#   behavior: ask                          # Options: ask, always, once, skip
#   devel: false                           # Set to true to always use --devel flag (updates -git packages)

# Settings
flatpak_scope: user
auto_prune: false
"#,
        hostname, machine_desc, hostname, machine_desc
    );

    let host_file = paths.config_dir.join(format!("hosts/{}.yaml", hostname));
    fs::write(&host_file, host_content).context("Failed to create host file")?;
    println!("  {} hosts/{}.yaml", "✓".green(), hostname);

    println!();
    println!("{}", "=== Installation Complete! ===".green());
    println!();
    println!(
        "{}",
        "Successfully installed from BlackDon's dotfiles".green()
    );
    println!();
    println!("{}", "⚠️  IMPORTANT: Repository Ownership".yellow());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".yellow());
    println!("• This configuration is now DISCONNECTED from BlackDon's repository");
    println!("• You CANNOT push changes back to BlackDon's repo (main branch is protected)");
    println!("• This is YOUR configuration now - customize it freely!");
    println!();
    println!("{}", "Structure:".bold());
    println!("  config.yaml          → Points to hosts/{}.yaml", hostname);
    println!("  hosts/{}.yaml   → Your full configuration", hostname);
    println!("  modules/             → Package modules from BlackDon");
    println!("  scripts/             → Post-install hook scripts");
    println!("  docs/                → mdots documentation");
    println!();
    println!("Next steps:");
    println!("  1. Edit host config: hosts/{}.yaml", hostname);
    println!("  2. Review modules: mdots module list");
    println!("  3. Enable modules: mdots module enable <module-name>");
    println!("  4. Validate config: mdots validate");
    println!("  5. Preview sync: mdots sync --dry-run");
    println!();
    println!("Optional:");
    println!("  • Initialize your own git repo: mdots repo init");

    Ok(())
}
