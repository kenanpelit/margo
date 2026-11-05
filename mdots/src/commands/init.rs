use anyhow::{Context, Result};
use colored::*;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::config::{ConfigPaths, load_config};

// `mdots init`: the plain YAML init lives here; the --advanced (Lua),
// --blackdon and --nix variants, tool detection, docs and Lua type hints
// live in init/*.rs.
mod advanced_lua;
mod blackdon;
mod detect;
mod docs;
mod lua_types;
mod nix;
use advanced_lua::run_advanced_lua;
use blackdon::bootstrap_blackdon_config;
use detect::{detect_aur_helper, detect_backup_tool, detect_default_apps};
use docs::{copy_dir_recursive, copy_user_docs};
use lua_types::create_lua_type_definitions;
use nix::{run_nix_config_init, run_nix_init};

/// The host's machine description: the user's trimmed input, or a hostname-based
/// default when they leave it blank.
fn machine_description(input: &str, hostname: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        format!("Configuration for {}", hostname)
    } else {
        trimmed.to_string()
    }
}

/// Create the top-level mdots config directory tree (hosts/, modules/, scripts/,
/// state/) and report each created path.
fn create_mdots_dirs(paths: &ConfigPaths) -> Result<()> {
    println!("{} Creating directory structure...", "→".blue());

    fs::create_dir_all(&paths.config_dir).context("Failed to create mdots config directory")?;

    // New structure: hosts/, modules/, scripts/, state/ at top level
    fs::create_dir_all(paths.config_dir.join("hosts"))
        .context("Failed to create hosts directory")?;

    fs::create_dir_all(paths.config_dir.join("modules"))
        .context("Failed to create modules directory")?;

    fs::create_dir_all(paths.config_dir.join("scripts"))
        .context("Failed to create scripts directory")?;

    fs::create_dir_all(&paths.state_dir).context("Failed to create state directory")?;

    println!("  {} {}", "✓".green(), paths.config_dir.display());
    println!("  {} {}/hosts", "✓".green(), paths.config_dir.display());
    println!("  {} {}/modules", "✓".green(), paths.config_dir.display());
    println!("  {} {}/scripts", "✓".green(), paths.config_dir.display());
    println!("  {} {}/state", "✓".green(), paths.config_dir.display());

    Ok(())
}

/// Given the current root `.gitignore` contents (`None` if the file is absent),
/// return the contents to write so that `system-packages-*.yaml` is ignored, or
/// `None` when it is already handled and no write is needed.
fn gitignore_with_system_packages(existing: Option<&str>) -> Option<String> {
    const ENTRY: &str = "# System packages merged from host (auto-generated, host-specific)\nsystem-packages-*.yaml\n";
    match existing {
        Some(content) if content.contains("system-packages") => None,
        Some(content) => Some(format!("{}\n{}", content, ENTRY)),
        None => Some(ENTRY.to_string()),
    }
}

/// Initialize mdots configuration directory structure
pub fn run(
    paths: &ConfigPaths,
    bootstrap_blackdon: bool,
    lua: bool,
    nix: bool,
    nix_init: bool,
) -> Result<()> {
    if bootstrap_blackdon {
        return bootstrap_blackdon_config(paths);
    }

    if lua {
        return run_advanced_lua(paths);
    }

    if nix {
        return run_nix_config_init(paths);
    }

    if nix_init {
        return run_nix_init(paths);
    }

    println!("{}", "=== Initializing mdots configuration ===".blue());
    println!();

    // Check if already exists
    if paths.config_dir.exists() {
        println!("{}", "mdots config directory already exists".yellow());
        println!("Location: {}", paths.config_dir.display());
        print!("Reinitialize? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }
    }

    // Detect package manager type
    let pkg_manager_type = crate::config::detect_package_manager_type().unwrap_or_else(|_| {
        println!(
            "{} Could not auto-detect package manager, defaulting to pacman",
            "⚠".yellow()
        );
        crate::config::PackageManagerType::Pacman
    });

    let pkg_manager_str = match pkg_manager_type {
        crate::config::PackageManagerType::Pacman => "pacman",
    };

    println!(
        "{} Detected package manager: {}",
        "→".blue(),
        pkg_manager_str.green()
    );
    println!();

    // Create NEW directory structure (no packages/ parent directory)
    create_mdots_dirs(paths)?;

    // Copy user-facing documentation
    println!("{} Copying documentation...", "→".blue());
    copy_user_docs(&paths.config_dir)?;

    // Create .gitignore for state directory
    let state_gitignore = paths.state_dir.join(".gitignore");
    if !state_gitignore.exists() {
        println!("{} Creating state/.gitignore...", "→".blue());
        fs::write(&state_gitignore, "# Ignore all state files\n*\n")
            .context("Failed to create state/.gitignore")?;
        println!("  {} state/.gitignore", "✓".green());
    }

    // Create/update .gitignore for system-packages-*.yaml
    let root_gitignore = paths.config_dir.join(".gitignore");
    let existing_gitignore = if root_gitignore.exists() {
        Some(fs::read_to_string(&root_gitignore)?)
    } else {
        None
    };
    if let Some(content) = gitignore_with_system_packages(existing_gitignore.as_deref()) {
        println!("{} Updating .gitignore...", "→".blue());
        fs::write(&root_gitignore, content).context("Failed to create/update .gitignore")?;
        println!(
            "  {} .gitignore (added system-packages-*.yaml)",
            "✓".green()
        );
    }

    // Get hostname
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());

    // Get machine description
    print!("Describe this machine (e.g., 'Gaming Desktop', 'Work Laptop'): ");
    io::stdout().flush()?;
    let mut machine_desc = String::new();
    io::stdin().read_line(&mut machine_desc)?;
    let machine_desc = machine_description(&machine_desc, &hostname);

    // Create config.yaml pointer, base module, host config and example module.
    write_config_pointer(paths, &hostname, pkg_manager_str)?;
    write_base_module(paths, &pkg_manager_type)?;
    write_host_config(paths, &hostname, &machine_desc, &pkg_manager_type)?;
    write_example_module(paths)?;

    print_init_summary(&hostname, pkg_manager_str);

    Ok(())
}

/// Write `config.yaml` — the pointer file naming the active host. No-op (with a
/// note) if it already exists.
fn write_config_pointer(paths: &ConfigPaths, hostname: &str, pkg_manager_str: &str) -> Result<()> {
    let config_file = paths.config_file.clone();
    if config_file.exists() {
        println!("  {} config.yaml already exists", "→".yellow());
        return Ok(());
    }
    println!("{} Creating config.yaml (pointer)...", "→".blue());

    let config_content = format!(
        r#"# mdots configuration pointer
# This file points to the active host configuration
# The full configuration lives in hosts/{hostname}.yaml

# Active host
host: {hostname}

# Package manager: pacman (Arch and Arch-based distros)
package_manager: {pkg_manager}
"#,
        hostname = hostname,
        pkg_manager = pkg_manager_str
    );

    fs::write(&config_file, config_content).context("Failed to create config.yaml")?;
    println!("  {} config.yaml", "✓".green());
    Ok(())
}

/// Write `modules/base.yaml` — the distro-aware base package set. No-op (with a
/// note) if it already exists.
fn write_base_module(
    paths: &ConfigPaths,
    pkg_manager_type: &crate::config::PackageManagerType,
) -> Result<()> {
    let base_file = paths.config_dir.join("modules/base.yaml");
    if base_file.exists() {
        println!("  {} modules/base.yaml already exists", "→".yellow());
        return Ok(());
    }
    println!("{} Creating modules/base.yaml...", "→".blue());

    let base_content = match pkg_manager_type {
        crate::config::PackageManagerType::Pacman => r#"# Base packages installed on all systems
# These packages are included regardless of host or modules

description: Base system packages

packages:
  # Essential base system
  - base
  - base-devel

  # Kernel (uncomment the one you use)
  # - linux              # Standard kernel
  # - linux-zen          # Zen kernel (optimized for desktop)
  # - linux-lts          # Long-term support kernel
  # - linux-hardened     # Security-focused kernel

  # Firmware (usually needed)
  # - linux-firmware

  # Basic tools (uncomment as needed)
  # - git
  # - vim
  # - neovim
  # - htop

  # mdots dependencies
  - paru         # AUR helper (required for AUR package management)
  - fzf          # Fuzzy finder (required for mdots search/module/backup TUI)
  - timeshift    # System backup tool (required for mdots backup commands)
"#
        .to_string(),
    };

    fs::write(&base_file, base_content).context("Failed to create base.yaml")?;
    println!("  {} modules/base.yaml", "✓".green());
    Ok(())
}

/// Write the host-specific full configuration file `hosts/<hostname>.yaml`.
/// No-op (with a note) if it already exists.
fn write_host_config(
    paths: &ConfigPaths,
    hostname: &str,
    machine_desc: &str,
    pkg_manager_type: &crate::config::PackageManagerType,
) -> Result<()> {
    let host_file = paths.config_dir.join(format!("hosts/{}.yaml", hostname));
    if host_file.exists() {
        println!("  {} hosts/{}.yaml already exists", "→".yellow(), hostname);
        return Ok(());
    }
    println!("{} Creating host configuration...", "→".blue());

    let backup_tool = detect_backup_tool();

    let pkg_manager_section = match pkg_manager_type {
        crate::config::PackageManagerType::Pacman => r#"# AUR helper to use (auto-detects paru or yay if not specified)
# aur_helper: paru   # Options: paru, yay, or any AUR helper

# Update hooks (optional - run scripts before/after system updates)
# update_hooks:
#   pre_update: "scripts/pre-update.sh"   # Run before system update
#   post_update: "scripts/post-update.sh" # Run after flatpak update
#   behavior: ask                          # Options: ask, always, once, skip
#   devel: false                           # Set to true to always use --devel flag (updates -git packages)"#.to_string(),
    };

    let host_content = format!(
        r#"# Host configuration for {hostname}
# {description}

host: {hostname}
description: {description}

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
  tool: {backup_tool}     # Backup tool: timeshift or snapper
  snapper_config: root    # Snapper config name (if using snapper)
  max_backups: 5          # Keep last N backups (0 = unlimited)

# Settings
flatpak_scope: user
auto_prune: false

{pkg_manager_section}
"#,
        hostname = hostname,
        description = machine_desc,
        backup_tool = backup_tool,
        pkg_manager_section = pkg_manager_section
    );

    fs::write(&host_file, host_content).context("Failed to create host file")?;
    println!("  {} hosts/{}.yaml", "✓".green(), hostname);
    Ok(())
}

/// Write `modules/example.yaml` — a template module the user can copy or delete.
/// No-op if it already exists.
fn write_example_module(paths: &ConfigPaths) -> Result<()> {
    let example_module = paths.config_dir.join("modules/example.yaml");
    if example_module.exists() {
        return Ok(());
    }
    println!("{} Creating example module...", "→".blue());

    let example_content = r#"# Example module template
# Copy this to create new modules, or delete it

description: Example module - customize or delete this

# List of packages in this module
packages: []

# Modules that conflict with this one
conflicts: []

# Script to run before installing packages (optional)
# Useful for system preparation like enabling multilib
pre_install_hook: ""

# Script to run after installing packages (optional)
post_install_hook: ""
"#;

    fs::write(&example_module, example_content).context("Failed to create example module")?;
    println!("  {} modules/example.yaml", "✓".green());
    Ok(())
}

/// Print the post-init summary: the directory layout and suggested next steps.
fn print_init_summary(hostname: &str, pkg_manager_str: &str) {
    println!();
    println!("{}", "✓ mdots initialized successfully!".green());
    println!();
    println!("{}", "Structure:".bold());
    println!("  config.yaml          → Points to hosts/{}.yaml", hostname);
    println!(
        "                         (package_manager: {})",
        pkg_manager_str
    );
    println!("  hosts/{}.yaml   → Your full configuration", hostname);
    println!("  modules/base.yaml    → Base packages for all hosts");
    println!("  modules/             → Optional package modules");
    println!("  scripts/             → Post-install hook scripts");
    println!("  docs/                → mdots documentation");
    println!();
    println!("Next steps:");
    println!("  1. Edit host config: hosts/{}.yaml", hostname);
    println!("  2. Edit base packages: modules/base.yaml");
    println!("  3. Run: mdots validate");
    println!("  4. Run: mdots module list");
    println!("  5. Run: mdots sync --dry-run");
    println!();
    println!("{}", "Advanced features:".bold());
    println!("  • Create shared configs in hosts/shared/");
    println!("  • Use 'import:' in host files to include shared configs");
    println!("  • Version control: mdots repo init");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_machine_description_defaults_when_blank() {
        // Empty/whitespace input falls back to a hostname-derived description.
        assert_eq!(
            machine_description("   ", "thinkpad"),
            "Configuration for thinkpad"
        );
        assert_eq!(
            machine_description("", "thinkpad"),
            "Configuration for thinkpad"
        );
    }

    #[test]
    fn test_machine_description_trims_user_input() {
        assert_eq!(
            machine_description("  Work Laptop  ", "thinkpad"),
            "Work Laptop"
        );
    }

    #[test]
    fn test_gitignore_created_when_absent() {
        let out = gitignore_with_system_packages(None).expect("absent → write");
        assert!(out.contains("system-packages-*.yaml"));
    }

    #[test]
    fn test_gitignore_appends_when_entry_missing() {
        let out =
            gitignore_with_system_packages(Some("target/\n")).expect("missing entry → append");
        assert!(out.starts_with("target/\n"), "existing content preserved");
        assert!(out.contains("system-packages-*.yaml"));
    }

    #[test]
    fn test_gitignore_unchanged_when_entry_present() {
        // Already handled → no rewrite.
        assert!(gitignore_with_system_packages(Some("system-packages-*.yaml\n")).is_none());
    }
}
