use anyhow::{Context, Result};
use colored::*;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::config::{load_config, ConfigPaths};

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

/// User-facing documentation files to include in arch-config
const USER_DOCS: &[&str] = &[
    "CHEAT-SHEET.md",
    "MDOTS-LUA-API.md",
    "DIRECTORY-MODULES.md",
    "LUA-HOSTS.md",
    "LUA-MODULES.md",
    "SERVICES.md",
];

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

/// Initialize arch-config with advanced Lua configuration
fn run_advanced_lua(paths: &ConfigPaths) -> Result<()> {
    println!(
        "{}",
        "=== Initializing arch-config (Advanced Lua Mode) ===".blue()
    );
    println!();
    println!(
        "{}",
        "This will set up your configuration using Lua files instead of YAML.".cyan()
    );
    println!(
        "{}",
        "Lua files allow dynamic, conditional configuration based on hardware,".cyan()
    );
    println!("{}", "hostname, and system state.".cyan());
    println!();

    // Check if already exists
    if paths.config_dir.exists() {
        println!("{}", "arch-config directory already exists".yellow());
        println!("Location: {}", paths.config_dir.display());
        print!("Reinitialize with Lua config? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }
    }

    // Create directory structure
    println!("{} Creating directory structure...", "→".blue());

    fs::create_dir_all(&paths.config_dir).context("Failed to create arch-config directory")?;

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

    // Create/update .gitignore
    let root_gitignore = paths.config_dir.join(".gitignore");
    let gitignore_content = "# System packages merged from host (auto-generated, host-specific)\nsystem-packages-*.yaml\n";
    if !root_gitignore.exists() {
        println!("{} Creating .gitignore...", "→".blue());
        fs::write(&root_gitignore, gitignore_content).context("Failed to create .gitignore")?;
        println!("  {} .gitignore", "✓".green());
    }

    // Create Lua type definitions for editor support
    create_lua_type_definitions(&paths.config_dir)?;

    // Get hostname
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());

    // Ask about single vs multi-host configuration
    println!();
    println!("{}", "Configuration mode:".bold());
    println!("  1. Single host  - All configuration in config.lua (simpler)");
    println!(
        "  2. Multi host   - Pointer config.lua + hosts/{}.lua (share configs across machines)",
        hostname
    );
    println!();
    print!("Choose mode [1/2] (default: 1): ");
    io::stdout().flush()?;
    let mut mode_input = String::new();
    io::stdin().read_line(&mut mode_input)?;
    let multi_host = mode_input.trim() == "2";

    // Get machine description
    print!("Describe this machine (e.g., 'Gaming Desktop', 'Work Laptop'): ");
    io::stdout().flush()?;
    let mut machine_desc = String::new();
    io::stdin().read_line(&mut machine_desc)?;
    let machine_desc = machine_description(&machine_desc, &hostname);

    // Detect system settings
    let backup_tool = detect_backup_tool();
    let aur_helper = detect_aur_helper();
    let default_apps = detect_default_apps();

    println!("  {} Detected hostname: {}", "→".blue(), hostname.green());
    println!(
        "  {} Detected AUR helper: {}",
        "→".blue(),
        aur_helper.green()
    );
    println!(
        "  {} Detected default apps: browser={}, terminal={}, editor={}, file_manager={}",
        "→".blue(),
        default_apps.browser.green(),
        default_apps.terminal.green(),
        default_apps.text_editor.green(),
        default_apps.file_manager.green()
    );

    // Create config.lua (and optionally host file for multi-host mode)
    let config_lua = paths.config_dir.join("config.lua");
    if !config_lua.exists() {
        if multi_host {
            // Multi-host mode: create pointer config.lua + hosts/{hostname}.lua
            println!("{} Creating config.lua (pointer)...", "→".blue());

            let pointer_content = format!(
                r#"-- mdots configuration pointer
-- This file points to the active host configuration
-- The full configuration lives in hosts/{hostname}.lua

-- Active host
return {{
    host = "{hostname}",
}}
"#,
                hostname = hostname,
            );

            fs::write(&config_lua, pointer_content).context("Failed to create config.lua")?;
            println!("  {} config.lua (pointer)", "✓".green());

            // Create host-specific file
            let host_lua = paths.config_dir.join(format!("hosts/{}.lua", hostname));
            println!("{} Creating hosts/{}.lua...", "→".blue(), hostname);

            let host_content = format!(
                r#"-- Host configuration for {hostname}
-- {desc}
-- See LUA-HOSTS.md for full documentation

local is_laptop = mdots.hardware.is_laptop()
local memory_mb = mdots.system.memory_total_mb()

mdots.log.info(string.format("Loading config for {hostname} (%d MB RAM)", memory_mb))

-- ═══════════════════════════════════════════════════════════════════
-- MODULE SELECTION
-- ═══════════════════════════════════════════════════════════════════

local enabled_modules = {{
    "base",
    -- Add your modules here
}}

-- Example: Add GPU drivers based on hardware detection
-- if mdots.hardware.has_nvidia() then
--     table.insert(enabled_modules, "nvidia-drivers")
-- elseif mdots.hardware.has_amd_gpu() then
--     table.insert(enabled_modules, "amd-drivers")
-- end

-- Example: Add laptop-specific modules
-- if is_laptop then
--     table.insert(enabled_modules, "laptop-power")
-- end

-- ═══════════════════════════════════════════════════════════════════
-- SERVICES
-- ═══════════════════════════════════════════════════════════════════

local services = {{
    enabled = {{}},
    disabled = {{}},
}}

-- Example: Enable docker if module is enabled
-- if mdots.util.contains(enabled_modules, "docker") then
--     table.insert(services.enabled, "docker.service")
-- end

-- ═══════════════════════════════════════════════════════════════════
-- RETURN CONFIGURATION
-- ═══════════════════════════════════════════════════════════════════

return {{
    host = "{hostname}",
    description = "{desc}",

    enabled_modules = enabled_modules,

    -- Host-specific packages (in addition to modules)
    packages = {{}},

    -- Packages to exclude from modules
    exclude = {{}},

    -- Services configuration
    services = services,

    -- Default applications
    default_apps = {{
        browser = "{browser}",
        terminal = "{terminal}",
        text_editor = "{text_editor}",
        file_manager = "{file_manager}",
    }},

    -- Settings
    flatpak_scope = "user",
    auto_prune = false,
    module_processing = "parallel",
    aur_helper = "{aur_helper}",

    -- Backup settings
    config_backups = {{
        enabled = true,
        max_backups = 5,
    }},

    system_backups = {{
        enabled = true,
        backup_on_sync = true,
        backup_on_update = true,
        tool = "{backup_tool}",
        snapper_config = "root",
    }},
}}
"#,
                hostname = hostname,
                desc = machine_desc,
                backup_tool = backup_tool,
                aur_helper = aur_helper,
                browser = default_apps.browser,
                terminal = default_apps.terminal,
                text_editor = default_apps.text_editor,
                file_manager = default_apps.file_manager,
            );

            fs::write(&host_lua, host_content).context("Failed to create host file")?;
            println!("  {} hosts/{}.lua", "✓".green(), hostname);
        } else {
            // Single-host mode: everything in config.lua
            println!("{} Creating config.lua...", "→".blue());

            let config_content = format!(
                r#"-- mdots configuration
-- This is a dynamic Lua configuration that adapts to your system
-- See LUA-HOSTS.md for full documentation

local hostname = mdots.system.hostname()
local is_laptop = mdots.hardware.is_laptop()
local memory_mb = mdots.system.memory_total_mb()

mdots.log.info(string.format("Loading config for %s (%d MB RAM)", hostname, memory_mb))

-- ═══════════════════════════════════════════════════════════════════
-- MODULE SELECTION
-- ═══════════════════════════════════════════════════════════════════

local enabled_modules = {{
    "base",
    -- Add your modules here
}}

-- Example: Add GPU drivers based on hardware detection
-- if mdots.hardware.has_nvidia() then
--     table.insert(enabled_modules, "nvidia-drivers")
-- elseif mdots.hardware.has_amd_gpu() then
--     table.insert(enabled_modules, "amd-drivers")
-- end

-- Example: Add laptop-specific modules
-- if is_laptop then
--     table.insert(enabled_modules, "laptop-power")
-- end

-- ═══════════════════════════════════════════════════════════════════
-- SERVICES
-- ═══════════════════════════════════════════════════════════════════

local services = {{
    enabled = {{}},
    disabled = {{}},
}}

-- Example: Enable docker if module is enabled
-- if mdots.util.contains(enabled_modules, "docker") then
--     table.insert(services.enabled, "docker.service")
-- end

-- ═══════════════════════════════════════════════════════════════════
-- RETURN CONFIGURATION
-- ═══════════════════════════════════════════════════════════════════

return {{
    host = hostname,
    description = "{desc}",

    enabled_modules = enabled_modules,

    -- Host-specific packages (in addition to modules)
    packages = {{}},

    -- Packages to exclude from modules
    exclude = {{}},

    -- Services configuration
    services = services,

    -- Default applications
    default_apps = {{
        browser = "{browser}",
        terminal = "{terminal}",
        text_editor = "{text_editor}",
        file_manager = "{file_manager}",
    }},

    -- Settings
    flatpak_scope = "user",
    auto_prune = false,
    module_processing = "parallel",
    aur_helper = "{aur_helper}",

    -- Backup settings
    config_backups = {{
        enabled = true,
        max_backups = 5,
    }},

    system_backups = {{
        enabled = true,
        backup_on_sync = true,
        backup_on_update = true,
        tool = "{backup_tool}",
        snapper_config = "root",
    }},
}}
"#,
                desc = machine_desc,
                backup_tool = backup_tool,
                aur_helper = aur_helper,
                browser = default_apps.browser,
                terminal = default_apps.terminal,
                text_editor = default_apps.text_editor,
                file_manager = default_apps.file_manager,
            );

            fs::write(&config_lua, config_content).context("Failed to create config.lua")?;
            println!("  {} config.lua", "✓".green());
        }
    } else {
        println!("  {} config.lua already exists", "→".yellow());
    }

    // Create base.lua module
    let base_module = paths.config_dir.join("modules/base.lua");
    if !base_module.exists() {
        println!("{} Creating modules/base.lua...", "→".blue());

        let base_content = format!(
            r#"-- Base system packages
-- These packages are included regardless of host or modules
-- Uses Lua for conditional package selection based on hardware

local packages = {{
    -- Essential base system
    "base",
    "base-devel",

    -- Kernel (uncomment the one you use)
    -- "linux",              -- Standard kernel
    -- "linux-zen",          -- Zen kernel (optimized for desktop)
    -- "linux-lts",          -- Long-term support kernel

    -- Firmware
    -- "linux-firmware",

    -- Basic tools
    -- "git",
    -- "vim",
    -- "neovim",
    -- "htop",

    -- mdots dependencies (uncomment as needed)
    -- "{aur_helper}",       -- AUR helper
    -- "fzf",                -- Fuzzy finder (for mdots TUI)
{backup_tool_line}
}}

-- Add CPU microcode based on vendor
local cpu = mdots.hardware.cpu_vendor()
if cpu == "intel" then
    mdots.log.info("Intel CPU detected - adding intel-ucode")
    table.insert(packages, "intel-ucode")
elseif cpu == "amd" then
    mdots.log.info("AMD CPU detected - adding amd-ucode")
    table.insert(packages, "amd-ucode")
end

return {{
    description = "Base system packages",
    packages = packages,
}}
"#,
            aur_helper = aur_helper,
            backup_tool_line = if backup_tool == "timeshift" || backup_tool == "snapper" {
                format!(
                    "    \"{}\",               -- System backup tool",
                    backup_tool
                )
            } else {
                "    -- \"timeshift\",          -- System backup tool (or snapper)".to_string()
            }
        );

        fs::write(&base_module, base_content).context("Failed to create base.lua")?;
        println!("  {} modules/base.lua", "✓".green());
    } else {
        println!("  {} modules/base.lua already exists", "→".yellow());
    }

    // Create example hardware module
    let hardware_module = paths.config_dir.join("modules/hardware.lua");
    if !hardware_module.exists() {
        println!("{} Creating modules/hardware.lua...", "→".blue());

        let hardware_content = r#"-- Hardware detection module
-- Automatically installs drivers based on detected hardware

local packages = {}
local description_parts = {}

-- ═══════════════════════════════════════════════════════════════════
-- GPU DRIVERS
-- ═══════════════════════════════════════════════════════════════════

if mdots.hardware.has_nvidia() then
    mdots.log.info("NVIDIA GPU detected")
    table.insert(description_parts, "NVIDIA")

    -- Proprietary drivers
    table.insert(packages, "nvidia")
    table.insert(packages, "nvidia-utils")
    table.insert(packages, "nvidia-settings")
    table.insert(packages, "lib32-nvidia-utils")
end

if mdots.hardware.has_amd_gpu() then
    mdots.log.info("AMD GPU detected")
    table.insert(description_parts, "AMD GPU")

    table.insert(packages, "mesa")
    table.insert(packages, "vulkan-radeon")
    table.insert(packages, "lib32-vulkan-radeon")
    table.insert(packages, "libva-mesa-driver")
end

if mdots.hardware.has_intel_gpu() then
    mdots.log.info("Intel GPU detected")
    table.insert(description_parts, "Intel GPU")

    table.insert(packages, "mesa")
    table.insert(packages, "vulkan-intel")
    table.insert(packages, "intel-media-driver")
end

-- ═══════════════════════════════════════════════════════════════════
-- LAPTOP PACKAGES
-- ═══════════════════════════════════════════════════════════════════

if mdots.hardware.is_laptop() then
    mdots.log.info("Laptop detected - adding power management")
    table.insert(description_parts, "Laptop")

    table.insert(packages, "tlp")
    table.insert(packages, "tlp-rdw")
    table.insert(packages, "powertop")
    table.insert(packages, "acpi")
end

-- Build description
local description = "Hardware drivers"
if #description_parts > 0 then
    description = description .. " (" .. table.concat(description_parts, ", ") .. ")"
end

-- Services for laptop
local services = { enabled = {}, disabled = {} }
if mdots.hardware.is_laptop() then
    table.insert(services.enabled, "tlp.service")
    table.insert(services.disabled, "power-profiles-daemon.service")
end

return {
    description = description,
    packages = packages,
    services = services,
}
"#;

        fs::write(&hardware_module, hardware_content).context("Failed to create hardware.lua")?;
        println!("  {} modules/hardware.lua", "✓".green());
    }

    // Create example gaming module
    let gaming_module = paths.config_dir.join("modules/gaming.lua");
    if !gaming_module.exists() {
        println!("{} Creating modules/gaming.lua (example)...", "→".blue());

        let gaming_content = r#"-- Gaming module example
-- Delete or customize this file

local packages = {
    "steam",
    "lutris",
    "gamemode",
    "lib32-gamemode",
    "mangohud",
    "lib32-mangohud",
}

-- Add Wine for non-native games
table.insert(packages, "wine")
table.insert(packages, "wine-mono")
table.insert(packages, "winetricks")

-- Add Discord via Flatpak
table.insert(packages, "flatpak:com.discordapp.Discord")

return {
    description = "Gaming packages",
    packages = packages,
    conflicts = { "minimal" },

    services = {
        enabled = { "gamemode.service" },
    },
}
"#;

        fs::write(&gaming_module, gaming_content).context("Failed to create gaming.lua")?;
        println!("  {} modules/gaming.lua (example)", "✓".green());
    }

    println!();
    println!(
        "{}",
        "✓ arch-config initialized with Lua configuration!".green()
    );
    println!();
    println!("{}", "Structure:".bold());
    if multi_host {
        println!("  config.lua           → Pointer to hosts/{}.lua", hostname);
        println!("  hosts/{}.lua    → Your full configuration", hostname);
    } else {
        println!("  config.lua           → Main configuration (dynamic)");
    }
    println!("  modules/base.lua     → Base packages with hardware detection");
    println!("  modules/hardware.lua → Auto-detected GPU/laptop drivers");
    println!("  modules/gaming.lua   → Example gaming module");
    println!("  scripts/             → Hook scripts");
    println!("  docs/                → mdots documentation");
    println!();
    println!("{}", "Available mdots APIs in Lua:".bold());
    println!("  mdots.hardware        → cpu_vendor(), has_nvidia(), is_laptop(), etc.");
    println!("  mdots.system          → hostname(), memory_total_mb(), cpu_cores(), etc.");
    println!("  mdots.package         → is_installed(), version(), flatpak_installed()");
    println!("  mdots.env             → home(), user(), config_dir()");
    println!("  mdots.util            → contains(), extend(), merge()");
    println!("  mdots.log             → info(), warn(), debug(), error()");
    println!();
    println!("Next steps:");
    if multi_host {
        println!(
            "  1. Edit config: {}",
            format!("hosts/{}.lua", hostname).cyan()
        );
    } else {
        println!("  1. Edit config: {}", "config.lua".cyan());
    }
    println!("  2. Review modules: {}", "mdots module list".cyan());
    println!(
        "  3. Enable hardware module: {}",
        "mdots module enable hardware".cyan()
    );
    println!("  4. Validate: {}", "mdots validate".cyan());
    println!("  5. Preview: {}", "mdots sync --dry-run".cyan());
    println!();
    println!("Documentation:");
    println!(
        "  • See {} for Lua module API reference",
        "LUA-MODULES.md".cyan()
    );
    println!(
        "  • See {} for host config documentation",
        "LUA-HOSTS.md".cyan()
    );

    Ok(())
}

/// Bootstrap from BlackDon's config repository
fn bootstrap_blackdon_config(paths: &ConfigPaths) -> Result<()> {
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

/// Detect which backup tool is installed
fn detect_backup_tool() -> String {
    // Check for timeshift first (most common)
    if which::which("timeshift").is_ok() {
        println!(
            "  {} Detected backup tool: {}",
            "→".blue(),
            "timeshift".green()
        );
        return "timeshift".to_string();
    }

    // Check for snapper
    if which::which("snapper").is_ok() {
        println!(
            "  {} Detected backup tool: {}",
            "→".blue(),
            "snapper".green()
        );
        return "snapper".to_string();
    }

    // Default to timeshift if neither is found
    println!(
        "  {} No backup tool detected, defaulting to: {}",
        "→".yellow(),
        "timeshift".cyan()
    );
    println!("    Install with: sudo pacman -S timeshift");
    println!("    Or: sudo pacman -S snapper");
    "timeshift".to_string()
}

/// Detect which AUR helper is installed
fn detect_aur_helper() -> String {
    // Check for paru first (preferred)
    if which::which("paru").is_ok() {
        return "paru".to_string();
    }

    // Check for yay
    if which::which("yay").is_ok() {
        return "yay".to_string();
    }

    // Default to paru if neither is found
    "paru".to_string()
}

/// Detect default applications using xdg-mime
fn detect_default_apps() -> DefaultApps {
    DefaultApps {
        browser: detect_xdg_default("x-scheme-handler/http")
            .unwrap_or_else(|| "firefox".to_string()),
        terminal: detect_terminal().unwrap_or_else(|| "kitty".to_string()),
        text_editor: detect_xdg_default("text/plain").unwrap_or_else(|| "code".to_string()),
        file_manager: detect_xdg_default("inode/directory").unwrap_or_else(|| "thunar".to_string()),
    }
}

struct DefaultApps {
    browser: String,
    terminal: String,
    text_editor: String,
    file_manager: String,
}

/// Query xdg-mime for a default application
fn detect_xdg_default(mime_type: &str) -> Option<String> {
    let output = std::process::Command::new("xdg-mime")
        .args(["query", "default", mime_type])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let desktop_file = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if desktop_file.is_empty() {
        return None;
    }

    // Strip .desktop extension and org.* prefix for cleaner names
    let name = desktop_file
        .strip_suffix(".desktop")
        .unwrap_or(&desktop_file)
        .to_string();

    // Handle org.kde.* and org.gnome.* prefixes
    let name = if let Some(stripped) = name.strip_prefix("org.kde.") {
        stripped.to_string()
    } else if let Some(stripped) = name.strip_prefix("org.gnome.") {
        stripped.to_string()
    } else {
        name
    };

    Some(name)
}

/// Detect default terminal (terminals don't have a standard MIME type)
fn detect_terminal() -> Option<String> {
    // Check $TERMINAL environment variable first
    if let Ok(terminal) = std::env::var("TERMINAL") {
        if !terminal.is_empty() {
            // Extract just the binary name
            let name = std::path::Path::new(&terminal)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&terminal);
            return Some(name.to_string());
        }
    }

    // Check for common terminals in order of preference
    let terminals = [
        "kitty",
        "alacritty",
        "foot",
        "wezterm",
        "konsole",
        "gnome-terminal",
        "xfce4-terminal",
        "xterm",
    ];

    for term in terminals {
        if which::which(term).is_ok() {
            return Some(term.to_string());
        }
    }

    None
}

/// Copy user-facing documentation to arch-config/docs
fn copy_user_docs(config_dir: &Path) -> Result<()> {
    // Find the mdots installation directory by looking for the docs folder
    // Try common locations in order of preference
    let exe_path = std::env::current_exe().ok();

    // Build list of possible documentation paths
    let mut possible_paths: Vec<std::path::PathBuf> = Vec::new();

    // Development: relative to executable (target/debug or target/release)
    // e.g., /home/user/mdots/target/release/mdots -> /home/user/mdots/docs
    if let Some(ref exe) = exe_path {
        if let Some(docs_path) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.join("docs"))
        {
            possible_paths.push(docs_path);
        }
    }

    // Compile-time path: embedded from CARGO_MANIFEST_DIR during build.
    // Debug builds only — baking the absolute manifest dir into a *release*
    // binary leaks the build directory (makepkg's "reference to $srcdir"
    // warning) and is useless off the build machine anyway. `env!` values
    // are not touched by `--remap-path-prefix`, so gating it out is the only
    // way to keep the packaged binary clean. Dev iteration (`cargo run`) is
    // already covered by the exe-relative path above; system/local installs
    // use the `/usr/share/mdots/docs` and `~/.local/share/mdots/docs` paths
    // below.
    #[cfg(debug_assertions)]
    {
        let manifest_docs = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs");
        possible_paths.push(manifest_docs);
    }

    // Installed: /usr/share/mdots/docs (system-wide install)
    possible_paths.push(std::path::PathBuf::from("/usr/share/mdots/docs"));

    // Local install: ~/.local/share/mdots/docs
    if let Some(data_local) = dirs::data_local_dir() {
        possible_paths.push(data_local.join("mdots/docs"));
    }

    let docs_src = possible_paths
        .into_iter()
        .find(|p| p.exists() && p.is_dir());

    let Some(docs_src) = docs_src else {
        println!(
            "  {} Documentation not found, skipping docs copy",
            "→".yellow()
        );
        return Ok(());
    };

    let docs_dest = config_dir.join("docs");
    fs::create_dir_all(&docs_dest).context("Failed to create docs directory")?;

    let mut copied = 0;
    for doc_file in USER_DOCS {
        let src_file = docs_src.join(doc_file);
        let dest_file = docs_dest.join(doc_file);

        if src_file.exists() {
            fs::copy(&src_file, &dest_file)
                .with_context(|| format!("Failed to copy {}", doc_file))?;
            copied += 1;
        }
    }

    if copied > 0 {
        println!("  {} docs/ ({} files)", "✓".green(), copied);
    }

    Ok(())
}

/// Recursively copy a directory
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    fs::create_dir_all(dst)
        .with_context(|| format!("Failed to create directory: {}", dst.display()))?;

    for entry in
        fs::read_dir(src).with_context(|| format!("Failed to read directory: {}", src.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            // Skip .git directory
            if entry.file_name() == ".git" {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_symlink() {
            // Handle symlinks - read the link and recreate it
            match fs::read_link(&src_path) {
                Ok(link_target) => {
                    // Try to create the symlink, but don't fail if it errors
                    // (broken symlinks are common in dotfiles)
                    let _ = std::os::unix::fs::symlink(&link_target, &dst_path);
                }
                Err(_) => {
                    // Skip broken symlinks
                    continue;
                }
            }
        } else {
            // Regular file - copy it
            match fs::copy(&src_path, &dst_path) {
                Ok(_) => {}
                Err(e) => {
                    // Log warning but continue with other files
                    eprintln!("Warning: Failed to copy {}: {}", src_path.display(), e);
                }
            }
        }
    }

    Ok(())
}

/// Create Lua type definitions for editor support (silences "undefined global 'mdots'" warnings)
fn create_lua_type_definitions(config_dir: &Path) -> Result<()> {
    // Create .luarc.json for lua-language-server configuration
    let luarc_path = config_dir.join(".luarc.json");
    if !luarc_path.exists() {
        let luarc_content = r#"{
    "workspace.library": [".lua"],
    "diagnostics.globals": ["mdots"],
    "runtime.version": "Lua 5.4"
}
"#;
        fs::write(&luarc_path, luarc_content).context("Failed to create .luarc.json")?;
    }

    // Create .lua directory for type definitions
    let lua_types_dir = config_dir.join(".lua");
    fs::create_dir_all(&lua_types_dir).context("Failed to create .lua directory")?;

    // Create mdots type definitions
    let mdots_types_path = lua_types_dir.join("mdots.lua");
    let mdots_types_content = r#"---@meta
-- mdots Lua API type definitions
-- This file provides type hints for editors/IDEs

---@class mdots
---@field hardware mdots.hardware
---@field system mdots.system
---@field package mdots.package
---@field file mdots.file
---@field env mdots.env
---@field util mdots.util
---@field log mdots.log
---@field service mdots.service
---@field power mdots.power
---@field security mdots.security
---@field desktop mdots.desktop
---@field boot mdots.boot
---@field network mdots.network
---@field audio mdots.audio
---@field storage mdots.storage
mdots = {}

---@class mdots.hardware
---@field cpu_vendor fun(): string Returns "intel", "amd", or "unknown"
---@field gpu_vendors fun(): string[] Returns array of GPU vendors
---@field has_nvidia fun(): boolean Check if NVIDIA GPU is present
---@field has_amd_gpu fun(): boolean Check if AMD GPU is present
---@field has_intel_gpu fun(): boolean Check if Intel GPU is present
---@field is_laptop fun(): boolean Check if system is a laptop
---@field has_battery fun(): boolean Check if battery is present
---@field chassis_type fun(): string Returns "desktop", "laptop", "server", "tablet", or "unknown"
mdots.hardware = {}

---@class mdots.system
---@field hostname fun(): string Get system hostname
---@field kernel_version fun(): string Get kernel version
---@field arch fun(): string Get system architecture
---@field os fun(): string Get operating system
---@field distro fun(): string Get distribution ID
---@field distro_name fun(): string Get full distribution name
---@field distro_version fun(): string Get distribution version
---@field memory_total_mb fun(): number Get total RAM in MB
---@field cpu_cores fun(): number Get number of CPU cores
mdots.system = {}

---@class mdots.package
---@field is_installed fun(name: string): boolean Check if package is installed
---@field version fun(name: string): string|nil Get package version
---@field is_available fun(name: string): boolean Check if package is in repos
---@field repo fun(name: string): string|nil Get package repository
---@field is_foreign fun(name: string): boolean Check if package is from AUR
---@field list_installed fun(): string[] Get all installed packages
---@field list_explicit fun(): string[] Get explicitly installed packages
---@field flatpak_installed fun(id: string): boolean Check if flatpak is installed
---@field flatpak_version fun(id: string): string|nil Get flatpak version
---@field aur_available fun(name: string): boolean Check if package is in AUR
mdots.package = {}

---@class mdots.file
---@field exists fun(path: string): boolean Check if file/directory exists
---@field is_file fun(path: string): boolean Check if path is a file
---@field is_dir fun(path: string): boolean Check if path is a directory
---@field read fun(path: string): string|nil Read file contents (sandboxed)
---@field read_lines fun(path: string): string[]|nil Read file as lines (sandboxed)
mdots.file = {}

---@class mdots.env
---@field get fun(name: string): string|nil Get environment variable
---@field home fun(): string Get home directory
---@field user fun(): string Get current username
---@field config_dir fun(): string Get XDG config directory
---@field data_dir fun(): string Get XDG data directory
---@field cache_dir fun(): string Get XDG cache directory
---@field shell fun(): string Get user's default shell
mdots.env = {}

---@class mdots.util
---@field contains fun(tbl: table, value: any): boolean Check if array contains value
---@field keys fun(tbl: table): any[] Get table keys
---@field values fun(tbl: table): any[] Get table values
---@field merge fun(t1: table, t2: table): table Merge tables (t2 overrides t1)
---@field extend fun(target: table, source: table): table Append source to target
---@field split fun(str: string, delim: string): string[] Split string by delimiter
---@field trim fun(str: string): string Remove leading/trailing whitespace
---@field starts_with fun(str: string, prefix: string): boolean Check string prefix
---@field ends_with fun(str: string, suffix: string): boolean Check string suffix
---@field version_compare fun(v1: string, v2: string): number Compare versions (-1, 0, 1)
---@field version_gte fun(v1: string, v2: string): boolean Check if v1 >= v2
---@field version_gt fun(v1: string, v2: string): boolean Check if v1 > v2
---@field version_lte fun(v1: string, v2: string): boolean Check if v1 <= v2
---@field version_lt fun(v1: string, v2: string): boolean Check if v1 < v2
mdots.util = {}

---@class mdots.log
---@field info fun(msg: string) Log info message
---@field warn fun(msg: string) Log warning message
---@field debug fun(msg: string) Log debug message
---@field error fun(msg: string) Log error message
mdots.log = {}

---@class mdots.service
---@field is_enabled fun(name: string): boolean Check if service is enabled
---@field is_active fun(name: string): boolean Check if service is active
---@field is_running fun(name: string): boolean Alias for is_active
---@field exists fun(name: string): boolean Check if service unit exists
---@field status fun(name: string): string Get service status
---@field list_enabled fun(): string[] Get enabled services
---@field list_active fun(): string[] Get active services
---@field list_failed fun(): string[] Get failed services
---@field is_user_service fun(name: string): boolean Check user service status
mdots.service = {}

---@class mdots.power
---@field on_battery fun(): boolean Check if on battery power
---@field on_ac fun(): boolean Check if on AC power
---@field battery_percent fun(): number|nil Get battery percentage
---@field battery_status fun(): string Get battery status
---@field has_suspend fun(): boolean Check suspend support
---@field has_hibernate fun(): boolean Check hibernate support
---@field cpu_governor fun(): string Get CPU governor
---@field available_governors fun(): string[] Get available governors
---@field supports_turbo fun(): boolean Check turbo boost support
---@field turbo_enabled fun(): boolean Check if turbo is enabled
mdots.power = {}

---@class mdots.security
---@field has_selinux fun(): boolean Check SELinux availability
---@field selinux_enabled fun(): boolean Check if SELinux is enabled
---@field has_apparmor fun(): boolean Check AppArmor availability
---@field apparmor_enabled fun(): boolean Check if AppArmor is enabled
---@field has_secureboot fun(): boolean Check Secure Boot support
---@field secureboot_enabled fun(): boolean Check if Secure Boot is enabled
---@field has_tpm fun(): boolean Check TPM presence
---@field tpm_version fun(): string|nil Get TPM version
---@field firewall_active fun(): boolean Check if firewall is active
---@field firewall_type fun(): string Get firewall type
---@field has_luks fun(): boolean Check for LUKS encryption
---@field kernel_lockdown fun(): string Get kernel lockdown mode
mdots.security = {}

---@class mdots.desktop
---@field environment fun(): string Get desktop environment
---@field display_server fun(): string Get display server type
---@field is_wayland fun(): boolean Check if running Wayland
---@field is_x11 fun(): boolean Check if running X11
---@field window_manager fun(): string Get window manager
---@field session_type fun(): string Get session type
---@field has_display fun(): boolean Check if display is available
---@field compositor fun(): string|nil Get compositor name
---@field theme fun(): string|nil Get desktop theme
---@field icon_theme fun(): string|nil Get icon theme
---@field screen_resolution fun(): string|nil Get screen resolution
mdots.desktop = {}

---@class mdots.boot
---@field bootloader fun(): string Get bootloader name
---@field is_uefi fun(): boolean Check if UEFI boot
---@field is_bios fun(): boolean Check if BIOS boot
---@field init_system fun(): string Get init system
---@field kernel_params fun(): string[] Get kernel parameters
---@field has_kernel_param fun(param: string): boolean Check kernel parameter
---@field efi_vars_supported fun(): boolean Check EFI variable support
---@field boot_id fun(): string Get boot session ID
mdots.boot = {}

---@class mdots.network
---@field has_wifi fun(): boolean Check WiFi hardware
---@field has_ethernet fun(): boolean Check Ethernet hardware
---@field has_bluetooth fun(): boolean Check Bluetooth hardware
---@field is_connected fun(): boolean Check network connectivity
---@field connection_type fun(): string Get connection type
---@field list_interfaces fun(): string[] Get network interfaces
mdots.network = {}

---@class mdots.audio
---@field has_pipewire fun(): boolean Check if PipeWire is running
---@field has_pulseaudio fun(): boolean Check if PulseAudio is running
---@field has_alsa fun(): boolean Check ALSA availability
---@field audio_server fun(): string Get audio server type
mdots.audio = {}

---@class mdots.storage
---@field has_ssd fun(): boolean Check for SSD
---@field has_nvme fun(): boolean Check for NVMe drive
---@field has_hdd fun(): boolean Check for HDD
---@field root_filesystem fun(): string Get root filesystem type
---@field list_disks fun(): string[] Get disk devices
---@field disk_info fun(device: string): table|nil Get disk information
mdots.storage = {}

return mdots
"#;

    fs::write(&mdots_types_path, mdots_types_content)
        .context("Failed to create mdots type definitions")?;

    println!("  {} .lua/mdots.lua (editor type hints)", "✓".green());

    Ok(())
}

/// Initialize mdots configuration with Nix configuration files
fn run_nix_config_init(paths: &ConfigPaths) -> Result<()> {
    println!("{}", "=== Initializing mdots (Nix Mode) ===".blue());
    println!();
    println!(
        "{}",
        "This will set up your configuration using Nix files instead of YAML.".cyan()
    );
    println!(
        "{}",
        "Nix files allow dynamic, conditional configuration based on system facts.".cyan()
    );
    println!();

    if !crate::nix_eval::is_nix_installed() {
        println!(
            "{}",
            "Nix is not installed. Nix config files require the Nix package manager.".yellow()
        );
        println!(
            "{}",
            "  Install Nix first: curl -L https://nixos.org/nix/install | sh".yellow()
        );
        println!("{}", "  Or run: mdots init --nix-init".yellow());
        println!();
        print!("Continue anyway? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }
        println!();
    }

    if paths.config_dir.exists() {
        println!("{}", "mdots config directory already exists".yellow());
        println!("Location: {}", paths.config_dir.display());
        print!("Reinitialize with Nix config? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }
    }

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

    println!("{} Creating directory structure...", "→".blue());

    fs::create_dir_all(&paths.config_dir)?;
    fs::create_dir_all(paths.config_dir.join("hosts"))?;
    fs::create_dir_all(paths.config_dir.join("modules"))?;
    fs::create_dir_all(paths.config_dir.join("scripts"))?;
    fs::create_dir_all(&paths.state_dir)?;

    println!("  {} {}", "✓".green(), paths.config_dir.display());
    println!("  {} {}/hosts", "✓".green(), paths.config_dir.display());
    println!("  {} {}/modules", "✓".green(), paths.config_dir.display());
    println!("  {} {}/scripts", "✓".green(), paths.config_dir.display());
    println!("  {} {}/state", "✓".green(), paths.config_dir.display());

    copy_user_docs(&paths.config_dir)?;

    let state_gitignore = paths.state_dir.join(".gitignore");
    if !state_gitignore.exists() {
        fs::write(&state_gitignore, "# Ignore all state files\n*\n")?;
        println!("  {} state/.gitignore", "✓".green());
    }

    let root_gitignore = paths.config_dir.join(".gitignore");
    let gitignore_content = "# System packages merged from host (auto-generated, host-specific)\nsystem-packages-*.yaml\n";
    if !root_gitignore.exists() {
        fs::write(&root_gitignore, gitignore_content)?;
        println!("  {} .gitignore", "✓".green());
    }

    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());

    // Create config.nix as pointer
    let config_nix = paths.config_dir.join("config.nix");
    if !config_nix.exists() {
        println!("{} Creating config.nix (pointer)...", "→".blue());
        let config_content = crate::nix_eval::generate_pointer_config_nix(&hostname);
        fs::write(&config_nix, config_content)?;
        println!("  {} config.nix", "✓".green());
    } else {
        println!("  {} config.nix already exists", "→".yellow());
    }

    // Create host config
    let host_nix = paths.config_dir.join(format!("hosts/{}.nix", hostname));
    if !host_nix.exists() {
        println!("{} Creating hosts/{}.nix...", "→".blue(), hostname);
        let host_content = crate::nix_eval::generate_config_nix(&hostname, pkg_manager_str);
        fs::write(&host_nix, host_content)?;
        println!("  {} hosts/{}.nix", "✓".green(), hostname);
    } else {
        println!("  {} hosts/{}.nix already exists", "→".yellow(), hostname);
    }

    // Create base module
    let base_nix = paths.config_dir.join("modules/base.nix");
    if !base_nix.exists() {
        println!("{} Creating modules/base.nix...", "→".blue());
        let base_content = crate::nix_eval::generate_module_nix("Base system packages");
        fs::write(&base_nix, base_content)?;
        println!("  {} modules/base.nix", "✓".green());
    } else {
        println!("  {} modules/base.nix already exists", "→".yellow());
    }

    // Create example module
    let example_nix = paths.config_dir.join("modules/example.nix");
    if !example_nix.exists() {
        println!("{} Creating modules/example.nix...", "→".blue());
        let example_content = crate::nix_eval::generate_module_nix("Example module");
        fs::write(&example_nix, example_content)?;
        println!("  {} modules/example.nix", "✓".green());
    }

    println!();
    println!("{}", "✓ mdots initialized with Nix configuration!".green());
    println!();
    println!("{}", "Structure:".bold());
    println!("  config.nix          → Points to hosts/{}.nix", hostname);
    println!("  hosts/{}.nix   → Your full configuration", hostname);
    println!("  modules/base.nix    → Base packages");
    println!("  modules/             → Optional package modules");
    println!("  scripts/             → Post-install hook scripts");
    println!();
    println!("{}", "Nix config advantages:".bold());
    println!("  • Conditional packages based on system facts");
    println!("  • Access to Nixpkgs via 'with pkgs;'");
    println!("  • Separate: packages, flatpak_packages, nix_packages");
    println!();
    println!("Next steps:");
    println!("  1. Edit host config: hosts/{}.nix", hostname);
    println!("  2. Edit base packages: modules/base.nix");
    println!("  3. Run: mdots validate");
    println!("  4. Run: mdots module list");
    println!("  5. Run: mdots sync --dry-run");

    Ok(())
}

/// Initialize Nix and Home Manager integration
fn run_nix_init(paths: &ConfigPaths) -> Result<()> {
    println!(
        "{}",
        "=== Initializing Nix & Home Manager Integration ===".blue()
    );
    println!();

    // Step 1: Install nix if not already installed (do this FIRST,
    // before load_config which may try to evaluate config.nix)
    if crate::nix::is_nix_installed() {
        println!("{} Nix is already installed", "✓".green());

        // Ensure nix-daemon is running (needed before any nix eval)
        if !crate::nix::is_nix_daemon_running() {
            println!("{} nix-daemon is not running, starting it...", "→".blue());
            if let Err(e) = crate::nix::start_nix_daemon() {
                eprintln!(
                    "{} Failed to start nix-daemon: {}\n  Please start it manually: sudo systemctl start nix-daemon",
                    "✗".red(),
                    e
                );
                std::process::exit(1);
            }
            println!("{} nix-daemon started successfully", "✓".green());
        }
    } else {
        // Autodetect package manager for nix installation (can't load config yet)
        let pm_type = crate::config::detect_package_manager_type()?;
        println!("{} Installing Nix...", "→".blue());
        crate::nix::install_nix(&pm_type)?;
        println!("{} Nix installed successfully", "✓".green());
        println!();
        println!(
            "{}",
            "Note: You may need to log out and log back in for PATH changes to take effect."
                .yellow()
        );
        println!(
            "{}",
            "Then run 'mdots init --nix-init' again to continue.".yellow()
        );
        return Ok(());
    }

    // Now it's safe to load the config (nix and daemon are running)
    let config = load_config(paths)?;

    // Step 2: Offer to migrate existing home-manager config
    let migrated = crate::nix::migrate_existing_hm(paths)?;
    if migrated {
        println!("{} Existing home-manager config migrated", "✓".green());
    } else {
        // Step 3: Create home-manager directory
        println!("{} Creating home-manager directory...", "→".blue());
        std::fs::create_dir_all(paths.home_manager_dir())
            .context("Failed to create home-manager directory")?;

        // Get username and home directory
        let username = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "user".to_string());
        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("/home/{}", username));

        // Generate home.nix template
        let home_nix_path = paths.home_manager_dir().join("home.nix");
        crate::nix::generate_home_nix_template(&username, &home_dir, &home_nix_path)?;
        println!("  {} Created {}", "✓".green(), home_nix_path.display());

        // Generate empty mdots-packages.nix
        let mdots_packages_path = paths.home_manager_dir().join("mdots-packages.nix");
        crate::nix::generate_mdots_packages_nix(&[], &mdots_packages_path)?;
        println!(
            "  {} Created {}",
            "✓".green(),
            mdots_packages_path.display()
        );
    }

    // Step 4: Ask about flakes
    println!();
    print!("Enable flakes for Home Manager? (Nix flakes provide better reproducibility, recommended) [Y/n] ");
    io::stdout().flush()?;
    let mut flake_input = String::new();
    io::stdin().read_line(&mut flake_input)?;
    let use_flakes = !flake_input.trim().eq_ignore_ascii_case("n");
    println!(
        "  {} Using {} mode",
        "→".blue(),
        if use_flakes { "flake" } else { "channel" }
    );

    if use_flakes {
        let hostname = config.host.clone();
        let hm_dir = paths.home_manager_dir();

        // Create hosts/ directory structure if migrating from flat
        if !hm_dir.join("hosts").exists() {
            // Migrate existing mdots-packages.nix from root if it exists
            let old_mdots = hm_dir.join("mdots-packages.nix");
            if old_mdots.exists() {
                println!(
                    "  {} Migrating from flat to per-host structure...",
                    "→".blue()
                );
            }
        }

        // Create this host's per-host directory
        let host_dir = hm_dir.join("hosts").join(&hostname);
        let is_new_host = !host_dir.exists();
        std::fs::create_dir_all(&host_dir)
            .context("Failed to create host home-manager directory")?;

        // Generate mdots-packages.nix for this host
        let mdots_packages_path = host_dir.join("mdots-packages.nix");
        if !mdots_packages_path.exists() || is_new_host {
            crate::nix::generate_mdots_packages_nix(&[], &mdots_packages_path)?;
            println!(
                "  {} Created hosts/{}/mdots-packages.nix",
                "✓".green(),
                hostname
            );
        }

        // Create packages.nix hint for this host (if not exists)
        let packages_nix = host_dir.join("packages.nix");
        if !packages_nix.exists() {
            let hint = format!(
                "{{ pkgs, ... }}:\n{{\n  # Per-host packages for {}\n  home.packages = with pkgs; [\n    # Add packages here\n  ];\n}}\n",
                hostname
            );
            std::fs::write(&packages_nix, hint)?;
            println!("  {} Created hosts/{}/packages.nix", "✓".green(), hostname);
        }

        // Generate shared home.nix (only if it doesn't exist)
        let home_nix_path = hm_dir.join("home.nix");
        if !home_nix_path.exists() {
            crate::nix::generate_shared_home_nix(&home_nix_path)?;
            println!("  {} Created shared home.nix", "✓".green());
        }

        // Generate per-host flake.nix (only if it doesn't exist)
        let flake_nix_path = hm_dir.join("flake.nix");
        if !flake_nix_path.exists() {
            let system_arch = crate::nix::detect_system_arch();
            crate::nix::generate_per_host_flake_nix(&system_arch, &flake_nix_path)?;
            println!("  {} Created flake.nix", "✓".green());
        }

        // Generate flake.lock (only if it doesn't exist)
        if !hm_dir.join("flake.lock").exists() {
            crate::nix::generate_flake_lock(paths)?;
        }
    }

    // Step 5: Set up channels (only if not using flakes)
    if !use_flakes {
        if crate::nix::is_home_manager_installed() {
            println!("{} Home Manager is already installed", "✓".green());
        } else {
            println!("{} Setting up nix channels...", "→".blue());
            crate::nix::setup_channels(
                &config.nix.nixpkgs_channel,
                &config.nix.home_manager_channel,
            )?;
            println!("{} Channels set up", "✓".green());

            println!("{} Installing Home Manager...", "→".blue());
            crate::nix::install_home_manager()?;
            println!("{} Home Manager installed successfully", "✓".green());
        }
    } else {
        if crate::nix::is_home_manager_installed() {
            println!("{} Home Manager is already installed", "✓".green());
        } else {
            println!("{} Installing Home Manager via nix-shell...", "→".blue());
            // For flakes, we still need home-manager binary installed
            // Use the channel approach for the binary even if config uses flakes
            crate::nix::setup_channels(
                &config.nix.nixpkgs_channel,
                &config.nix.home_manager_channel,
            )?;
            crate::nix::install_home_manager()?;
            println!("{} Home Manager installed successfully", "✓".green());
        }
    }

    // Step 6: Update host config to enable nix
    println!("{} Enabling Nix in host config...", "→".blue());
    update_host_config_nix(paths, use_flakes)?;

    // Step 7: Ensure nix is on PATH for the user's shell
    ensure_nix_on_path();

    println!();
    println!(
        "{}",
        "✓ Nix & Home Manager integration initialized!".green()
    );
    println!();
    println!(
        "Home Manager config location: {}",
        paths.home_manager_dir().display()
    );
    if use_flakes {
        println!("Mode: Flakes (flake.nix + flake.lock)");
        println!();
        println!("Next steps:");
        println!(
            "  1. Edit your config: {}",
            "nano ~/.config/mdots/home-manager/home.nix".cyan()
        );
        println!(
            "  2. Add nix packages to modules with {}",
            "type: nix".cyan()
        );
        println!("  3. Run {} to apply changes", "mdots sync".cyan());
        println!(
            "  4. Run {} to update flake inputs and apply",
            "mdots nix update".cyan()
        );
    } else {
        println!("Mode: Channels");
        println!();
        println!("Next steps:");
        println!(
            "  1. Edit your config: {}",
            "nano ~/.config/mdots/home-manager/home.nix".cyan()
        );
        println!(
            "  2. Add nix packages to modules with {}",
            "type: nix".cyan()
        );
        println!("  3. Run {} to apply changes", "mdots sync".cyan());
        println!(
            "  4. Run {} to manually apply home-manager",
            "mdots nix switch".cyan()
        );
    }

    Ok(())
}

/// Update host config to enable nix
fn update_host_config_nix(paths: &ConfigPaths, use_flakes: bool) -> Result<()> {
    let config_path = crate::config::resolve_config_path(paths)?;

    if config_path.extension().and_then(|e| e.to_str()) == Some("lua") {
        // For Lua configs, we just print instructions
        println!(
            "  {} Lua config detected. Add this to your config:",
            "→".yellow()
        );
        println!("    nix = {{");
        println!("        enabled = true,");
        println!("        home_manager_enabled = true,");
        if use_flakes {
            println!("        flake_enabled = true,");
        }
        println!("    }}");
        return Ok(());
    }

    // For Nix configs, print instructions (injecting into Nix attrsets is fragile)
    if config_path.extension().and_then(|e| e.to_str()) == Some("nix") {
        println!(
            "  {} Nix config detected. Add this inside your host attribute set:",
            "→".yellow()
        );
        println!("    nix = {{");
        println!("      enabled = true;");
        println!("      home_manager_enabled = true;");
        if use_flakes {
            println!("      flake_enabled = true;");
        }
        println!("      nixpkgs_channel = \"nixpkgs-unstable\";");
        println!("      home_manager_channel = \"release-25.05\";");
        println!("    }};");
        return Ok(());
    }

    // For YAML configs, add nix section
    let content = std::fs::read_to_string(&config_path)?;

    // Check if nix section already exists
    if content.contains("nix:") {
        // Update flake_enabled if needed
        if use_flakes && !content.contains("flake_enabled") {
            let updated = content.replace(
                "home_manager_enabled: true",
                "home_manager_enabled: true\n  flake_enabled: true",
            );
            std::fs::write(&config_path, updated)?;
            println!("  {} Added flake_enabled: true to nix config", "✓".green());
        } else {
            println!("  {} Nix section already exists in config", "→".yellow());
        }
        return Ok(());
    }

    // Add nix section at the end
    let flake_line = if use_flakes {
        "  flake_enabled: true\n"
    } else {
        ""
    };
    let nix_section = format!(
        r#"
# Nix integration
nix:
  enabled: true
  home_manager_enabled: true
  {flake_line}  nixpkgs_channel: nixpkgs-unstable
  home_manager_channel: release-25.05
"#,
        flake_line = flake_line
    );

    let updated = format!("{}{}", content, nix_section);
    std::fs::write(&config_path, updated)?;
    println!(
        "  {} Added nix section to {}",
        "✓".green(),
        config_path.display()
    );

    Ok(())
}

/// Detect the user's shell from $SHELL and ensure ~/.nix-profile/bin is on PATH
fn ensure_nix_on_path() {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let shell_name = std::path::Path::new(&shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let nix_profile_bin = "$HOME/.nix-profile/bin";
    let path_line: Option<String> = match shell_name {
        "fish" => {
            let fish_conf_d = dirs::home_dir()
                .map(|h| h.join(".config/fish/conf.d"))
                .unwrap_or_else(|| std::path::PathBuf::from("~/.config/fish/conf.d"));
            let nix_file = fish_conf_d.join("nix.fish");

            if !nix_file.exists() {
                let _ = std::fs::create_dir_all(&fish_conf_d);
                let content = format!(
                    "{}\n{}\n",
                    "# nix profile PATH set by mdots",
                    "fish_add_path --move --prepend $HOME/.nix-profile/bin"
                );
                if std::fs::write(&nix_file, &content).is_ok() {
                    println!(
                        "{} Added {} to {}",
                        "✓".green(),
                        nix_profile_bin.cyan(),
                        nix_file.display()
                    );
                }
            }
            None
        }
        "bash" => Some(format!(
            "\n# nix profile PATH set by mdots\nexport PATH=\"{}:\"$PATH\n",
            nix_profile_bin
        )),
        "zsh" => Some(format!(
            "\n# nix profile PATH set by mdots\nexport PATH=\"{}:\"$PATH\n",
            nix_profile_bin
        )),
        _ => None,
    };

    if let Some(line) = path_line {
        let rc_file = match shell_name {
            "bash" => dirs::home_dir()
                .map(|h| h.join(".bashrc"))
                .unwrap_or_else(|| std::path::PathBuf::from("~/.bashrc")),
            "zsh" => dirs::home_dir()
                .map(|h| h.join(".zshrc"))
                .unwrap_or_else(|| std::path::PathBuf::from("~/.zshrc")),
            _ => return,
        };

        if let Ok(existing) = std::fs::read_to_string(&rc_file) {
            if !existing.contains("mdots")
                && !existing.contains(nix_profile_bin)
                && std::fs::write(&rc_file, format!("{}{}", existing, line)).is_ok()
            {
                println!(
                    "{} Added {} to {}",
                    "✓".green(),
                    nix_profile_bin.cyan(),
                    rc_file.display()
                );
            }
        }
    }
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
