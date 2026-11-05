//! `mdots init --advanced`: scaffold a Lua-based configuration (host and
//! module templates, editor type hints).

use super::*;

/// Initialize arch-config with advanced Lua configuration
pub(super) fn run_advanced_lua(paths: &ConfigPaths) -> Result<()> {
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
