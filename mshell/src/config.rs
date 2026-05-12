use crate::app::Message;
use crate::i18n::UnitSystem;
use crate::services::upower::PeripheralDeviceKind;
use crate::utils::celsius_to_fahrenheit;
use hex_color::HexColor;
use iced::futures::StreamExt;
use iced::{Color, Subscription, futures::SinkExt, stream::channel, theme::palette};
use inotify::EventMask;
use inotify::Inotify;
use inotify::WatchMask;
use log::{debug, error, info, warn};
use regex::Regex;
use serde::{Deserialize, Deserializer, de::Visitor};
use serde_with::DisplayFromStr;
use serde_with::serde_as;
use std::path::PathBuf;
use std::time::Duration;
use std::{collections::HashMap, error::Error, ops::Deref, path::Path};
use tokio::time::sleep;

pub const DEFAULT_CONFIG_FILE_PATH: &str = "~/.config/margo/mshell.toml";

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub log_level: String,
    pub language: Option<String>,
    pub region: Option<String>,
    pub position: Position,
    pub layer: Layer,
    pub outputs: Outputs,
    pub modules: Modules,
    #[serde(rename = "CustomModule")]
    pub custom_modules: Vec<CustomModuleDef>,
    pub updates: Option<UpdatesModuleConfig>,
    pub workspaces: WorkspacesModuleConfig,
    pub window_title: WindowTitleConfig,
    pub system_info: SystemInfoModuleConfig,
    pub network_speed: NetworkSpeedModuleConfig,
    pub dns: DnsModuleConfig,
    pub ufw: UfwModuleConfig,
    pub power: PowerModuleConfig,
    pub podman: PodmanModuleConfig,
    pub notifications: NotificationsModuleConfig,
    pub tray: TrayModuleConfig,
    pub tempo: TempoModuleConfig,
    pub settings: SettingsModuleConfig,
    pub appearance: Appearance,
    pub media_player: MediaPlayerModuleConfig,
    pub keyboard_layout: KeyboardLayoutModuleConfig,
    pub animations: AnimationsConfig,
    pub enable_esc_key: bool,
    pub osd: OsdConfig,
    pub wallpaper: WallpaperConfig,
    pub matugen: MatugenConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            log_level: "warn".to_owned(),
            language: None,
            region: None,
            position: Position::default(),
            layer: Layer::default(),
            outputs: Outputs::default(),
            modules: Modules::default(),
            updates: None,
            workspaces: WorkspacesModuleConfig::default(),
            window_title: WindowTitleConfig::default(),
            system_info: SystemInfoModuleConfig::default(),
            network_speed: NetworkSpeedModuleConfig::default(),
            dns: DnsModuleConfig::default(),
            ufw: UfwModuleConfig::default(),
            power: PowerModuleConfig::default(),
            podman: PodmanModuleConfig::default(),
            notifications: NotificationsModuleConfig::default(),
            tray: TrayModuleConfig::default(),
            tempo: TempoModuleConfig::default(),
            settings: SettingsModuleConfig::default(),
            appearance: Appearance::default(),
            media_player: MediaPlayerModuleConfig::default(),
            keyboard_layout: KeyboardLayoutModuleConfig::default(),
            animations: AnimationsConfig::default(),
            custom_modules: vec![],
            enable_esc_key: false,
            osd: OsdConfig::default(),
            wallpaper: WallpaperConfig::default(),
            matugen: MatugenConfig::default(),
        }
    }
}

/// Matugen otomatik teması — `mshell matugen` subcommand'inin
/// hangi olaylarda otomatik tetikleneceğini kontrol eder.
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct MatugenConfig {
    /// Aktif output'un wallpaper'ı değiştiğinde `mshell matugen`'i
    /// background tokio task'ında otomatik çalıştır. `false` (default)
    /// = sadece manuel tetikleme (super+ctrl+t veya `mshell matugen`).
    pub auto_on_wallpaper_change: bool,
}

impl Default for MatugenConfig {
    fn default() -> Self {
        Self {
            auto_on_wallpaper_change: false,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct WallpaperConfig {
    /// Enable the wallpaper renderer. When false, mshell does not
    /// create any Background-layer surface and the user can run
    /// `swaybg`/`swww` etc. as before.
    pub enabled: bool,
    /// How to scale the image to the output. "Cover" fills the
    /// entire output (may crop), "Contain" letterboxes, "Fill"
    /// stretches, "None" shows at 1:1.
    pub fit: WallpaperFit,
    /// Background colour painted *behind* the image (visible when
    /// the image is None, Contain-letterboxed, or partially
    /// transparent). Hex string.
    pub fallback_color: String,
    /// Optional shuffle / rotate behaviour. When enabled, mshell
    /// picks wallpapers from a directory and **bypasses** the
    /// `tags` map below as well as margo's state.json paths.
    pub shuffle: WallpaperShuffleConfig,
    /// Per-tag wallpaper assignments owned by mshell. Keyed by tag
    /// id ("1".."9"), value is an image path with `~` expansion.
    /// When this map is non-empty, mshell uses the active tag's
    /// entry instead of whatever margo wrote to `state.json` —
    /// `tagrule = id:N,wallpaper:…` lines in margo's config.conf
    /// become unnecessary. Precedence:
    ///
    ///   1. `[wallpaper.shuffle].enabled` overrides everything.
    ///   2. `[wallpaper.tags]` (this map) for the active tag.
    ///   3. State.json's path (backward compat).
    pub tags: HashMap<String, String>,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fit: WallpaperFit::default(),
            fallback_color: "#1e1e2e".to_owned(),
            shuffle: WallpaperShuffleConfig::default(),
            tags: HashMap::new(),
        }
    }
}

/// Wallpaper shuffle / slideshow configuration.
///
/// When `enabled`, the path margo writes to `state.json` (driven by
/// `tagrule = id:N,wallpaper:…`) is ignored; mshell scans `directory`,
/// shuffles the list once at startup, and assigns one image to each
/// output (or all outputs share one if `per_output = false`).
///
/// If `interval_secs > 0`, mshell rotates to the next image at that
/// cadence on top of the initial pick.
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct WallpaperShuffleConfig {
    pub enabled: bool,
    /// Source directory. `~` is expanded. Reads `.jpg`, `.jpeg`,
    /// `.png`, `.webp`, case-insensitive.
    pub directory: String,
    /// Each output gets its own random pick when `true`; when
    /// `false`, mshell picks one image and shares it across every
    /// output (single coherent backdrop on multi-monitor setups).
    pub per_output: bool,
    /// Rotate cadence in seconds. `0` = pick once at process start
    /// and never rotate.
    pub interval_secs: u64,
    /// `Random` reshuffles independently each pick. `Sequential`
    /// walks the directory listing in order, wrapping around.
    pub mode: WallpaperShuffleMode,
}

impl Default for WallpaperShuffleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            directory: "~/Pictures/wallpapers".to_owned(),
            per_output: true,
            interval_secs: 0,
            mode: WallpaperShuffleMode::default(),
        }
    }
}

#[derive(Deserialize, Default, Copy, Clone, Debug, PartialEq, Eq)]
pub enum WallpaperShuffleMode {
    #[default]
    Random,
    Sequential,
}

#[derive(Deserialize, Default, Copy, Clone, Debug, PartialEq, Eq)]
pub enum WallpaperFit {
    #[default]
    Cover,
    Contain,
    Fill,
    None,
}

impl From<WallpaperFit> for iced::ContentFit {
    fn from(fit: WallpaperFit) -> Self {
        match fit {
            WallpaperFit::Cover => iced::ContentFit::Cover,
            WallpaperFit::Contain => iced::ContentFit::Contain,
            WallpaperFit::Fill => iced::ContentFit::Fill,
            WallpaperFit::None => iced::ContentFit::None,
        }
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct AnimationsConfig {
    pub enabled: bool,
}

impl Config {
    fn validate(&mut self) {
        if let Some(ref mut updates) = self.updates {
            updates.validate();
        }
        self.system_info.validate();
        self.network_speed.validate();
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct UpdatesModuleConfig {
    pub check_cmd: String,
    pub update_cmd: String,
    #[serde(default = "UpdatesModuleConfig::default_interval")]
    pub interval: u64,
}

impl UpdatesModuleConfig {
    const fn default_interval() -> u64 {
        3600
    }

    fn validate(&mut self) {
        if self.interval == 0 {
            warn!("UpdatesModuleConfig.interval is 0, setting to 1");
            self.interval = 1;
        }
    }
}

#[derive(Deserialize, Copy, Clone, Default, PartialEq, Eq, Debug)]
pub enum WorkspaceVisibilityMode {
    #[default]
    All,
    MonitorSpecific,
    MonitorSpecificExclusive,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct WorkspacesModuleConfig {
    pub visibility_mode: WorkspaceVisibilityMode,
    pub group_by_monitor: bool,
    pub enable_workspace_filling: bool,
    pub disable_special_workspaces: bool,
    pub max_workspaces: Option<u32>,
    pub workspace_names: Vec<String>,
    pub enable_virtual_desktops: bool,
    pub invert_scroll_direction: Option<InvertScrollDirection>,
    /// Workspace etiketleri için özel font ailesi. None ise bar'ın
    /// global fontu kullanılır (Maple Mono NF vb.). Örnek: "JetBrains Mono".
    #[serde(default)]
    pub font_name: Option<String>,
    /// Workspace etiketleri için font boyutu override. None ise
    /// `bar_font_size` kullanılır (font birliği). Bu alanı set edersen
    /// workspace numaraları daha büyük/küçük gözükür.
    #[serde(default)]
    pub font_size: Option<f32>,
    /// Workspace'lerde 1'den fazla pencere varsa numaranın yanına
    /// küçük üst-simge sayı (¹²³…⁹) yazar. Eski davranışı korumak için
    /// varsayılan false.
    #[serde(default)]
    pub show_window_count: bool,
    /// Per-tag renk override — `[appearance].workspace_colors`'tan daha
    /// keşfedilebilir basit syntax. Key tag numarası ("1".."9"), value
    /// hex renk ("#cba6f7"). Buradaki herhangi bir tag varsa
    /// `[appearance].workspace_colors`'taki ilgili index override edilir;
    /// boş bırakılırsa `appearance`'ınki kullanılır.
    #[serde(default)]
    pub colors: std::collections::HashMap<String, String>,
}

#[derive(Deserialize, Copy, Clone, Default, PartialEq, Eq, Debug)]
pub enum InvertScrollDirection {
    #[default]
    All,
    Mouse,
    Trackpad,
}

#[derive(Deserialize, Copy, Clone, Default, PartialEq, Eq, Debug)]
pub enum WindowTitleMode {
    #[default]
    Title,
    Class,
    InitialTitle,
    InitialClass,
}

#[derive(Deserialize, Copy, Clone, Debug)]
#[serde(default)]
pub struct WindowTitleConfig {
    pub mode: WindowTitleMode,
    pub truncate_title_after_length: u32,
}

impl Default for WindowTitleConfig {
    fn default() -> Self {
        Self {
            mode: Default::default(),
            truncate_title_after_length: 150,
        }
    }
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct KeyboardLayoutModuleConfig {
    pub labels: HashMap<String, String>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SystemInfoCpu {
    pub warn_threshold: u32,
    pub alert_threshold: u32,

    pub format: CpuFormat,
}

fn validate_thresholds<T: PartialOrd + Copy + std::fmt::Display>(
    warn: &mut T,
    alert: &mut T,
    name: &str,
) {
    if *warn >= *alert {
        warn!(
            "{name} warn_threshold ({warn}) >= alert_threshold ({alert}), setting both to {alert}"
        );
        *warn = *alert;
    }
}

impl SystemInfoCpu {
    fn validate(&mut self) {
        validate_thresholds(&mut self.warn_threshold, &mut self.alert_threshold, "CPU");
    }
}

impl Default for SystemInfoCpu {
    fn default() -> Self {
        Self {
            warn_threshold: 60,
            alert_threshold: 80,
            format: CpuFormat::Percentage,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SystemInfoMemory {
    pub warn_threshold: u32,
    pub alert_threshold: u32,
    pub format: MemoryFormat,
}

impl SystemInfoMemory {
    fn validate(&mut self) {
        validate_thresholds(
            &mut self.warn_threshold,
            &mut self.alert_threshold,
            "Memory",
        );
    }
}

impl Default for SystemInfoMemory {
    fn default() -> Self {
        Self {
            warn_threshold: 70,
            alert_threshold: 85,
            format: MemoryFormat::Percentage,
        }
    }
}

const DEFAULT_TEMP_WARN_CELSIUS: i32 = 60;
const DEFAULT_TEMP_ALERT_CELSIUS: i32 = 80;

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SystemInfoTemperature {
    warn_threshold: Option<i32>,
    alert_threshold: Option<i32>,
    pub sensor: String,
}

impl SystemInfoTemperature {
    pub fn warn_threshold(&self) -> i32 {
        self.warn_threshold
            .unwrap_or_else(|| match crate::i18n::unit_system() {
                UnitSystem::Metric => DEFAULT_TEMP_WARN_CELSIUS,
                UnitSystem::Imperial => celsius_to_fahrenheit(DEFAULT_TEMP_WARN_CELSIUS),
            })
    }

    pub fn alert_threshold(&self) -> i32 {
        self.alert_threshold
            .unwrap_or_else(|| match crate::i18n::unit_system() {
                UnitSystem::Metric => DEFAULT_TEMP_ALERT_CELSIUS,
                UnitSystem::Imperial => celsius_to_fahrenheit(DEFAULT_TEMP_ALERT_CELSIUS),
            })
    }

    fn validate(&mut self) {
        if let (Some(warn), Some(alert)) = (&mut self.warn_threshold, &mut self.alert_threshold) {
            validate_thresholds(warn, alert, "Temperature");
        }
    }
}

impl Default for SystemInfoTemperature {
    fn default() -> Self {
        Self {
            warn_threshold: None,
            alert_threshold: None,
            sensor: "acpitz temp1".to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub enum DiskFormat {
    #[default]
    Percentage,
    Fraction,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub enum MemoryFormat {
    #[default]
    Percentage,
    Fraction,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub enum CpuFormat {
    #[default]
    Percentage,
    Frequency,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SystemInfoDisk {
    pub warn_threshold: u32,
    pub alert_threshold: u32,
    pub format: DiskFormat,
    pub mounts: Option<Vec<String>>,
}

impl SystemInfoDisk {
    fn validate(&mut self) {
        validate_thresholds(&mut self.warn_threshold, &mut self.alert_threshold, "Disk");
    }
}

impl Default for SystemInfoDisk {
    fn default() -> Self {
        Self {
            warn_threshold: 80,
            alert_threshold: 90,
            format: DiskFormat::Percentage,
            mounts: None,
        }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SystemInfoDiskIndicatorConfig {
    #[serde(rename = "Disk")]
    pub path: String,
    #[serde(rename = "Name")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub enum SystemInfoIndicator {
    Cpu,
    Memory,
    MemorySwap,
    Temperature,
    #[serde(untagged)]
    Disk(SystemInfoDiskIndicatorConfig),
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SystemInfoModuleConfig {
    pub indicators: Vec<SystemInfoIndicator>,
    #[serde(default = "SystemInfoModuleConfig::default_interval")]
    pub interval: u64,
    pub cpu: SystemInfoCpu,
    pub memory: SystemInfoMemory,
    pub temperature: SystemInfoTemperature,
    pub disk: SystemInfoDisk,
}

impl SystemInfoModuleConfig {
    const fn default_interval() -> u64 {
        5
    }

    fn validate(&mut self) {
        if self.interval == 0 {
            warn!("SystemInfoModuleConfig.interval is 0, setting to 1");
            self.interval = 1;
        }
        self.cpu.validate();
        self.memory.validate();
        self.temperature.validate();
        self.disk.validate();
    }
}

impl Default for SystemInfoModuleConfig {
    fn default() -> Self {
        Self {
            indicators: vec![
                SystemInfoIndicator::Cpu,
                SystemInfoIndicator::Memory,
                SystemInfoIndicator::Temperature,
            ],
            interval: Self::default_interval(),
            cpu: SystemInfoCpu::default(),
            memory: SystemInfoMemory::default(),
            temperature: SystemInfoTemperature::default(),
            disk: SystemInfoDisk::default(),
        }
    }
}

// ─── NetworkSpeed module ─────────────────────────────────────────────────────
// system_info'dan ayırılmış indirme/yükleme hızı modülü. Kendi
// interval'ı, kendi eşikleri var. Aktif/Kapalı durumu modules.left/right
// listesinde NetworkSpeed olarak kullanılarak belirlenir.

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum NetworkSpeedIndicator {
    Download,
    Upload,
    IpAddress,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NetworkSpeedUnit {
    /// Otomatik: <1000 KB/s → KB/s, ≥1000 → MB/s
    #[default]
    Auto,
    Kbps,
    Mbps,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct NetworkSpeedModuleConfig {
    pub indicators: Vec<NetworkSpeedIndicator>,
    #[serde(default = "NetworkSpeedModuleConfig::default_interval")]
    pub interval: u64,
    pub unit: NetworkSpeedUnit,
    /// İndirme hızının KB/s cinsinden warn eşiği.
    pub download_warn_kbps: u32,
    /// İndirme hızının KB/s cinsinden alert eşiği.
    pub download_alert_kbps: u32,
    pub upload_warn_kbps: u32,
    pub upload_alert_kbps: u32,
}

impl NetworkSpeedModuleConfig {
    const fn default_interval() -> u64 {
        2
    }

    fn validate(&mut self) {
        if self.interval == 0 {
            warn!("NetworkSpeedModuleConfig.interval is 0, setting to 1");
            self.interval = 1;
        }
    }
}

impl Default for NetworkSpeedModuleConfig {
    fn default() -> Self {
        Self {
            indicators: vec![
                NetworkSpeedIndicator::Download,
                NetworkSpeedIndicator::Upload,
            ],
            interval: Self::default_interval(),
            unit: NetworkSpeedUnit::default(),
            download_warn_kbps: 5_000,    // 5 MB/s
            download_alert_kbps: 20_000,  // 20 MB/s
            upload_warn_kbps: 2_000,      // 2 MB/s
            upload_alert_kbps: 10_000,    // 10 MB/s
        }
    }
}

// ─── DNS / VPN module ────────────────────────────────────────────────────────
// Mullvad + Blocky + nmcli DNS preset switcher. ndns noctalia plugin'inden
// tam Rust port'u: state.sh + apply.sh script'leri tokio::process ile
// inline çalıştırılıyor; harici dosya yok.

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct DnsProviderEntry {
    /// "google", "cloudflare" gibi tanımlayıcı (dahili karşılaştırma için).
    pub id: String,
    /// UI'da gösterilecek metin: "Google", "Cloudflare" vs.
    pub label: String,
    /// Boşlukla ayrılmış IPv4 listesi:  "8.8.8.8 8.8.4.4".
    pub ip: String,
}

impl Default for DnsProviderEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            ip: String::new(),
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct DnsModuleConfig {
    /// `osc-mullvad` script'ine giden komut (binary adı veya tam yol).
    pub osc_command: String,
    /// State polling aralığı (saniye). Min 5.
    pub watchdog_secs: u64,
    /// DNS preset listesi. Boş bırakılırsa varsayılan 5 (Google, Cloudflare,
    /// OpenDNS, AdGuard, Quad9) kullanılır.
    pub providers: Vec<DnsProviderEntry>,
}

impl DnsModuleConfig {
    fn default_providers() -> Vec<DnsProviderEntry> {
        vec![
            DnsProviderEntry {
                id: "google".into(),
                label: "Google".into(),
                ip: "8.8.8.8 8.8.4.4".into(),
            },
            DnsProviderEntry {
                id: "cloudflare".into(),
                label: "Cloudflare".into(),
                ip: "1.1.1.1 1.0.0.1".into(),
            },
            DnsProviderEntry {
                id: "opendns".into(),
                label: "OpenDNS".into(),
                ip: "208.67.222.222 208.67.220.220".into(),
            },
            DnsProviderEntry {
                id: "adguard".into(),
                label: "AdGuard".into(),
                ip: "94.140.14.14 94.140.15.15".into(),
            },
            DnsProviderEntry {
                id: "quad9".into(),
                label: "Quad9".into(),
                ip: "9.9.9.9 149.112.112.112".into(),
            },
        ]
    }
}

impl Default for DnsModuleConfig {
    fn default() -> Self {
        Self {
            osc_command: "osc-mullvad".into(),
            watchdog_secs: 30,
            providers: Self::default_providers(),
        }
    }
}

// ─── UFW firewall module ─────────────────────────────────────────────────────

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct UfwModuleConfig {
    pub watchdog_secs: u64,
    /// Salt-okunur durum sorguları başarısız olursa `sudo -n ufw status`
    /// dene. Mutating aksiyonlar her zaman sudo/pkexec gerektirir.
    pub allow_privileged_reads: bool,
}

impl Default for UfwModuleConfig {
    fn default() -> Self {
        Self {
            watchdog_secs: 30,
            allow_privileged_reads: true,
        }
    }
}

// ─── Power (laptop power profile / battery / session) ───────────────────────

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PowerModuleConfig {
    pub watchdog_secs: u64,
}

impl Default for PowerModuleConfig {
    fn default() -> Self {
        Self { watchdog_secs: 15 }
    }
}

// ─── Podman (container manager) ─────────────────────────────────────────────

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PodmanModuleConfig {
    pub watchdog_secs: u64,
}

impl Default for PodmanModuleConfig {
    fn default() -> Self {
        Self { watchdog_secs: 60 }
    }
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToastPosition {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct NotificationsModuleConfig {
    pub format: String,
    pub show_timestamps: bool,
    pub show_bodies: bool,
    pub grouped: bool,
    pub toast: bool,
    pub toast_position: ToastPosition,
    pub toast_timeout: u64,
    pub toast_limit: usize,
    pub toast_max_height: u32,
    pub blocklist: Vec<RegexCfg>,
    // ─── Toast geometry (mako-tarzı tam ayar) ─────────────────────────
    /// Toast kartı genişliği (piksel). 320–420 arası iyi denge.
    #[serde(default = "default_toast_width")]
    pub toast_width: u32,
    /// Kart iç padding'i (piksel).
    #[serde(default = "default_toast_padding")]
    pub toast_padding: u16,
    /// Kart köşe yuvarlama yarıçapı (piksel).
    #[serde(default = "default_toast_radius")]
    pub toast_radius: f32,
    /// Uygulama / bildirim ikonu boyutu (piksel).
    #[serde(default = "default_toast_icon_size")]
    pub toast_icon_size: f32,
    /// Summary (başlık) font boyutu.
    #[serde(default = "default_toast_summary_font_size")]
    pub toast_summary_font_size: f32,
    /// Body (içerik) font boyutu.
    #[serde(default = "default_toast_body_font_size")]
    pub toast_body_font_size: f32,
    /// Toast kartı opaklığı (0.0..=1.0). 0.92 mat-cam hissi verir.
    #[serde(default = "default_toast_opacity")]
    pub toast_opacity: f32,
    /// Toast metni için özel font ailesi. None ise bar'ın global fontu
    /// (`[appearance].font_name`). Mako'nun `font = "Noto Sans Regular 14"`
    /// satırının karşılığı (boyut alanı ayrı: toast_summary_font_size
    /// + toast_body_font_size).
    #[serde(default)]
    pub font_name: Option<String>,
    // ─── Per-urgency timeout override ─────────────────────────────────
    /// Critical bildirim timeout'u (ms). 0 = otomatik kaybolmaz.
    /// None ise `toast_timeout` kullanılır.
    #[serde(default)]
    pub critical_timeout: Option<u64>,
    /// Low urgency için timeout (ms). None ise `toast_timeout`.
    #[serde(default)]
    pub low_timeout: Option<u64>,
    /// Yeni bildirim geldiğinde çalıştırılacak shell komutu.
    /// Örn. "paplay ~/.sounds/message.oga" ya da
    /// "sh -c 'mpv --no-terminal ~/sounds/bell.wav'".
    #[serde(default)]
    pub on_notify_command: Option<String>,
    /// Per-app override'lar (mako'nun `[app-name=...]` blokları gibi).
    /// Key = uygulama adı (D-Bus app_name eşleşmesi), value = override.
    #[serde(default)]
    pub apps: std::collections::HashMap<String, AppNotificationOverride>,
    // ─── Görsel iyileştirmeler ──────────────────────────────────────────
    /// Kartın sol kenarındaki 4px renkli urgency bar'ını çiz.
    /// Varsayılan false — yalnızca Critical zaten danger renkli border'a sahip.
    #[serde(default)]
    pub show_urgency_bar: bool,
    /// Summary (başlık) font ağırlığı bold olsun.
    #[serde(default = "default_true")]
    pub summary_bold: bool,
    /// App name başlığı primary accent rengiyle göster.
    #[serde(default = "default_true")]
    pub accent_app_name: bool,
    /// Bar ikonu yanında okunmamış bildirim sayı badge'i.
    #[serde(default = "default_true")]
    pub show_count_badge: bool,
    /// Bildirim kartının altında D-Bus action butonlarını render et.
    #[serde(default = "default_true")]
    pub show_actions: bool,
    /// Urgency'ye göre özel renkler. None ise theme palette
    /// (text / warning / danger) kullanılır.
    #[serde(default)]
    pub urgency_colors: NotificationUrgencyColors,
    /// History menüsünde "Today / Yesterday / Older" başlıklarıyla
    /// gün-bazlı bölümleme yap. `grouped`'ı override eder (app-bazlı
    /// gruplama yerine tarih-bazlı section'lar gözükür).
    #[serde(default)]
    pub group_by_date: bool,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct NotificationUrgencyColors {
    pub low: Option<String>,
    pub normal: Option<String>,
    pub critical: Option<String>,
}

/// Mako'nun `[app-name=Spotify]` bloğunun karşılığı. Bir uygulamaya
/// özel görsel + zamanlama özelliklerini override eder.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct AppNotificationOverride {
    /// Bu uygulamanın bildirimleri için özel border rengi (hex).
    pub border_color: Option<String>,
    /// Bu uygulamanın timeout'u (ms). 0 = otomatik kaybolmaz.
    pub timeout: Option<u64>,
}

fn default_true() -> bool {
    true
}

fn default_toast_width() -> u32 {
    380
}
fn default_toast_padding() -> u16 {
    16
}
fn default_toast_radius() -> f32 {
    14.0
}
fn default_toast_icon_size() -> f32 {
    48.0
}
fn default_toast_summary_font_size() -> f32 {
    13.0
}
fn default_toast_body_font_size() -> f32 {
    11.0
}
fn default_toast_opacity() -> f32 {
    0.92
}
impl Default for NotificationsModuleConfig {
    fn default() -> Self {
        Self {
            format: "%H:%M".to_string(),
            show_timestamps: true,
            show_bodies: true,
            grouped: false,
            toast: true,
            toast_position: ToastPosition::default(),
            toast_timeout: 5000,
            toast_limit: 5,
            toast_max_height: 150,
            blocklist: vec![],
            show_urgency_bar: false,
            summary_bold: true,
            accent_app_name: true,
            show_count_badge: true,
            show_actions: true,
            urgency_colors: NotificationUrgencyColors::default(),
            group_by_date: true,
            toast_width: default_toast_width(),
            toast_padding: default_toast_padding(),
            toast_radius: default_toast_radius(),
            toast_icon_size: default_toast_icon_size(),
            toast_summary_font_size: default_toast_summary_font_size(),
            toast_body_font_size: default_toast_body_font_size(),
            toast_opacity: default_toast_opacity(),
            font_name: None,
            critical_timeout: None,
            low_timeout: None,
            on_notify_command: None,
            apps: std::collections::HashMap::new(),
        }
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct TrayModuleConfig {
    pub blocklist: Vec<RegexCfg>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct TempoModuleConfig {
    pub clock_format: String,
    /// Optional smaller second line under the clock (e.g. the date).
    /// When set, the primary clock renders bold and this string is
    /// drawn beneath it in a muted micro size — the "rich" composite
    /// look. Leave empty/None for the classic single-line bar item.
    #[serde(default)]
    pub secondary_format: Option<String>,
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default)]
    pub timezones: Vec<String>,
    #[serde(default)]
    pub weather_location: Option<WeatherLocation>,
    pub weather_indicator: WeatherIndicator,
}

#[derive(Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub enum WeatherIndicator {
    #[default]
    IconAndTemperature,
    Icon,
    None,
}

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
pub enum WeatherLocation {
    #[default]
    Current,
    City(String),
    Coordinates(f32, f32),
}

impl std::hash::Hash for WeatherLocation {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            WeatherLocation::Current => {}
            WeatherLocation::City(city) => city.hash(state),
            WeatherLocation::Coordinates(lat, lon) => {
                lat.to_bits().hash(state);
                lon.to_bits().hash(state);
            }
        }
    }
}

impl Default for TempoModuleConfig {
    fn default() -> Self {
        Self {
            clock_format: "%a %d %b %R".to_string(),
            secondary_format: None,
            formats: vec![],
            timezones: vec![],
            weather_location: None,
            weather_indicator: WeatherIndicator::IconAndTemperature,
        }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum SettingsIndicator {
    IdleInhibitor,
    PowerProfile,
    Audio,
    Microphone,
    Network,
    Vpn,
    Bluetooth,
    Battery,
    PeripheralBattery,
    Brightness,
}

#[derive(Deserialize, Copy, Clone, Default, PartialEq, Eq, Debug)]
pub enum SettingsFormat {
    Icon,
    #[serde(alias = "Value")]
    Percentage,
    #[default]
    #[serde(alias = "IconAndValue")]
    IconAndPercentage,
    Time,
    IconAndTime,
}

#[derive(Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub enum PeripheralIndicators {
    #[default]
    All,
    Specific(Vec<PeripheralDeviceKind>),
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SettingsModuleConfig {
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub lock_cmd: Option<String>,
    pub shutdown_cmd: String,
    pub suspend_cmd: String,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub hibernate_cmd: Option<String>,
    pub reboot_cmd: String,
    pub logout_cmd: String,
    pub battery_format: SettingsFormat,
    pub battery_hide_when_full: bool,
    pub peripheral_indicators: PeripheralIndicators,
    pub peripheral_battery_format: SettingsFormat,
    pub peripheral_expanded_by_default: bool,
    pub audio_indicator_format: SettingsFormat,
    pub microphone_indicator_format: SettingsFormat,
    pub network_indicator_format: SettingsFormat,
    pub bluetooth_indicator_format: SettingsFormat,
    pub brightness_indicator_format: SettingsFormat,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub audio_sinks_more_cmd: Option<String>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub audio_sources_more_cmd: Option<String>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub wifi_more_cmd: Option<String>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub vpn_more_cmd: Option<String>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub bluetooth_more_cmd: Option<String>,
    pub remove_airplane_btn: bool,
    pub remove_idle_btn: bool,
    pub indicators: Vec<SettingsIndicator>,
    #[serde(rename = "CustomButton")]
    pub custom_buttons: Vec<SettingsCustomButton>,
    /// Menüdeki bölüm başlıkları + custom button isimleri için özel font ailesi.
    /// `None` ⇒ varsayılan tema fontu.
    #[serde(default)]
    pub font_name: Option<String>,
    /// "Bağlantı / Sistem / Özel" bölüm başlıklarını göster (default true).
    #[serde(default = "default_settings_section_headers")]
    pub section_headers: bool,
    /// Bölüm başlığı font boyutu (default 11 — küçük, muted-style).
    #[serde(default = "default_settings_header_font_size")]
    pub header_font_size: f32,
}

fn default_settings_section_headers() -> bool {
    true
}

fn default_settings_header_font_size() -> f32 {
    11.0
}

impl Default for SettingsModuleConfig {
    fn default() -> Self {
        Self {
            lock_cmd: Default::default(),
            shutdown_cmd: "shutdown now".to_string(),
            suspend_cmd: "systemctl suspend".to_string(),
            hibernate_cmd: Default::default(),
            reboot_cmd: "systemctl reboot".to_string(),
            logout_cmd: "loginctl kill-user $(whoami)".to_string(),
            battery_format: SettingsFormat::IconAndPercentage,
            battery_hide_when_full: false,
            peripheral_indicators: Default::default(),
            peripheral_battery_format: SettingsFormat::Icon,
            peripheral_expanded_by_default: false,
            audio_indicator_format: SettingsFormat::Icon,
            microphone_indicator_format: SettingsFormat::Icon,
            network_indicator_format: SettingsFormat::Icon,
            bluetooth_indicator_format: SettingsFormat::Icon,
            brightness_indicator_format: SettingsFormat::Icon,
            audio_sinks_more_cmd: Default::default(),
            audio_sources_more_cmd: Default::default(),
            wifi_more_cmd: Default::default(),
            vpn_more_cmd: Default::default(),
            bluetooth_more_cmd: Default::default(),
            remove_airplane_btn: Default::default(),
            remove_idle_btn: Default::default(),
            indicators: vec![
                SettingsIndicator::IdleInhibitor,
                SettingsIndicator::PowerProfile,
                SettingsIndicator::Audio,
                SettingsIndicator::Microphone,
                SettingsIndicator::Bluetooth,
                SettingsIndicator::Network,
                SettingsIndicator::Vpn,
                SettingsIndicator::Battery,
                SettingsIndicator::Brightness,
            ],
            custom_buttons: Default::default(),
            font_name: None,
            section_headers: default_settings_section_headers(),
            header_font_size: default_settings_header_font_size(),
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct SettingsCustomButton {
    pub name: String,
    pub icon: String,
    pub command: String,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub status_command: Option<String>,
    pub tooltip: Option<String>,
}

#[derive(Deserialize, Copy, Clone, Default, PartialEq, Eq, Debug)]
pub enum MediaPlayerFormat {
    Icon,
    #[default]
    IconAndTitle,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct MediaPlayerModuleConfig {
    pub max_title_length: u32,
    pub indicator_format: MediaPlayerFormat,
}

impl Default for MediaPlayerModuleConfig {
    fn default() -> Self {
        MediaPlayerModuleConfig {
            max_title_length: 100,
            indicator_format: MediaPlayerFormat::default(),
        }
    }
}

fn hex_to_color(hex: HexColor) -> Color {
    Color::from_rgb8(hex.r, hex.g, hex.b)
}

fn hex_to_pair(hex: HexColor, text: Option<HexColor>, text_fallback: Color) -> palette::Pair {
    palette::Pair::new(
        hex_to_color(hex),
        text.map(hex_to_color).unwrap_or(text_fallback),
    )
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(untagged)]
pub enum AppearanceColor {
    Simple(HexColor),
    Complete {
        base: HexColor,
        strong: Option<HexColor>,
        weak: Option<HexColor>,
        text: Option<HexColor>,
    },
}

impl AppearanceColor {
    pub fn get_base(&self) -> Color {
        match self {
            AppearanceColor::Simple(color) => hex_to_color(*color),
            AppearanceColor::Complete { base, .. } => hex_to_color(*base),
        }
    }

    pub fn get_text(&self) -> Option<Color> {
        match self {
            AppearanceColor::Simple(_) => None,
            AppearanceColor::Complete { text, .. } => text.map(hex_to_color),
        }
    }

    pub fn get_weak_pair(&self, text_fallback: Color) -> Option<palette::Pair> {
        match self {
            AppearanceColor::Simple(_) => None,
            AppearanceColor::Complete { weak, text, .. } => {
                weak.map(|color| hex_to_pair(color, *text, text_fallback))
            }
        }
    }

    pub fn get_strong_pair(&self, text_fallback: Color) -> Option<palette::Pair> {
        match self {
            AppearanceColor::Simple(_) => None,
            AppearanceColor::Complete { strong, text, .. } => {
                strong.map(|color| hex_to_pair(color, *text, text_fallback))
            }
        }
    }
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(untagged)]
pub enum BackgroundAppearanceColor {
    Simple(HexColor),
    Complete {
        base: HexColor,
        weakest: Option<HexColor>,
        weaker: Option<HexColor>,
        weak: Option<HexColor>,
        neutral: Option<HexColor>,
        strong: Option<HexColor>,
        stronger: Option<HexColor>,
        strongest: Option<HexColor>,
        text: Option<HexColor>,
    },
}

impl BackgroundAppearanceColor {
    pub fn get_base(&self) -> Color {
        match self {
            BackgroundAppearanceColor::Simple(color) => hex_to_color(*color),
            BackgroundAppearanceColor::Complete { base, .. } => hex_to_color(*base),
        }
    }

    pub fn get_text(&self) -> Option<Color> {
        match self {
            BackgroundAppearanceColor::Simple(_) => None,
            BackgroundAppearanceColor::Complete { text, .. } => text.map(hex_to_color),
        }
    }

    pub fn get_pair(&self, level: BackgroundLevel, text_fallback: Color) -> Option<palette::Pair> {
        match self {
            BackgroundAppearanceColor::Simple(_) => None,
            BackgroundAppearanceColor::Complete {
                weakest,
                weaker,
                weak,
                neutral,
                strong,
                stronger,
                strongest,
                text,
                ..
            } => {
                let hex = match level {
                    BackgroundLevel::Weakest => *weakest,
                    BackgroundLevel::Weaker => *weaker,
                    BackgroundLevel::Weak => *weak,
                    BackgroundLevel::Neutral => *neutral,
                    BackgroundLevel::Strong => *strong,
                    BackgroundLevel::Stronger => *stronger,
                    BackgroundLevel::Strongest => *strongest,
                };
                hex.map(|h| hex_to_pair(h, *text, text_fallback))
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BackgroundLevel {
    Weakest,
    Weaker,
    Weak,
    Neutral,
    Strong,
    Stronger,
    Strongest,
}

#[derive(Deserialize, Default, Copy, Clone, Eq, PartialEq, Debug)]
pub enum AppearanceStyle {
    #[default]
    Islands,
    Solid,
    Gradient,
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(default)]
pub struct MenuAppearance {
    #[serde(deserialize_with = "opacity_deserializer")]
    pub opacity: f32,
    pub backdrop: f32,
}

impl Default for MenuAppearance {
    fn default() -> Self {
        Self {
            opacity: default_opacity(),
            backdrop: f32::default(),
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Appearance {
    pub font_name: Option<String>,
    #[serde(deserialize_with = "scale_factor_deserializer")]
    pub scale_factor: f64,
    pub style: AppearanceStyle,
    #[serde(deserialize_with = "opacity_deserializer")]
    pub opacity: f32,
    /// Bar yüksekliği "density" — noctalia naming:
    ///   Mini (23) / Compact (27) / Default (31) / Comfortable (37) /
    ///   Spacious (47). 0/Default seçilirse legacy 34px kullanılır.
    #[serde(default)]
    pub bar_density: BarDensity,
    /// Islands modunda her modül kapsülüne ince outline çiz.
    #[serde(default)]
    pub show_outline: bool,
    pub menu: MenuAppearance,
    pub background_color: BackgroundAppearanceColor,
    pub primary_color: AppearanceColor,
    pub success_color: AppearanceColor,
    pub warning_color: AppearanceColor,
    pub danger_color: AppearanceColor,
    pub text_color: AppearanceColor,
    pub workspace_colors: Vec<AppearanceColor>,
    pub special_workspace_colors: Option<Vec<AppearanceColor>>,
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BarDensity {
    Mini,
    Compact,
    #[default]
    Default,
    Comfortable,
    Spacious,
}

impl BarDensity {
    /// Bar yüksekliği piksel (yatay bar için). Noctalia tablosuyla
    /// uyumlu, scale_factor uygulanmadan önceki ham değerler.
    pub fn height(self) -> f64 {
        match self {
            BarDensity::Mini => 23.0,
            BarDensity::Compact => 27.0,
            BarDensity::Default => 31.0,
            BarDensity::Comfortable => 37.0,
            BarDensity::Spacious => 47.0,
        }
    }
}

static PRIMARY: HexColor = HexColor::rgb(122, 162, 247);

fn scale_factor_deserializer<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = f64::deserialize(deserializer)?;

    if v <= 0.0 {
        return Err(serde::de::Error::custom(
            "Scale factor must be greater than 0.0",
        ));
    }

    if v > 2.0 {
        return Err(serde::de::Error::custom(
            "Scale factor cannot be greater than 2.0",
        ));
    }

    Ok(v)
}

fn opacity_deserializer<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = f32::deserialize(deserializer)?;

    if v < 0.0 {
        return Err(serde::de::Error::custom("Opacity cannot be negative"));
    }

    if v > 1.0 {
        return Err(serde::de::Error::custom(
            "Opacity cannot be greater than 1.0",
        ));
    }

    Ok(v)
}

fn default_opacity() -> f32 {
    1.0
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            font_name: None,
            scale_factor: 1.0,
            style: AppearanceStyle::default(),
            opacity: default_opacity(),
            bar_density: BarDensity::default(),
            show_outline: false,
            menu: MenuAppearance::default(),
            background_color: BackgroundAppearanceColor::Complete {
                base: HexColor::rgb(26, 27, 38),
                weakest: None,
                weaker: None,
                weak: Some(HexColor::rgb(36, 39, 58)),
                neutral: None,
                strong: Some(HexColor::rgb(65, 72, 104)),
                stronger: None,
                strongest: None,
                text: None,
            },
            primary_color: AppearanceColor::Simple(PRIMARY),
            success_color: AppearanceColor::Simple(HexColor::rgb(158, 206, 106)),
            warning_color: AppearanceColor::Simple(HexColor::rgb(224, 175, 104)),
            danger_color: AppearanceColor::Simple(HexColor::rgb(247, 118, 142)),
            text_color: AppearanceColor::Simple(HexColor::rgb(169, 177, 214)),
            workspace_colors: vec![
                AppearanceColor::Simple(PRIMARY),
                AppearanceColor::Simple(HexColor::rgb(158, 206, 106)),
            ],
            special_workspace_colors: None,
        }
    }
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    #[default]
    Top,
    Bottom,
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layer {
    #[default]
    Bottom,
    Top,
    Overlay,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleName {
    Updates,
    Workspaces,
    WindowTitle,
    SystemInfo,
    NetworkSpeed,
    Dns,
    Ufw,
    Power,
    Podman,
    KeyboardLayout,
    Tray,
    Tempo,
    Privacy,
    Settings,
    MediaPlayer,
    Custom(String),
    Notifications,
}

impl<'de> Deserialize<'de> for ModuleName {
    fn deserialize<D>(deserializer: D) -> Result<ModuleName, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ModuleNameVisitor;
        impl Visitor<'_> for ModuleNameVisitor {
            type Value = ModuleName;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string representing a ModuleName")
            }
            fn visit_str<E>(self, value: &str) -> Result<ModuleName, E>
            where
                E: serde::de::Error,
            {
                Ok(match value {
                    "Updates" => ModuleName::Updates,
                    "Workspaces" => ModuleName::Workspaces,
                    "WindowTitle" => ModuleName::WindowTitle,
                    "SystemInfo" => ModuleName::SystemInfo,
                    "NetworkSpeed" => ModuleName::NetworkSpeed,
                    "Dns" => ModuleName::Dns,
                    "Ufw" => ModuleName::Ufw,
                    "Power" => ModuleName::Power,
                    "Podman" => ModuleName::Podman,
                    "KeyboardLayout" => ModuleName::KeyboardLayout,
                    "Tray" => ModuleName::Tray,
                    "Notifications" => ModuleName::Notifications,
                    "Tempo" => ModuleName::Tempo,
                    "Privacy" => ModuleName::Privacy,
                    "Settings" => ModuleName::Settings,
                    "MediaPlayer" => ModuleName::MediaPlayer,
                    other => ModuleName::Custom(other.to_string()),
                })
            }
        }
        deserializer.deserialize_str(ModuleNameVisitor)
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum ModuleDef {
    Single(ModuleName),
    Group(Vec<ModuleName>),
}

#[derive(Deserialize, Clone, Debug)]
pub struct Modules {
    #[serde(default)]
    pub left: Vec<ModuleDef>,
    #[serde(default)]
    pub center: Vec<ModuleDef>,
    #[serde(default)]
    pub right: Vec<ModuleDef>,
}

impl Default for Modules {
    fn default() -> Self {
        Self {
            left: vec![ModuleDef::Single(ModuleName::Workspaces)],
            center: vec![ModuleDef::Single(ModuleName::WindowTitle)],
            right: vec![ModuleDef::Group(vec![
                ModuleName::Tempo,
                ModuleName::Privacy,
                ModuleName::Settings,
            ])],
        }
    }
}

#[derive(Deserialize, Clone, Default, Debug, PartialEq, Eq)]
pub enum Outputs {
    #[default]
    All,
    Active,
    #[serde(deserialize_with = "non_empty")]
    Targets(Vec<String>),
}

fn non_empty<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let vec = <Vec<T>>::deserialize(d)?;
    if vec.is_empty() {
        use serde::de::Error;

        Err(D::Error::custom("need non-empty"))
    } else {
        Ok(vec)
    }
}

fn empty_string_as_none<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(d)?
        .and_then(|value| (!value.trim().is_empty()).then_some(value)))
}

/// Newtype wrapper around `Regex`to be deserializable and usable as a hashmap key
#[serde_as]
#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct RegexCfg(#[serde_as(as = "DisplayFromStr")] pub Regex);

impl PartialEq for RegexCfg {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}
impl Eq for RegexCfg {}

impl std::hash::Hash for RegexCfg {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // hash the raw pattern string
        self.0.as_str().hash(state);
    }
}

impl Deref for RegexCfg {
    type Target = Regex;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Deserialize, Copy, Clone, Default, PartialEq, Eq, Debug)]
pub enum CustomModuleType {
    #[default]
    Button,
    Text,
}

#[serde_as]
#[derive(Deserialize, Clone, Debug)]
pub struct CustomModuleDef {
    pub name: String,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,

    /// yields json lines containing text, alt, (pot tooltip)
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub listen_cmd: Option<String>,
    /// map of regex -> icon
    pub icons: Option<HashMap<RegexCfg, String>>,
    /// regex to show alert
    pub alert: Option<RegexCfg>,
    /// Display type: Button (clickable) or Text (display only)
    #[serde(default)]
    pub r#type: CustomModuleType,
    // .. appearance etc
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct OsdConfig {
    pub enabled: bool,
    pub timeout: u64,
}

impl Default for OsdConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout: 1500,
        }
    }
}

pub fn get_config(path: Option<PathBuf>) -> Result<(Config, PathBuf), Box<dyn Error + Send>> {
    match path {
        Some(p) => {
            info!("Config path provided {p:?}");
            expand_path(p).and_then(|expanded| {
                if !expanded.exists() {
                    Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("Config file does not exist: {}", expanded.display()),
                    )))
                } else {
                    Ok((read_config(&expanded).unwrap_or_default(), expanded))
                }
            })
        }
        None => expand_path(PathBuf::from(DEFAULT_CONFIG_FILE_PATH)).map(|expanded| {
            // Safety: DEFAULT_CONFIG_FILE_PATH is "~/.config/margo/mshell.toml" which
            // always has directory components. shellexpand only expands ~/$HOME and never
            // strips path components, so .parent() always returns Some.
            let parent = expanded
                .parent()
                .expect("Failed to get default config parent directory");

            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .expect("Failed to create default config parent directory");
            }

            (read_config(&expanded).unwrap_or_default(), expanded)
        }),
    }
}

fn expand_path(path: PathBuf) -> Result<PathBuf, Box<dyn Error + Send>> {
    let str_path = path.to_string_lossy();
    let expanded =
        shellexpand::full(&str_path).map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;

    Ok(PathBuf::from(expanded.to_string()))
}

fn read_config(path: &Path) -> Result<Config, Box<dyn Error + Send>> {
    let content =
        std::fs::read_to_string(path).map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;

    info!("Decoding config file {path:?}");

    let res = toml::from_str(&content);

    match res {
        Ok(config) => {
            info!("Config file loaded successfully");
            let mut config: Config = config;
            config.validate();
            Ok(config)
        }
        Err(e) => {
            warn!("Failed to parse config file: {e}");
            Err(Box::new(e))
        }
    }
}

enum Event {
    Changed,
    Removed,
}

pub fn subscription(path: &Path) -> Subscription<Message> {
    let path = path.to_path_buf();

    Subscription::run_with(path, |path| {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        channel(100, async move |mut output| {
            match (path.parent(), path.file_name(), Inotify::init()) {
                (Some(folder), Some(file_name), Ok(inotify)) => {
                    debug!("Watching config file at {path:?}");

                    let res = inotify.watches().add(
                        folder,
                        WatchMask::CREATE | WatchMask::DELETE | WatchMask::MOVE | WatchMask::MODIFY,
                    );

                    if let Err(e) = res {
                        error!("Failed to add watch for {folder:?}: {e}");
                        return;
                    }

                    let buffer = [0; 1024];
                    let stream = inotify.into_event_stream(buffer);

                    if let Ok(stream) = stream {
                        let mut stream = stream.ready_chunks(10);

                        debug!("Starting config file watch loop");

                        loop {
                            let events = stream.next().await.unwrap_or(vec![]);

                            debug!("Received inotify events: {events:?}");

                            let mut file_event = None;

                            for event in events {
                                debug!("Event: {event:?}");
                                match event {
                                    Ok(inotify::Event {
                                        name: Some(name),
                                        mask: EventMask::DELETE | EventMask::MOVED_FROM,
                                        ..
                                    }) if file_name == name => {
                                        debug!("File deleted or moved");
                                        file_event = Some(Event::Removed);
                                    }
                                    Ok(inotify::Event {
                                        name: Some(name),
                                        mask:
                                            EventMask::CREATE | EventMask::MODIFY | EventMask::MOVED_TO,
                                        ..
                                    }) if file_name == name => {
                                        debug!("File created or moved");

                                        file_event = Some(Event::Changed);
                                    }
                                    _ => {
                                        debug!("Ignoring event");
                                    }
                                }
                            }

                            match file_event {
                                Some(Event::Changed) => {
                                    info!("Reload config file");

                                    let new_config = read_config(&path).unwrap_or_default();

                                    let _ = output
                                        .send(Message::ConfigChanged(Box::new(new_config)))
                                        .await;
                                }
                                Some(Event::Removed) => {
                                    // wait and double check if the file is really gone
                                    sleep(Duration::from_millis(500)).await;

                                    if !path.exists() {
                                        info!("Config file removed");
                                        let _ = output
                                            .send(Message::ConfigChanged(Box::default()))
                                            .await;
                                    }
                                }
                                None => {
                                    debug!("No relevant file event detected.");
                                }
                            }
                        }
                    } else {
                        error!("Failed to create inotify event stream");
                    }
                }
                (None, _, _) => {
                    error!(
                        "Config file path does not have a parent directory, cannot watch for changes"
                    );
                }
                (_, None, _) => {
                    error!("Config file path does not have a file name, cannot watch for changes");
                }
                (_, _, Err(e)) => {
                    error!("Failed to initialize inotify: {e}");
                }
            }
        })
    })
}
