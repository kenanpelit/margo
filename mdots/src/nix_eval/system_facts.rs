use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SystemFacts {
    pub hostname: String,
    pub arch: String,
    pub distro: String,
    pub distro_name: String,
    pub distro_version: String,
    pub kernel_version: String,
    pub cpu_cores: u32,
    pub memory_total_mb: u64,
    pub hardware: HardwareFacts,
    pub network: NetworkFacts,
    pub boot: BootFacts,
    pub desktop: DesktopFacts,
    pub package: PackageFacts,
    pub service: ServiceFacts,
    pub power: PowerFacts,
    pub security: SecurityFacts,
    pub audio: AudioFacts,
    pub storage: StorageFacts,
    pub env: EnvFacts,
}

#[derive(Debug, Clone, Serialize)]
pub struct HardwareFacts {
    pub cpu_vendor: String,
    pub gpu_vendors: Vec<String>,
    pub has_nvidia: bool,
    pub has_amd_gpu: bool,
    pub has_intel_gpu: bool,
    pub is_laptop: bool,
    pub has_battery: bool,
    pub chassis_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkFacts {
    pub has_wifi: bool,
    pub has_ethernet: bool,
    pub has_bluetooth: bool,
    pub is_connected: bool,
    pub connection_type: String,
    pub interfaces: Vec<String>,
    pub active_interface: Option<String>,
    pub interface_types: HashMap<String, String>,
    pub has_ipv6: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootFacts {
    pub bootloader: String,
    pub is_uefi: bool,
    pub init_system: String,
    pub kernel_params: Vec<String>,
    pub efi_vars_supported: bool,
    pub boot_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesktopFacts {
    pub environment: String,
    pub display_server: String,
    pub is_wayland: bool,
    pub is_x11: bool,
    pub window_manager: String,
    pub session_type: String,
    pub has_display: bool,
    pub compositor: Option<String>,
    pub theme: Option<String>,
    pub icon_theme: Option<String>,
    pub screen_resolution: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageFacts {
    pub installed: Vec<String>,
    pub explicit: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceFacts {
    pub enabled: Vec<String>,
    pub active: Vec<String>,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PowerFacts {
    pub on_battery: bool,
    pub on_ac: bool,
    pub battery_percent: Option<u8>,
    pub battery_status: String,
    pub has_suspend: bool,
    pub has_hibernate: bool,
    pub cpu_governor: String,
    pub available_governors: Vec<String>,
    pub supports_turbo: bool,
    pub turbo_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityFacts {
    pub has_selinux: bool,
    pub selinux_enabled: bool,
    pub has_apparmor: bool,
    pub apparmor_enabled: bool,
    pub has_secureboot: bool,
    pub secureboot_enabled: bool,
    pub has_tpm: bool,
    pub tpm_version: Option<String>,
    pub firewall_active: bool,
    pub firewall_type: String,
    pub has_luks: bool,
    pub kernel_lockdown: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioFacts {
    pub server: String,
    pub has_pulseaudio: bool,
    pub has_pipewire: bool,
    pub has_jack: bool,
    pub has_alsa: bool,
    pub sound_cards: Vec<String>,
    pub default_sink: Option<String>,
    pub default_source: Option<String>,
    pub bluetooth_available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageFacts {
    pub has_ssd: bool,
    pub has_hdd: bool,
    pub has_nvme: bool,
    pub disks: Vec<DiskInfo>,
    pub has_swap: bool,
    pub swap_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskInfo {
    pub name: String,
    pub disk_type: String,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvFacts {
    pub home: String,
    pub user: String,
    pub shell: String,
    pub config_dir: String,
}

impl SystemFacts {
    pub fn collect() -> Self {
        Self {
            hostname: collect_hostname(),
            arch: collect_arch(),
            distro: collect_distro_id(),
            distro_name: collect_distro_name(),
            distro_version: collect_distro_version(),
            kernel_version: collect_kernel_version(),
            cpu_cores: collect_cpu_cores(),
            memory_total_mb: collect_memory_mb(),
            hardware: HardwareFacts::collect(),
            network: NetworkFacts::collect(),
            boot: BootFacts::collect(),
            desktop: DesktopFacts::collect(),
            package: PackageFacts::collect(),
            service: ServiceFacts::collect(),
            power: PowerFacts::collect(),
            security: SecurityFacts::collect(),
            audio: AudioFacts::collect(),
            storage: StorageFacts::collect(),
            env: EnvFacts::collect(),
        }
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}

fn collect_hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .or_else(|| {
            fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn collect_arch() -> String {
    Command::new("uname")
        .arg("-m")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn collect_distro_id() -> String {
    if let Ok(content) = fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(id) = line.strip_prefix("ID=") {
                return id.trim_matches('"').to_string();
            }
        }
    }
    "unknown".to_string()
}

fn collect_distro_name() -> String {
    if let Ok(content) = fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(name) = line.strip_prefix("NAME=") {
                return name.trim_matches('"').to_string();
            }
        }
    }
    "unknown".to_string()
}

fn collect_distro_version() -> String {
    if let Ok(content) = fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(ver) = line.strip_prefix("VERSION_ID=") {
                return ver.trim_matches('"').to_string();
            }
        }
    }
    "rolling".to_string()
}

fn collect_kernel_version() -> String {
    fs::read_to_string("/proc/version")
        .map(|s| s.split_whitespace().nth(2).unwrap_or("unknown").to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn collect_cpu_cores() -> u32 {
    fs::read_to_string("/proc/cpuinfo")
        .map(|s| s.lines().filter(|l| l.starts_with("processor")).count() as u32)
        .unwrap_or(1)
}

fn collect_memory_mb() -> u64 {
    fs::read_to_string("/proc/meminfo")
        .map(|s| {
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    if let Some(kb_str) = rest.split_whitespace().next() {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            return kb / 1024;
                        }
                    }
                }
            }
            0
        })
        .unwrap_or(0)
}

impl HardwareFacts {
    fn collect() -> Self {
        let gpu_vendors = detect_gpu_vendors();
        Self {
            cpu_vendor: detect_cpu_vendor(),
            has_nvidia: gpu_vendors.contains(&"nvidia".to_string()),
            has_amd_gpu: gpu_vendors.contains(&"amd".to_string()),
            has_intel_gpu: gpu_vendors.contains(&"intel".to_string()),
            gpu_vendors,
            is_laptop: detect_is_laptop(),
            has_battery: detect_has_battery(),
            chassis_type: detect_chassis_type(),
        }
    }
}

fn detect_cpu_vendor() -> String {
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if line.starts_with("vendor_id") {
                if line.contains("GenuineIntel") {
                    return "intel".to_string();
                } else if line.contains("AuthenticAMD") {
                    return "amd".to_string();
                }
            }
        }
    }
    "unknown".to_string()
}

fn detect_gpu_vendors() -> Vec<String> {
    let mut vendors = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
        for entry in entries.filter_map(|e| e.ok()) {
            let class_path = entry.path().join("class");
            let vendor_path = entry.path().join("vendor");
            if let Ok(class) = fs::read_to_string(&class_path) {
                if class.trim().starts_with("0x03") {
                    if let Ok(vendor) = fs::read_to_string(&vendor_path) {
                        let v = vendor.trim();
                        match v {
                            "0x10de" if !vendors.contains(&"nvidia".to_string()) => {
                                vendors.push("nvidia".to_string())
                            }
                            "0x1002" if !vendors.contains(&"amd".to_string()) => {
                                vendors.push("amd".to_string())
                            }
                            "0x8086" if !vendors.contains(&"intel".to_string()) => {
                                vendors.push("intel".to_string())
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    vendors
}

fn detect_is_laptop() -> bool {
    if let Ok(chassis) = fs::read_to_string("/sys/class/dmi/id/chassis_type") {
        let ct: u32 = chassis.trim().parse().unwrap_or(0);
        if matches!(ct, 8 | 9 | 10 | 11 | 14) {
            return true;
        }
        if matches!(
            ct,
            3 | 4 | 5 | 6 | 7 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 | 25
        ) {
            return false;
        }
    }
    detect_has_battery() || Path::new("/proc/acpi/button/lid").exists()
}

fn detect_has_battery() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.filter_map(|e| e.ok()) {
            let type_path = entry.path().join("type");
            if let Ok(supply_type) = fs::read_to_string(&type_path) {
                if supply_type.trim() == "Battery" {
                    return true;
                }
            }
        }
    }
    false
}

fn detect_chassis_type() -> String {
    if let Ok(chassis) = fs::read_to_string("/sys/class/dmi/id/chassis_type") {
        let ct: u32 = chassis.trim().parse().unwrap_or(0);
        return match ct {
            1 => "other",
            2 => "unknown",
            3 | 4 | 5 | 6 | 7 | 15 | 16 => "desktop",
            8 | 9 | 10 | 11 | 14 => "laptop",
            17..=25 => "server",
            30..=32 => "tablet",
            _ => "unknown",
        }
        .to_string();
    }
    "unknown".to_string()
}

impl NetworkFacts {
    fn collect() -> Self {
        let interfaces = list_interfaces();
        let mut interface_types = HashMap::new();
        for iface in &interfaces {
            interface_types.insert(iface.clone(), get_interface_type(iface));
        }
        Self {
            has_wifi: has_wifi(),
            has_ethernet: has_ethernet(),
            has_bluetooth: has_bluetooth(),
            is_connected: is_connected(),
            connection_type: get_connection_type(),
            active_interface: get_active_interface(),
            interface_types,
            interfaces,
            has_ipv6: has_ipv6(),
        }
    }
}

fn has_wifi() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.path().join("wireless").exists() {
                return true;
            }
        }
    }
    false
}

fn has_ethernet() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "lo" {
                continue;
            }
            if !entry.path().join("wireless").exists() {
                return true;
            }
        }
    }
    false
}

fn has_bluetooth() -> bool {
    Path::new("/sys/class/bluetooth").exists()
        && fs::read_dir("/sys/class/bluetooth")
            .map(|mut e| e.next().is_some())
            .unwrap_or(false)
}

fn is_connected() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "lo" {
                continue;
            }
            if let Ok(state) = fs::read_to_string(entry.path().join("operstate")) {
                if state.trim() == "up" {
                    return true;
                }
            }
        }
    }
    false
}

fn get_connection_type() -> String {
    if let Some(iface) = get_active_interface() {
        return get_interface_type(&iface);
    }
    "none".to_string()
}

fn list_interfaces() -> Vec<String> {
    let mut ifaces = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.filter_map(|e| e.ok()) {
            ifaces.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    ifaces
}

fn get_active_interface() -> Option<String> {
    if let Ok(output) = Command::new("ip")
        .args(["route", "show", "default"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(stripped) = stdout.split("dev ").nth(1) {
            if let Some(iface) = stripped.split_whitespace().next() {
                return Some(iface.to_string());
            }
        }
    }
    None
}

fn get_interface_type(name: &str) -> String {
    if name == "lo" {
        return "loopback".to_string();
    }
    let iface_path = Path::new("/sys/class/net").join(name);
    if iface_path.join("wireless").exists() {
        return "wifi".to_string();
    }
    if name.starts_with("wl") || name.starts_with("wlan") {
        return "wifi".to_string();
    }
    if name.starts_with("en") || name.starts_with("eth") {
        return "ethernet".to_string();
    }
    if name.starts_with("br") {
        return "bridge".to_string();
    }
    if name.starts_with("docker") || name.starts_with("veth") {
        return "virtual".to_string();
    }
    "unknown".to_string()
}

fn has_ipv6() -> bool {
    if let Ok(content) = fs::read_to_string("/proc/sys/net/ipv6/conf/all/disable_ipv6") {
        if content.trim() == "1" {
            return false;
        }
    }
    Command::new("ip")
        .args(["-6", "addr", "show", "scope", "global"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("inet6"))
        .unwrap_or(false)
}

impl BootFacts {
    fn collect() -> Self {
        Self {
            bootloader: detect_bootloader(),
            is_uefi: Path::new("/sys/firmware/efi").exists(),
            init_system: detect_init_system(),
            kernel_params: get_kernel_params(),
            efi_vars_supported: Path::new("/sys/firmware/efi/efivars").exists()
                && fs::read_dir("/sys/firmware/efi/efivars")
                    .map(|mut e| e.next().is_some())
                    .unwrap_or(false),
            boot_id: fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
        }
    }
}

fn detect_bootloader() -> String {
    if Path::new("/boot/grub").exists() || Path::new("/boot/grub2").exists() {
        return "grub".to_string();
    }
    if Path::new("/boot/loader/loader.conf").exists()
        || Path::new("/efi/loader/loader.conf").exists()
    {
        return "systemd-boot".to_string();
    }
    if Path::new("/boot/efi/EFI/refind").exists() {
        return "refind".to_string();
    }
    if Path::new("/etc/lilo.conf").exists() {
        return "lilo".to_string();
    }
    if Path::new("/boot/syslinux").exists() {
        return "syslinux".to_string();
    }
    if let Ok(output) = Command::new("efibootmgr").output() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
        if stdout.contains("grub") {
            return "grub".to_string();
        }
        if stdout.contains("systemd") {
            return "systemd-boot".to_string();
        }
    }
    "unknown".to_string()
}

fn detect_init_system() -> String {
    if Path::new("/run/systemd/system").exists() {
        return "systemd".to_string();
    }
    if Path::new("/run/openrc").exists() {
        return "openrc".to_string();
    }
    if Path::new("/run/runit").exists() {
        return "runit".to_string();
    }
    if let Ok(init) = fs::read_to_string("/proc/1/comm") {
        let init = init.trim();
        if init == "systemd" {
            return "systemd".to_string();
        }
        return init.to_string();
    }
    "unknown".to_string()
}

fn get_kernel_params() -> Vec<String> {
    fs::read_to_string("/proc/cmdline")
        .map(|s| s.split_whitespace().map(|p| p.to_string()).collect())
        .unwrap_or_default()
}

impl DesktopFacts {
    fn collect() -> Self {
        let is_wayland = env::var("WAYLAND_DISPLAY").is_ok()
            || env::var("XDG_SESSION_TYPE")
                .map(|s| s.to_lowercase() == "wayland")
                .unwrap_or(false);
        let is_x11 = env::var("DISPLAY").is_ok()
            || env::var("XDG_SESSION_TYPE")
                .map(|s| s.to_lowercase() == "x11")
                .unwrap_or(false);
        Self {
            environment: detect_desktop_environment(),
            display_server: if is_wayland {
                "wayland".to_string()
            } else if is_x11 {
                "x11".to_string()
            } else {
                "unknown".to_string()
            },
            is_wayland,
            is_x11,
            window_manager: detect_window_manager(),
            session_type: get_session_type(),
            has_display: env::var("DISPLAY").is_ok() || env::var("WAYLAND_DISPLAY").is_ok(),
            compositor: detect_compositor(),
            theme: get_desktop_theme(),
            icon_theme: get_icon_theme(),
            screen_resolution: get_screen_resolution(),
        }
    }
}

fn detect_desktop_environment() -> String {
    if let Ok(de) = env::var("XDG_CURRENT_DESKTOP") {
        return de.to_lowercase();
    }
    if let Ok(de) = env::var("DESKTOP_SESSION") {
        return de.to_lowercase();
    }
    "unknown".to_string()
}

fn is_process_running(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn detect_window_manager() -> String {
    let wms = [
        ("kwin_x11", "kwin"),
        ("kwin_wayland", "kwin"),
        ("mutter", "mutter"),
        ("Hyprland", "hyprland"),
        ("sway", "sway"),
        ("i3", "i3"),
        ("bspwm", "bspwm"),
        ("openbox", "openbox"),
        ("xfwm4", "xfwm4"),
        ("awesome", "awesome"),
        ("dwm", "dwm"),
        ("qtile", "qtile"),
        ("river", "river"),
        ("wayfire", "wayfire"),
        ("labwc", "labwc"),
    ];
    for (proc, name) in &wms {
        if is_process_running(proc) {
            return name.to_string();
        }
    }
    "unknown".to_string()
}

fn get_session_type() -> String {
    if let Ok(s) = env::var("XDG_SESSION_TYPE") {
        return s.to_lowercase();
    }
    if env::var("WAYLAND_DISPLAY").is_ok() {
        return "wayland".to_string();
    }
    if env::var("DISPLAY").is_ok() {
        return "x11".to_string();
    }
    "unknown".to_string()
}

fn detect_compositor() -> Option<String> {
    let compositors = ["picom", "compton", "Hyprland", "sway", "wayfire"];
    for c in &compositors {
        if is_process_running(c) {
            return Some(c.to_string());
        }
    }
    None
}

fn get_desktop_theme() -> Option<String> {
    if let Ok(home) = env::var("HOME") {
        let path = format!("{}/.config/gtk-3.0/settings.ini", home);
        if let Ok(content) = fs::read_to_string(&path) {
            for line in content.lines() {
                if let Some(theme) = line.strip_prefix("gtk-theme-name=") {
                    return Some(theme.trim().to_string());
                }
            }
        }
    }
    None
}

fn get_icon_theme() -> Option<String> {
    if let Ok(home) = env::var("HOME") {
        let path = format!("{}/.config/gtk-3.0/settings.ini", home);
        if let Ok(content) = fs::read_to_string(&path) {
            for line in content.lines() {
                if let Some(theme) = line.strip_prefix("gtk-icon-theme-name=") {
                    return Some(theme.trim().to_string());
                }
            }
        }
    }
    None
}

fn get_screen_resolution() -> Option<String> {
    if let Ok(output) = Command::new("xrandr").output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains('*') {
                if let Some(res) = line.split_whitespace().next() {
                    return Some(res.to_string());
                }
            }
        }
    }
    None
}

impl PackageFacts {
    fn collect() -> Self {
        Self {
            installed: list_pacman_packages(&["-Qq"]),
            explicit: list_pacman_packages(&["-Qqe"]),
        }
    }
}

fn list_pacman_packages(args: &[&str]) -> Vec<String> {
    Command::new("pacman")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

impl ServiceFacts {
    fn collect() -> Self {
        Self {
            enabled: list_systemctl(&[
                "list-units",
                "--type=service",
                "--state=enabled",
                "--no-pager",
            ]),
            active: list_systemctl(&[
                "list-units",
                "--type=service",
                "--state=active",
                "--no-pager",
            ]),
            failed: list_systemctl(&[
                "list-units",
                "--type=service",
                "--state=failed",
                "--no-pager",
            ]),
        }
    }
}

fn list_systemctl(args: &[&str]) -> Vec<String> {
    Command::new("systemctl")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.split_whitespace().next().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

impl PowerFacts {
    fn collect() -> Self {
        Self {
            on_battery: is_on_battery(),
            on_ac: is_on_ac(),
            battery_percent: get_battery_percent(),
            battery_status: get_battery_status(),
            has_suspend: has_suspend(),
            has_hibernate: has_hibernate(),
            cpu_governor: get_cpu_governor(),
            available_governors: get_available_governors(),
            supports_turbo: supports_turbo_boost(),
            turbo_enabled: is_turbo_enabled(),
        }
    }
}

fn is_on_battery() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(t) = fs::read_to_string(entry.path().join("type")) {
                if t.trim() == "Battery" {
                    if let Ok(s) = fs::read_to_string(entry.path().join("status")) {
                        if s.trim() == "Discharging" {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn is_on_ac() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(t) = fs::read_to_string(entry.path().join("type")) {
                if t.trim() == "Mains" {
                    if let Ok(o) = fs::read_to_string(entry.path().join("online")) {
                        if o.trim() == "1" {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn get_battery_percent() -> Option<u8> {
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(t) = fs::read_to_string(entry.path().join("type")) {
                if t.trim() == "Battery" {
                    if let Ok(c) = fs::read_to_string(entry.path().join("capacity")) {
                        if let Ok(p) = c.trim().parse::<u8>() {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }
    None
}

fn get_battery_status() -> String {
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(t) = fs::read_to_string(entry.path().join("type")) {
                if t.trim() == "Battery" {
                    if let Ok(s) = fs::read_to_string(entry.path().join("status")) {
                        return s.trim().to_lowercase();
                    }
                }
            }
        }
    }
    "unknown".to_string()
}

fn has_suspend() -> bool {
    fs::read_to_string("/sys/power/state")
        .map(|s| s.contains("mem"))
        .unwrap_or(false)
}

fn has_hibernate() -> bool {
    fs::read_to_string("/sys/power/state")
        .map(|s| s.contains("disk"))
        .unwrap_or(false)
}

fn get_cpu_governor() -> String {
    fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn get_available_governors() -> Vec<String> {
    fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors")
        .map(|s| s.split_whitespace().map(|g| g.to_string()).collect())
        .unwrap_or_default()
}

fn supports_turbo_boost() -> bool {
    Path::new("/sys/devices/system/cpu/intel_pstate/no_turbo").exists()
        || Path::new("/sys/devices/system/cpu/cpufreq/boost").exists()
}

fn is_turbo_enabled() -> bool {
    if let Ok(c) = fs::read_to_string("/sys/devices/system/cpu/intel_pstate/no_turbo") {
        return c.trim() == "0";
    }
    if let Ok(c) = fs::read_to_string("/sys/devices/system/cpu/cpufreq/boost") {
        return c.trim() == "1";
    }
    false
}

impl SecurityFacts {
    fn collect() -> Self {
        Self {
            has_selinux: Path::new("/sys/fs/selinux").exists()
                || Path::new("/etc/selinux/config").exists(),
            selinux_enabled: fs::read_to_string("/sys/fs/selinux/enforce")
                .map(|s| s.trim() == "1")
                .unwrap_or(false),
            has_apparmor: Path::new("/sys/kernel/security/apparmor").exists()
                || Path::new("/sys/module/apparmor").exists(),
            apparmor_enabled: fs::read_to_string("/sys/module/apparmor/parameters/enabled")
                .map(|s| s.trim() == "Y")
                .unwrap_or(false),
            has_secureboot: Path::new(
                "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c",
            )
            .exists(),
            secureboot_enabled: fs::read(
                "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c",
            )
            .ok()
            .and_then(|d| d.last().copied())
            .map(|b| b == 1)
            .unwrap_or(false),
            has_tpm: Path::new("/sys/class/tpm/tpm0").exists()
                || Path::new("/dev/tpm0").exists()
                || Path::new("/dev/tpmrm0").exists(),
            tpm_version: detect_tpm_version(),
            firewall_active: detect_firewall_active(),
            firewall_type: detect_firewall_type(),
            has_luks: Command::new("lsblk")
                .args(["-o", "TYPE", "-n"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("crypt"))
                .unwrap_or(false),
            kernel_lockdown: detect_kernel_lockdown(),
        }
    }
}

fn detect_tpm_version() -> Option<String> {
    if Path::new("/sys/class/tpm/tpm0/tpm_version_major").exists() {
        if let Ok(major) = fs::read_to_string("/sys/class/tpm/tpm0/tpm_version_major") {
            if major.trim() == "2" {
                return Some("2.0".to_string());
            }
        }
    }
    if Path::new("/sys/class/tpm/tpm0/device/caps").exists() {
        return Some("1.2".to_string());
    }
    None
}

fn detect_firewall_active() -> bool {
    Command::new("ufw")
        .args(["status"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("Status: active"))
        .unwrap_or(false)
}

fn detect_firewall_type() -> String {
    if Command::new("ufw")
        .args(["status"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "ufw".to_string();
    }
    if Command::new("firewall-cmd")
        .args(["--state"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "firewalld".to_string();
    }
    if Command::new("nft")
        .args(["list", "ruleset"])
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
    {
        return "nftables".to_string();
    }
    "none".to_string()
}

fn detect_kernel_lockdown() -> String {
    if let Ok(content) = fs::read_to_string("/sys/kernel/security/lockdown") {
        if content.contains("[none]") {
            return "none".to_string();
        }
        if content.contains("[integrity]") {
            return "integrity".to_string();
        }
        if content.contains("[confidentiality]") {
            return "confidentiality".to_string();
        }
    }
    "none".to_string()
}

impl AudioFacts {
    fn collect() -> Self {
        let has_pw = has_pipewire_audio();
        let has_pa = has_pulseaudio_audio();
        Self {
            server: detect_audio_server(&has_pw, &has_pa),
            has_pulseaudio: has_pa,
            has_pipewire: has_pw,
            has_jack: is_process_running("jackd") || is_process_running("jackdbus"),
            has_alsa: Path::new("/proc/asound").exists() || Path::new("/dev/snd").exists(),
            sound_cards: list_sound_cards(),
            default_sink: get_default_sink(),
            default_source: get_default_source(),
            bluetooth_available: Path::new("/sys/class/bluetooth").exists(),
        }
    }
}

fn has_pipewire_audio() -> bool {
    is_process_running("pipewire")
        || env::var("XDG_RUNTIME_DIR")
            .ok()
            .map(|d| Path::new(&format!("{}/pipewire-0", d)).exists())
            .unwrap_or(false)
}

fn has_pulseaudio_audio() -> bool {
    is_process_running("pulseaudio")
        || env::var("XDG_RUNTIME_DIR")
            .ok()
            .map(|d| Path::new(&format!("{}/pulse/native", d)).exists())
            .unwrap_or(false)
}

fn detect_audio_server(has_pw: &bool, has_pa: &bool) -> String {
    if *has_pw {
        return "pipewire".to_string();
    }
    if *has_pa {
        return "pulseaudio".to_string();
    }
    if Path::new("/proc/asound").exists() {
        return "alsa".to_string();
    }
    "none".to_string()
}

fn list_sound_cards() -> Vec<String> {
    let mut cards = Vec::new();
    if let Ok(content) = fs::read_to_string("/proc/asound/cards") {
        for line in content.lines() {
            if let Some(stripped) = line.strip_prefix(' ') {
                let parts: Vec<&str> = stripped.split('[').collect();
                if parts.len() >= 2 {
                    if let Some(name) = parts[1].split(']').next() {
                        cards.push(name.trim().to_string());
                    }
                }
            }
        }
    }
    cards
}

fn get_default_sink() -> Option<String> {
    if let Ok(output) = Command::new("pactl").args(["get-default-sink"]).output() {
        if output.status.success() {
            let sink = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !sink.is_empty() {
                return Some(sink);
            }
        }
    }
    None
}

fn get_default_source() -> Option<String> {
    None
}

impl StorageFacts {
    fn collect() -> Self {
        let disks = collect_disk_info();
        let has_ssd = disks
            .iter()
            .any(|d| d.disk_type == "ssd" || d.disk_type == "nvme");
        let has_hdd = disks.iter().any(|d| d.disk_type == "hdd");
        let has_nvme = disks.iter().any(|d| d.disk_type == "nvme");
        Self {
            has_ssd,
            has_hdd,
            has_nvme,
            disks,
            has_swap: has_swap_space(),
            swap_size_bytes: get_swap_size_bytes(),
        }
    }
}

fn collect_disk_info() -> Vec<DiskInfo> {
    let mut disks = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/block") {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("loop")
                || name.starts_with("ram")
                || name.starts_with("dm-")
                || name.starts_with("sr")
                || name.starts_with("zram")
            {
                continue;
            }
            let dt = if name.starts_with("nvme") {
                "nvme".to_string()
            } else {
                fs::read_to_string(format!("/sys/block/{}/queue/rotational", name))
                    .map(|s| match s.trim() {
                        "0" => "ssd",
                        "1" => "hdd",
                        _ => "unknown",
                    })
                    .unwrap_or("unknown")
                    .to_string()
            };
            let sz = fs::read_to_string(format!("/sys/block/{}/size", name))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|s| s * 512);
            disks.push(DiskInfo {
                name,
                disk_type: dt,
                size_bytes: sz,
            });
        }
    }
    disks
}

fn has_swap_space() -> bool {
    fs::read_to_string("/proc/swaps")
        .map(|s| s.lines().count() > 1)
        .unwrap_or(false)
}

fn get_swap_size_bytes() -> Option<u64> {
    if let Ok(content) = fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("SwapTotal:") {
                if let Some(kb_str) = rest.split_whitespace().next() {
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        return Some(kb * 1024);
                    }
                }
            }
        }
    }
    None
}

impl EnvFacts {
    fn collect() -> Self {
        Self {
            home: env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()),
            user: env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
            shell: env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
            config_dir: dirs::config_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/tmp/.config".to_string()),
        }
    }
}
