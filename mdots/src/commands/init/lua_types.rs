//! Write the Lua type definitions editors use to resolve the `mdots` global.

use super::*;

/// Create Lua type definitions for editor support (silences "undefined global 'mdots'" warnings)
pub(super) fn create_lua_type_definitions(config_dir: &Path) -> Result<()> {
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
