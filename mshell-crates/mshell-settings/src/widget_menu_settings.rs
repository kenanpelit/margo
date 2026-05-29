//! Per-widget menu settings — one component, parameterised by
//! `MenuKind`, that surfaces a given menu's `position` and
//! `minimum_width`. Used inside the `Widgets` sub-sidebar so each
//! menu gets its own focused settings page.
//!
//! The widgets-list editor (which BarWidget pills live inside a
//! menu) stays in the existing `menu_settings::Layout` page —
//! that's a cross-cutting view of every menu at once. These
//! per-menu pages are the "I just want to tweak THIS one"
//! shortcut.

use crate::cc_tiles_settings::{CcTilesSettingsInit, CcTilesSettingsModel};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, MenuStoreFields, MenusStoreFields,
    SystemUpdateBarWidgetStoreFields,
};
use mshell_config::schema::position::Position;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

/// Which menu this settings page targets. The enum carries
/// everything we need to read / write through `config_manager`
/// (descriptive label + reactive-field accessor dispatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuKind {
    AppLauncher,
    AudioDashboard,
    Bluetooth,
    Clipboard,
    Clock,
    CpuDashboard,
    Dashboard,
    MargoLayout,
    PluginPanel,
    MediaPlayer,
    Dns,
    Ip,
    Network,
    Notes,
    Notifications,
    Podman,
    Power,
    Screenshot,
    SystemUpdate,
    Valent,
    Weather,
    KeepAwake,
    Twilight,
    Keybinds,
    AlarmClock,
    ControlCenter,
    SshSessions,
    Ufw,
    Wallpaper,
}

impl MenuKind {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::AppLauncher => "App Launcher",
            Self::AudioDashboard => "Audio Dashboard",
            Self::Bluetooth => "Bluetooth",
            Self::Clipboard => "Clipboard",
            Self::Clock => "Clock",
            Self::CpuDashboard => "CPU Dashboard",
            Self::Dashboard => "Dashboard",
            Self::MargoLayout => "Margo Layout",
            Self::PluginPanel => "Plugin Panel",
            Self::MediaPlayer => "Media Player",
            Self::Dns => "DNS / VPN",
            Self::Ip => "Public IP",
            Self::Network => "Network Console",
            Self::Notes => "Notes Hub",
            Self::Notifications => "Notifications",
            Self::Podman => "Podman",
            Self::Power => "Power Profile",
            Self::Screenshot => "Screenshot",
            Self::SystemUpdate => "System Updates",
            Self::Valent => "Valent Connect",
            Self::Weather => "Weather",
            Self::KeepAwake => "Keep Awake",
            Self::Twilight => "Twilight",
            Self::Keybinds => "Keyboard Shortcuts",
            Self::AlarmClock => "Alarm Clock",
            Self::ControlCenter => "Control Center",
            Self::SshSessions => "SSH Sessions",
            Self::Ufw => "UFW Firewall",
            Self::Wallpaper => "Wallpaper",
        }
    }

    /// All known menu kinds, in the order they should appear in
    /// the cross-cutting Menus settings page. Kept stable so the
    /// scroll position survives a config reload.
    pub(crate) fn all() -> &'static [MenuKind] {
        &[
            MenuKind::Clock,
            MenuKind::Dashboard,
            MenuKind::Clipboard,
            MenuKind::Screenshot,
            MenuKind::Notifications,
            MenuKind::AppLauncher,
            MenuKind::Wallpaper,
            MenuKind::MediaPlayer,
            MenuKind::Power,
            MenuKind::Bluetooth,
            MenuKind::CpuDashboard,
            MenuKind::AudioDashboard,
            MenuKind::SystemUpdate,
            MenuKind::Valent,
            MenuKind::Weather,
            MenuKind::KeepAwake,
            MenuKind::Twilight,
            MenuKind::Keybinds,
            MenuKind::AlarmClock,
            MenuKind::ControlCenter,
            MenuKind::SshSessions,
            MenuKind::Ufw,
            MenuKind::Dns,
            MenuKind::Podman,
            MenuKind::Notes,
            MenuKind::Ip,
            MenuKind::Network,
            MenuKind::MargoLayout,
            MenuKind::PluginPanel,
        ]
    }

    /// Snapshot the menu's current position. `_untracked` so the
    /// initial model load doesn't subscribe; the `EffectScope`
    /// below subscribes explicitly.
    fn read_position(self) -> Position {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().position().get_untracked(),
            Self::Clipboard => m.clipboard_menu().position().get_untracked(),
            Self::Clock => m.clock_menu().position().get_untracked(),
            Self::Dashboard => m.dashboard_menu().position().get_untracked(),
            Self::MediaPlayer => m.media_player_menu().position().get_untracked(),
            Self::Dns => m.dns_menu().position().get_untracked(),
            Self::Ip => m.ip_menu().position().get_untracked(),
            Self::Network => m.network_menu().position().get_untracked(),
            Self::Notes => m.notes_menu().position().get_untracked(),
            Self::Notifications => m.notification_menu().position().get_untracked(),
            Self::Podman => m.podman_menu().position().get_untracked(),
            Self::Wallpaper => m.wallpaper_menu().position().get_untracked(),
            Self::Power => m.power_menu().position().get_untracked(),
            Self::Screenshot => m.screenshot_menu().position().get_untracked(),
            Self::Ufw => m.ufw_menu().position().get_untracked(),
            Self::Bluetooth => m.bluetooth_menu().position().get_untracked(),
            Self::CpuDashboard => m.cpu_dashboard_menu().position().get_untracked(),
            Self::AudioDashboard => m.audio_dashboard_menu().position().get_untracked(),
            Self::SystemUpdate => m.system_update_menu().position().get_untracked(),
            Self::Valent => m.valent_menu().position().get_untracked(),
            Self::Weather => m.weather_menu().position().get_untracked(),
            Self::KeepAwake => m.keep_awake_menu().position().get_untracked(),
            Self::Twilight => m.twilight_menu().position().get_untracked(),
            Self::Keybinds => m.keybinds_menu().position().get_untracked(),
            Self::AlarmClock => m.alarmclock_menu().position().get_untracked(),
            Self::ControlCenter => m.control_center_menu().position().get_untracked(),
            Self::SshSessions => m.ssh_menu().position().get_untracked(),
            Self::MargoLayout => m.margo_layout_menu().position().get_untracked(),
            Self::PluginPanel => m.plugin_panel_menu().position().get_untracked(),
        }
    }

    fn read_min_width(self) -> i32 {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().minimum_width().get_untracked(),
            Self::Clipboard => m.clipboard_menu().minimum_width().get_untracked(),
            Self::Clock => m.clock_menu().minimum_width().get_untracked(),
            Self::Dashboard => m.dashboard_menu().minimum_width().get_untracked(),
            Self::MediaPlayer => m.media_player_menu().minimum_width().get_untracked(),
            Self::Dns => m.dns_menu().minimum_width().get_untracked(),
            Self::Ip => m.ip_menu().minimum_width().get_untracked(),
            Self::Network => m.network_menu().minimum_width().get_untracked(),
            Self::Notes => m.notes_menu().minimum_width().get_untracked(),
            Self::Notifications => m.notification_menu().minimum_width().get_untracked(),
            Self::Podman => m.podman_menu().minimum_width().get_untracked(),
            Self::Wallpaper => m.wallpaper_menu().minimum_width().get_untracked(),
            Self::Power => m.power_menu().minimum_width().get_untracked(),
            Self::Screenshot => m.screenshot_menu().minimum_width().get_untracked(),
            Self::Ufw => m.ufw_menu().minimum_width().get_untracked(),
            Self::Bluetooth => m.bluetooth_menu().minimum_width().get_untracked(),
            Self::CpuDashboard => m.cpu_dashboard_menu().minimum_width().get_untracked(),
            Self::AudioDashboard => m.audio_dashboard_menu().minimum_width().get_untracked(),
            Self::SystemUpdate => m.system_update_menu().minimum_width().get_untracked(),
            Self::Valent => m.valent_menu().minimum_width().get_untracked(),
            Self::Weather => m.weather_menu().minimum_width().get_untracked(),
            Self::KeepAwake => m.keep_awake_menu().minimum_width().get_untracked(),
            Self::Twilight => m.twilight_menu().minimum_width().get_untracked(),
            Self::Keybinds => m.keybinds_menu().minimum_width().get_untracked(),
            Self::AlarmClock => m.alarmclock_menu().minimum_width().get_untracked(),
            Self::ControlCenter => m.control_center_menu().minimum_width().get_untracked(),
            Self::SshSessions => m.ssh_menu().minimum_width().get_untracked(),
            Self::MargoLayout => m.margo_layout_menu().minimum_width().get_untracked(),
            Self::PluginPanel => m.plugin_panel_menu().minimum_width().get_untracked(),
        }
    }

    fn tracked_position(self) -> Position {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().position().get(),
            Self::Clipboard => m.clipboard_menu().position().get(),
            Self::Clock => m.clock_menu().position().get(),
            Self::Dashboard => m.dashboard_menu().position().get(),
            Self::MediaPlayer => m.media_player_menu().position().get(),
            Self::Dns => m.dns_menu().position().get(),
            Self::Ip => m.ip_menu().position().get(),
            Self::Network => m.network_menu().position().get(),
            Self::Notes => m.notes_menu().position().get(),
            Self::Notifications => m.notification_menu().position().get(),
            Self::Podman => m.podman_menu().position().get(),
            Self::Wallpaper => m.wallpaper_menu().position().get(),
            Self::Power => m.power_menu().position().get(),
            Self::Screenshot => m.screenshot_menu().position().get(),
            Self::Ufw => m.ufw_menu().position().get(),
            Self::Bluetooth => m.bluetooth_menu().position().get(),
            Self::CpuDashboard => m.cpu_dashboard_menu().position().get(),
            Self::AudioDashboard => m.audio_dashboard_menu().position().get(),
            Self::SystemUpdate => m.system_update_menu().position().get(),
            Self::Valent => m.valent_menu().position().get(),
            Self::Weather => m.weather_menu().position().get(),
            Self::KeepAwake => m.keep_awake_menu().position().get(),
            Self::Twilight => m.twilight_menu().position().get(),
            Self::Keybinds => m.keybinds_menu().position().get(),
            Self::AlarmClock => m.alarmclock_menu().position().get(),
            Self::ControlCenter => m.control_center_menu().position().get(),
            Self::SshSessions => m.ssh_menu().position().get(),
            Self::MargoLayout => m.margo_layout_menu().position().get(),
            Self::PluginPanel => m.plugin_panel_menu().position().get(),
        }
    }

    fn tracked_min_width(self) -> i32 {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().minimum_width().get(),
            Self::Clipboard => m.clipboard_menu().minimum_width().get(),
            Self::Clock => m.clock_menu().minimum_width().get(),
            Self::Dashboard => m.dashboard_menu().minimum_width().get(),
            Self::MediaPlayer => m.media_player_menu().minimum_width().get(),
            Self::Dns => m.dns_menu().minimum_width().get(),
            Self::Ip => m.ip_menu().minimum_width().get(),
            Self::Network => m.network_menu().minimum_width().get(),
            Self::Notes => m.notes_menu().minimum_width().get(),
            Self::Notifications => m.notification_menu().minimum_width().get(),
            Self::Podman => m.podman_menu().minimum_width().get(),
            Self::Wallpaper => m.wallpaper_menu().minimum_width().get(),
            Self::Power => m.power_menu().minimum_width().get(),
            Self::Screenshot => m.screenshot_menu().minimum_width().get(),
            Self::Ufw => m.ufw_menu().minimum_width().get(),
            Self::Bluetooth => m.bluetooth_menu().minimum_width().get(),
            Self::CpuDashboard => m.cpu_dashboard_menu().minimum_width().get(),
            Self::AudioDashboard => m.audio_dashboard_menu().minimum_width().get(),
            Self::SystemUpdate => m.system_update_menu().minimum_width().get(),
            Self::Valent => m.valent_menu().minimum_width().get(),
            Self::Weather => m.weather_menu().minimum_width().get(),
            Self::KeepAwake => m.keep_awake_menu().minimum_width().get(),
            Self::Twilight => m.twilight_menu().minimum_width().get(),
            Self::Keybinds => m.keybinds_menu().minimum_width().get(),
            Self::AlarmClock => m.alarmclock_menu().minimum_width().get(),
            Self::ControlCenter => m.control_center_menu().minimum_width().get(),
            Self::SshSessions => m.ssh_menu().minimum_width().get(),
            Self::MargoLayout => m.margo_layout_menu().minimum_width().get(),
            Self::PluginPanel => m.plugin_panel_menu().minimum_width().get(),
        }
    }

    fn write_position(self, p: Position) {
        config_manager().update_config(|c| match self {
            Self::AppLauncher => c.menus.app_launcher_menu.position = p,
            Self::Clipboard => c.menus.clipboard_menu.position = p,
            Self::Clock => c.menus.clock_menu.position = p,
            Self::Dashboard => c.menus.dashboard_menu.position = p,
            Self::MediaPlayer => c.menus.media_player_menu.position = p,
            Self::Dns => c.menus.dns_menu.position = p,
            Self::Ip => c.menus.ip_menu.position = p,
            Self::Network => c.menus.network_menu.position = p,
            Self::Notes => c.menus.notes_menu.position = p,
            Self::Notifications => c.menus.notification_menu.position = p,
            Self::Podman => c.menus.podman_menu.position = p,
            Self::Wallpaper => c.menus.wallpaper_menu.position = p,
            Self::Power => c.menus.power_menu.position = p,
            Self::Screenshot => c.menus.screenshot_menu.position = p,
            Self::Ufw => c.menus.ufw_menu.position = p,
            Self::Bluetooth => c.menus.bluetooth_menu.position = p,
            Self::CpuDashboard => c.menus.cpu_dashboard_menu.position = p,
            Self::AudioDashboard => c.menus.audio_dashboard_menu.position = p,
            Self::SystemUpdate => c.menus.system_update_menu.position = p,
            Self::Valent => c.menus.valent_menu.position = p,
            Self::Weather => c.menus.weather_menu.position = p,
            Self::KeepAwake => c.menus.keep_awake_menu.position = p,
            Self::Twilight => c.menus.twilight_menu.position = p,
            Self::Keybinds => c.menus.keybinds_menu.position = p,
            Self::AlarmClock => c.menus.alarmclock_menu.position = p,
            Self::ControlCenter => c.menus.control_center_menu.position = p,
            Self::SshSessions => c.menus.ssh_menu.position = p,
            Self::MargoLayout => c.menus.margo_layout_menu.position = p,
            Self::PluginPanel => c.menus.plugin_panel_menu.position = p,
        });
    }

    fn write_min_width(self, w: i32) {
        config_manager().update_config(|c| match self {
            Self::AppLauncher => c.menus.app_launcher_menu.minimum_width = w,
            Self::Clipboard => c.menus.clipboard_menu.minimum_width = w,
            Self::Clock => c.menus.clock_menu.minimum_width = w,
            Self::Dashboard => c.menus.dashboard_menu.minimum_width = w,
            Self::MediaPlayer => c.menus.media_player_menu.minimum_width = w,
            Self::Dns => c.menus.dns_menu.minimum_width = w,
            Self::Ip => c.menus.ip_menu.minimum_width = w,
            Self::Network => c.menus.network_menu.minimum_width = w,
            Self::Notes => c.menus.notes_menu.minimum_width = w,
            Self::Notifications => c.menus.notification_menu.minimum_width = w,
            Self::Podman => c.menus.podman_menu.minimum_width = w,
            Self::Wallpaper => c.menus.wallpaper_menu.minimum_width = w,
            Self::Power => c.menus.power_menu.minimum_width = w,
            Self::Screenshot => c.menus.screenshot_menu.minimum_width = w,
            Self::Ufw => c.menus.ufw_menu.minimum_width = w,
            Self::Bluetooth => c.menus.bluetooth_menu.minimum_width = w,
            Self::CpuDashboard => c.menus.cpu_dashboard_menu.minimum_width = w,
            Self::AudioDashboard => c.menus.audio_dashboard_menu.minimum_width = w,
            Self::SystemUpdate => c.menus.system_update_menu.minimum_width = w,
            Self::Valent => c.menus.valent_menu.minimum_width = w,
            Self::Weather => c.menus.weather_menu.minimum_width = w,
            Self::KeepAwake => c.menus.keep_awake_menu.minimum_width = w,
            Self::Twilight => c.menus.twilight_menu.minimum_width = w,
            Self::Keybinds => c.menus.keybinds_menu.minimum_width = w,
            Self::AlarmClock => c.menus.alarmclock_menu.minimum_width = w,
            Self::ControlCenter => c.menus.control_center_menu.minimum_width = w,
            Self::SshSessions => c.menus.ssh_menu.minimum_width = w,
            Self::MargoLayout => c.menus.margo_layout_menu.minimum_width = w,
            Self::PluginPanel => c.menus.plugin_panel_menu.minimum_width = w,
        });
    }

    fn read_max_height(self) -> i32 {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().maximum_height().get_untracked(),
            Self::Clipboard => m.clipboard_menu().maximum_height().get_untracked(),
            Self::Clock => m.clock_menu().maximum_height().get_untracked(),
            Self::Dashboard => m.dashboard_menu().maximum_height().get_untracked(),
            Self::MediaPlayer => m.media_player_menu().maximum_height().get_untracked(),
            Self::Dns => m.dns_menu().maximum_height().get_untracked(),
            Self::Ip => m.ip_menu().maximum_height().get_untracked(),
            Self::Network => m.network_menu().maximum_height().get_untracked(),
            Self::Notes => m.notes_menu().maximum_height().get_untracked(),
            Self::Notifications => m.notification_menu().maximum_height().get_untracked(),
            Self::Podman => m.podman_menu().maximum_height().get_untracked(),
            Self::Wallpaper => m.wallpaper_menu().maximum_height().get_untracked(),
            Self::Power => m.power_menu().maximum_height().get_untracked(),
            Self::Screenshot => m.screenshot_menu().maximum_height().get_untracked(),
            Self::Ufw => m.ufw_menu().maximum_height().get_untracked(),
            Self::Bluetooth => m.bluetooth_menu().maximum_height().get_untracked(),
            Self::CpuDashboard => m.cpu_dashboard_menu().maximum_height().get_untracked(),
            Self::AudioDashboard => m.audio_dashboard_menu().maximum_height().get_untracked(),
            Self::SystemUpdate => m.system_update_menu().maximum_height().get_untracked(),
            Self::Valent => m.valent_menu().maximum_height().get_untracked(),
            Self::Weather => m.weather_menu().maximum_height().get_untracked(),
            Self::KeepAwake => m.keep_awake_menu().maximum_height().get_untracked(),
            Self::Twilight => m.twilight_menu().maximum_height().get_untracked(),
            Self::Keybinds => m.keybinds_menu().maximum_height().get_untracked(),
            Self::AlarmClock => m.alarmclock_menu().maximum_height().get_untracked(),
            Self::ControlCenter => m.control_center_menu().maximum_height().get_untracked(),
            Self::SshSessions => m.ssh_menu().maximum_height().get_untracked(),
            Self::MargoLayout => m.margo_layout_menu().maximum_height().get_untracked(),
            Self::PluginPanel => m.plugin_panel_menu().maximum_height().get_untracked(),
        }
    }

    fn tracked_max_height(self) -> i32 {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().maximum_height().get(),
            Self::Clipboard => m.clipboard_menu().maximum_height().get(),
            Self::Clock => m.clock_menu().maximum_height().get(),
            Self::Dashboard => m.dashboard_menu().maximum_height().get(),
            Self::MediaPlayer => m.media_player_menu().maximum_height().get(),
            Self::Dns => m.dns_menu().maximum_height().get(),
            Self::Ip => m.ip_menu().maximum_height().get(),
            Self::Network => m.network_menu().maximum_height().get(),
            Self::Notes => m.notes_menu().maximum_height().get(),
            Self::Notifications => m.notification_menu().maximum_height().get(),
            Self::Podman => m.podman_menu().maximum_height().get(),
            Self::Wallpaper => m.wallpaper_menu().maximum_height().get(),
            Self::Power => m.power_menu().maximum_height().get(),
            Self::Screenshot => m.screenshot_menu().maximum_height().get(),
            Self::Ufw => m.ufw_menu().maximum_height().get(),
            Self::Bluetooth => m.bluetooth_menu().maximum_height().get(),
            Self::CpuDashboard => m.cpu_dashboard_menu().maximum_height().get(),
            Self::AudioDashboard => m.audio_dashboard_menu().maximum_height().get(),
            Self::SystemUpdate => m.system_update_menu().maximum_height().get(),
            Self::Valent => m.valent_menu().maximum_height().get(),
            Self::Weather => m.weather_menu().maximum_height().get(),
            Self::KeepAwake => m.keep_awake_menu().maximum_height().get(),
            Self::Twilight => m.twilight_menu().maximum_height().get(),
            Self::Keybinds => m.keybinds_menu().maximum_height().get(),
            Self::AlarmClock => m.alarmclock_menu().maximum_height().get(),
            Self::ControlCenter => m.control_center_menu().maximum_height().get(),
            Self::SshSessions => m.ssh_menu().maximum_height().get(),
            Self::MargoLayout => m.margo_layout_menu().maximum_height().get(),
            Self::PluginPanel => m.plugin_panel_menu().maximum_height().get(),
        }
    }

    fn write_max_height(self, h: i32) {
        config_manager().update_config(|c| match self {
            Self::AppLauncher => c.menus.app_launcher_menu.maximum_height = h,
            Self::Clipboard => c.menus.clipboard_menu.maximum_height = h,
            Self::Clock => c.menus.clock_menu.maximum_height = h,
            Self::Dashboard => c.menus.dashboard_menu.maximum_height = h,
            Self::MediaPlayer => c.menus.media_player_menu.maximum_height = h,
            Self::Dns => c.menus.dns_menu.maximum_height = h,
            Self::Ip => c.menus.ip_menu.maximum_height = h,
            Self::Network => c.menus.network_menu.maximum_height = h,
            Self::Notes => c.menus.notes_menu.maximum_height = h,
            Self::Notifications => c.menus.notification_menu.maximum_height = h,
            Self::Podman => c.menus.podman_menu.maximum_height = h,
            Self::Wallpaper => c.menus.wallpaper_menu.maximum_height = h,
            Self::Power => c.menus.power_menu.maximum_height = h,
            Self::Screenshot => c.menus.screenshot_menu.maximum_height = h,
            Self::Ufw => c.menus.ufw_menu.maximum_height = h,
            Self::Bluetooth => c.menus.bluetooth_menu.maximum_height = h,
            Self::CpuDashboard => c.menus.cpu_dashboard_menu.maximum_height = h,
            Self::AudioDashboard => c.menus.audio_dashboard_menu.maximum_height = h,
            Self::SystemUpdate => c.menus.system_update_menu.maximum_height = h,
            Self::Valent => c.menus.valent_menu.maximum_height = h,
            Self::Weather => c.menus.weather_menu.maximum_height = h,
            Self::KeepAwake => c.menus.keep_awake_menu.maximum_height = h,
            Self::Twilight => c.menus.twilight_menu.maximum_height = h,
            Self::Keybinds => c.menus.keybinds_menu.maximum_height = h,
            Self::AlarmClock => c.menus.alarmclock_menu.maximum_height = h,
            Self::ControlCenter => c.menus.control_center_menu.maximum_height = h,
            Self::SshSessions => c.menus.ssh_menu.maximum_height = h,
            Self::MargoLayout => c.menus.margo_layout_menu.maximum_height = h,
            Self::PluginPanel => c.menus.plugin_panel_menu.maximum_height = h,
        });
    }

    /// Snapshot the menu's current widget list. Used to seed the
    /// `MenuWidgetListModel` factory at panel-creation time.
    pub(crate) fn read_widgets(self) -> Vec<mshell_config::schema::menu_widgets::MenuWidget> {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().widgets().get_untracked(),
            Self::Clipboard => m.clipboard_menu().widgets().get_untracked(),
            Self::Clock => m.clock_menu().widgets().get_untracked(),
            Self::Dashboard => m.dashboard_menu().widgets().get_untracked(),
            Self::MediaPlayer => m.media_player_menu().widgets().get_untracked(),
            Self::Dns => m.dns_menu().widgets().get_untracked(),
            Self::Ip => m.ip_menu().widgets().get_untracked(),
            Self::Network => m.network_menu().widgets().get_untracked(),
            Self::Notes => m.notes_menu().widgets().get_untracked(),
            Self::Notifications => m.notification_menu().widgets().get_untracked(),
            Self::Podman => m.podman_menu().widgets().get_untracked(),
            Self::Wallpaper => m.wallpaper_menu().widgets().get_untracked(),
            Self::Power => m.power_menu().widgets().get_untracked(),
            Self::Screenshot => m.screenshot_menu().widgets().get_untracked(),
            Self::Ufw => m.ufw_menu().widgets().get_untracked(),
            Self::Bluetooth => m.bluetooth_menu().widgets().get_untracked(),
            Self::CpuDashboard => m.cpu_dashboard_menu().widgets().get_untracked(),
            Self::AudioDashboard => m.audio_dashboard_menu().widgets().get_untracked(),
            Self::SystemUpdate => m.system_update_menu().widgets().get_untracked(),
            Self::Valent => m.valent_menu().widgets().get_untracked(),
            Self::Weather => m.weather_menu().widgets().get_untracked(),
            Self::KeepAwake => m.keep_awake_menu().widgets().get_untracked(),
            Self::Twilight => m.twilight_menu().widgets().get_untracked(),
            Self::Keybinds => m.keybinds_menu().widgets().get_untracked(),
            Self::AlarmClock => m.alarmclock_menu().widgets().get_untracked(),
            Self::ControlCenter => m.control_center_menu().widgets().get_untracked(),
            Self::SshSessions => m.ssh_menu().widgets().get_untracked(),
            Self::MargoLayout => m.margo_layout_menu().widgets().get_untracked(),
            Self::PluginPanel => m.plugin_panel_menu().widgets().get_untracked(),
        }
    }

    /// Tracked read — subscribes the calling effect to widget-list
    /// changes so an external `mshellctl config reload` repaints
    /// the panel without a UI restart.
    pub(crate) fn tracked_widgets(self) -> Vec<mshell_config::schema::menu_widgets::MenuWidget> {
        let m = config_manager().config().menus();
        match self {
            Self::AppLauncher => m.app_launcher_menu().widgets().get(),
            Self::Clipboard => m.clipboard_menu().widgets().get(),
            Self::Clock => m.clock_menu().widgets().get(),
            Self::Dashboard => m.dashboard_menu().widgets().get(),
            Self::MediaPlayer => m.media_player_menu().widgets().get(),
            Self::Dns => m.dns_menu().widgets().get(),
            Self::Ip => m.ip_menu().widgets().get(),
            Self::Network => m.network_menu().widgets().get(),
            Self::Notes => m.notes_menu().widgets().get(),
            Self::Notifications => m.notification_menu().widgets().get(),
            Self::Podman => m.podman_menu().widgets().get(),
            Self::Wallpaper => m.wallpaper_menu().widgets().get(),
            Self::Power => m.power_menu().widgets().get(),
            Self::Screenshot => m.screenshot_menu().widgets().get(),
            Self::Ufw => m.ufw_menu().widgets().get(),
            Self::Bluetooth => m.bluetooth_menu().widgets().get(),
            Self::CpuDashboard => m.cpu_dashboard_menu().widgets().get(),
            Self::AudioDashboard => m.audio_dashboard_menu().widgets().get(),
            Self::SystemUpdate => m.system_update_menu().widgets().get(),
            Self::Valent => m.valent_menu().widgets().get(),
            Self::Weather => m.weather_menu().widgets().get(),
            Self::KeepAwake => m.keep_awake_menu().widgets().get(),
            Self::Twilight => m.twilight_menu().widgets().get(),
            Self::Keybinds => m.keybinds_menu().widgets().get(),
            Self::AlarmClock => m.alarmclock_menu().widgets().get(),
            Self::ControlCenter => m.control_center_menu().widgets().get(),
            Self::SshSessions => m.ssh_menu().widgets().get(),
            Self::MargoLayout => m.margo_layout_menu().widgets().get(),
            Self::PluginPanel => m.plugin_panel_menu().widgets().get(),
        }
    }

    /// Persist a new widget list to disk. Called from the panel
    /// when the in-UI reorder/add/remove fires.
    pub(crate) fn write_widgets(
        self,
        widgets: Vec<mshell_config::schema::menu_widgets::MenuWidget>,
    ) {
        config_manager().update_config(|c| match self {
            Self::AppLauncher => c.menus.app_launcher_menu.widgets = widgets,
            Self::Clipboard => c.menus.clipboard_menu.widgets = widgets,
            Self::Clock => c.menus.clock_menu.widgets = widgets,
            Self::Dashboard => c.menus.dashboard_menu.widgets = widgets,
            Self::MediaPlayer => c.menus.media_player_menu.widgets = widgets,
            Self::Dns => c.menus.dns_menu.widgets = widgets,
            Self::Ip => c.menus.ip_menu.widgets = widgets,
            Self::Network => c.menus.network_menu.widgets = widgets,
            Self::Notes => c.menus.notes_menu.widgets = widgets,
            Self::Notifications => c.menus.notification_menu.widgets = widgets,
            Self::Podman => c.menus.podman_menu.widgets = widgets,
            Self::Wallpaper => c.menus.wallpaper_menu.widgets = widgets,
            Self::Power => c.menus.power_menu.widgets = widgets,
            Self::Screenshot => c.menus.screenshot_menu.widgets = widgets,
            Self::Ufw => c.menus.ufw_menu.widgets = widgets,
            Self::Bluetooth => c.menus.bluetooth_menu.widgets = widgets,
            Self::CpuDashboard => c.menus.cpu_dashboard_menu.widgets = widgets,
            Self::AudioDashboard => c.menus.audio_dashboard_menu.widgets = widgets,
            Self::SystemUpdate => c.menus.system_update_menu.widgets = widgets,
            Self::Valent => c.menus.valent_menu.widgets = widgets,
            Self::Weather => c.menus.weather_menu.widgets = widgets,
            Self::KeepAwake => c.menus.keep_awake_menu.widgets = widgets,
            Self::Twilight => c.menus.twilight_menu.widgets = widgets,
            Self::Keybinds => c.menus.keybinds_menu.widgets = widgets,
            Self::AlarmClock => c.menus.alarmclock_menu.widgets = widgets,
            Self::ControlCenter => c.menus.control_center_menu.widgets = widgets,
            Self::SshSessions => c.menus.ssh_menu.widgets = widgets,
            Self::MargoLayout => c.menus.margo_layout_menu.widgets = widgets,
            Self::PluginPanel => c.menus.plugin_panel_menu.widgets = widgets,
        });
    }
}

#[derive(Debug)]
pub(crate) struct WidgetMenuSettingsModel {
    kind: MenuKind,
    position: Position,
    minimum_width: i32,
    /// Maximum visible content height in pixels. 0 = no cap.
    maximum_height: i32,
    /// SystemUpdate-only: the pill's poll cadence in minutes.
    /// Unused (kept at 0) for every other kind — the view hides
    /// the cadence section unless `kind == SystemUpdate`.
    check_interval_minutes: u32,
    position_model: gtk::StringList,
    /// ControlCenter-only: the tiles order/visibility sub-section.
    /// `None` for every other menu kind.
    _cc_tiles_controller: Option<Controller<CcTilesSettingsModel>>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum WidgetMenuSettingsInput {
    PositionPicked(u32),
    MinWidthChanged(i32),
    MaxHeightChanged(i32),
    PositionEffect(Position),
    MinWidthEffect(i32),
    MaxHeightEffect(i32),
    CheckIntervalChanged(u32),
    CheckIntervalEffect(u32),
}

#[derive(Debug)]
pub(crate) enum WidgetMenuSettingsOutput {}

pub(crate) struct WidgetMenuSettingsInit {
    pub(crate) kind: MenuKind,
}

#[relm4::component(pub(crate))]
impl Component for WidgetMenuSettingsModel {
    type CommandOutput = ();
    type Input = WidgetMenuSettingsInput;
    type Output = WidgetMenuSettingsOutput;
    type Init = WidgetMenuSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            #[name = "page_box"]
            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("view-list-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Menu widget",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Per-menu widget configuration — which sub-widgets show up inside a given menu and in what order.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    #[watch]
                    set_label: model.kind.display_name(),
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Per-menu layout. The widgets that appear inside this menu are configured under Widgets → Layout.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Position",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Which screen edge this menu anchors to.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 180,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.position_model),
                        #[watch]
                        #[block_signal(position_handler)]
                        set_selected: model.position.to_index(),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(WidgetMenuSettingsInput::PositionPicked(dd.selected()));
                        } @position_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Minimum Width",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Width floor in pixels. The menu may grow past this for long content.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (200.0, 2000.0),
                        set_increments: (10.0, 50.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(min_width_handler)]
                        set_value: model.minimum_width as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WidgetMenuSettingsInput::MinWidthChanged(s.value() as i32));
                        } @min_width_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Maximum Height",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Viewport cap in pixels. The menu scrolls vertically past this height. Set to 0 to disable the cap and let the menu grow to fit its contents.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        // 0 = uncapped; otherwise reasonable monitor-sized range.
                        set_range: (0.0, 2000.0),
                        set_increments: (10.0, 50.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(max_height_handler)]
                        set_value: model.maximum_height as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WidgetMenuSettingsInput::MaxHeightChanged(s.value() as i32));
                        } @max_height_handler,
                    },
                },

                // ── System-update-only cadence ───────────────
                //
                // The repo / AUR / Flatpak source toggles live in
                // the panel itself (open the menu → top row); only
                // the poll cadence is a set-once preference, so it
                // stays here. Hidden for every other menu kind.
                gtk::Separator {
                    #[watch]
                    set_visible: model.kind == MenuKind::SystemUpdate,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    #[watch]
                    set_visible: model.kind == MenuKind::SystemUpdate,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Check interval (minutes)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "How often the pill re-checks pending upgrades. Default 180 (3 h). Right-click the pill for an immediate manual re-check. Which sources to probe (Repo / AUR / Flatpak) is toggled inside the panel itself.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1.0, 1440.0),
                        set_increments: (5.0, 30.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(interval_handler)]
                        set_value: model.check_interval_minutes as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WidgetMenuSettingsInput::CheckIntervalChanged(s.value() as u32));
                        } @interval_handler,
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let position_refs: Vec<&str> =
            Position::all().iter().map(|p| p.display_name()).collect();
        let position_model = gtk::StringList::new(&position_refs);

        let mut effects = EffectScope::new();

        let kind = params.kind;
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let p = kind.tracked_position();
            sender_clone.input(WidgetMenuSettingsInput::PositionEffect(p));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let w = kind.tracked_min_width();
            sender_clone.input(WidgetMenuSettingsInput::MinWidthEffect(w));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let h = kind.tracked_max_height();
            sender_clone.input(WidgetMenuSettingsInput::MaxHeightEffect(h));
        });
        // SystemUpdate-only: track the pill's poll cadence so an
        // external `mshellctl config reload` repaints the spin.
        // Harmless for other kinds — the read just doesn't drive
        // a visible field.
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .system_update()
                .check_interval_minutes()
                .get();
            sender_clone.input(WidgetMenuSettingsInput::CheckIntervalEffect(v));
        });

        // ControlCenter-only: build the Tiles sub-section and append it to
        // the page box after the generic position/size controls.
        let cc_tiles_controller = if kind == MenuKind::ControlCenter {
            Some(
                CcTilesSettingsModel::builder()
                    .launch(CcTilesSettingsInit {})
                    .detach(),
            )
        } else {
            None
        };

        let model = WidgetMenuSettingsModel {
            kind,
            position: kind.read_position(),
            minimum_width: kind.read_min_width(),
            maximum_height: kind.read_max_height(),
            check_interval_minutes: config_manager()
                .config()
                .bars()
                .widgets()
                .system_update()
                .check_interval_minutes()
                .get_untracked(),
            position_model,
            _cc_tiles_controller: cc_tiles_controller,
            _effects: effects,
        };

        let widgets = view_output!();

        // Append the CC tiles section widget to the page box when present.
        if let Some(ctrl) = &model._cc_tiles_controller {
            widgets.page_box.append(ctrl.widget());
        }

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            WidgetMenuSettingsInput::PositionPicked(idx) => {
                let p = Position::from_index(idx);
                if self.position != p {
                    self.position = p.clone();
                    self.kind.write_position(p);
                }
            }
            WidgetMenuSettingsInput::MinWidthChanged(w) => {
                if self.minimum_width != w {
                    self.minimum_width = w;
                    self.kind.write_min_width(w);
                }
            }
            WidgetMenuSettingsInput::MaxHeightChanged(h) => {
                if self.maximum_height != h {
                    self.maximum_height = h;
                    self.kind.write_max_height(h);
                }
            }
            WidgetMenuSettingsInput::CheckIntervalChanged(v) => {
                if self.check_interval_minutes != v {
                    self.check_interval_minutes = v;
                    config_manager().update_config(move |c| {
                        c.bars.widgets.system_update.check_interval_minutes = v;
                    });
                }
            }
            WidgetMenuSettingsInput::PositionEffect(p) => self.position = p,
            WidgetMenuSettingsInput::MinWidthEffect(w) => self.minimum_width = w,
            WidgetMenuSettingsInput::MaxHeightEffect(h) => self.maximum_height = h,
            WidgetMenuSettingsInput::CheckIntervalEffect(v) => self.check_interval_minutes = v,
        }
    }
}
