//! `MenuId`: the uniform menus and their stack names / positions.

use super::*;

// Stack names of every menu (the `GtkStack` child name / config key),
// kept here beside their ids.
pub(super) const CLOCK_MENU: &str = "clock";
pub(super) const CLIPBOARD_MENU: &str = "clipboard";
pub(super) const APP_LAUNCHER_MENU: &str = "app_launcher";
pub(super) const SCREENSHOT_MENU: &str = "screenshot";
pub(super) const NOTIFICATION_MENU: &str = "notification";
pub(super) const WALLPAPER_MENU: &str = "wallpaper";
pub(super) const SCREENSHARE_MENU: &str = "screenshare";
pub(super) const WIZARD_MENU: &str = "wizard";
pub(super) const NUFW_MENU: &str = "ufw";
pub(super) const PRIVACY_MENU: &str = "privacy";
pub(super) const BLUETOOTH_MENU: &str = "bluetooth";
pub(super) const CPU_DASHBOARD_MENU: &str = "cpu_dashboard";
pub(super) const AUDIO_DASHBOARD_MENU: &str = "audio_dashboard";
pub(super) const SYSTEM_UPDATE_MENU: &str = "system_update";
pub(super) const VALENT_MENU: &str = "valent";
pub(super) const WEATHER_MENU: &str = "weather";
pub(super) const KEEP_AWAKE_MENU: &str = "keep_awake";
pub(super) const TWILIGHT_MENU: &str = "twilight";
pub(super) const KEYBINDS_MENU: &str = "keybinds";
pub(super) const ALARMCLOCK_MENU: &str = "alarmclock";
pub(super) const DOCK_MENU: &str = "dock";
pub(super) const CONTROL_CENTER_MENU: &str = "control_center";
pub(super) const SSH_MENU: &str = "ssh_sessions";
pub(super) const USB_MENU: &str = "usb";
pub(super) const DISCORD_MENU: &str = "discord_voice";
pub(super) const CRASHES_MENU: &str = "crashes";
pub(super) const NDNS_MENU: &str = "dns";
pub(super) const NVPN_MENU: &str = "vpn";
pub(super) const NAI_MENU: &str = "ai";
pub(super) const NPODMAN_MENU: &str = "podman";
pub(super) const NNOTES_MENU: &str = "notes";
pub(super) const NPLUGIN_PANEL_MENU: &str = "plugin-panel";
pub(super) const NIP_MENU: &str = "ip";
pub(super) const NVPN_INDICATOR_MENU: &str = "vpn_indicator";
pub(super) const NNETWORK_MENU: &str = "network";
pub(super) const NPOWER_MENU: &str = "power";
pub(super) const MEDIA_PLAYER_MENU: &str = "media_player";
pub(super) const MTUNE_MENU: &str = "mtune";
pub(super) const LYRICS_MENU: &str = "lyrics";
pub(super) const SESSION_MENU: &str = "session";
pub(super) const SETTINGS_MENU: &str = "settings";
pub(super) const MDASH_MENU: &str = "mdash";
pub(super) const MARGO_LAYOUT_MENU: &str = "margo_layout";
pub(super) const TRANSLATE_MENU: &str = "translate";
pub(super) const COLOR_PICKER_MENU: &str = "color_picker";
pub(super) const NAGENTS_MENU: &str = "agents";
pub(super) const NGITHUB_MENU: &str = "github";
pub(super) const NQR_MENU: &str = "qr";

/// The menus that share the uniform open path: toggle the menu's reveal,
/// then re-sync the layer surface's keyboard mode. Each maps to its
/// menu-name key via [`MenuId::menu_name`]. Folding these into the single
/// parametrised [`FrameInput::ToggleMenu`] replaces the ~34 data-less
/// `Toggle*Menu` variants (and their identical match arms) that used to
/// enumerate them one by one. Menus whose open path is *not* uniform —
/// Settings (built on demand), Screenshare (forwards a reply channel), the
/// WASM / declarative plugin panels, and AppLauncher-with-tab — keep their
/// own bespoke `FrameInput` variants below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuId {
    Clock,
    Clipboard,
    Notification,
    Screenshot,
    AppLauncher,
    Wallpaper,
    Wizard,
    Ufw,
    Privacy,
    Bluetooth,
    CpuDashboard,
    AudioDashboard,
    SystemUpdate,
    Valent,
    Weather,
    KeepAwake,
    Twilight,
    Keybinds,
    AlarmClock,
    Dock,
    ControlCenter,
    SshSessions,
    Usb,
    DiscordVoice,
    Crashes,
    Dns,
    Vpn,
    Ai,
    Podman,
    Github,
    Qr,
    Agents,
    Notes,
    Ip,
    VpnIndicator,
    Network,
    Power,
    MediaPlayer,
    Mtune,
    Lyrics,
    Session,
    Mdash,
    MargoLayout,
    Translate,
    ColorPicker,
}

impl MenuId {
    /// Every id, for tests that must cover each menu.
    #[cfg(test)]
    pub(super) const ALL: [MenuId; 44] = [
        MenuId::Clock,
        MenuId::Clipboard,
        MenuId::Notification,
        MenuId::Screenshot,
        MenuId::AppLauncher,
        MenuId::Wallpaper,
        MenuId::Wizard,
        MenuId::Ufw,
        MenuId::Privacy,
        MenuId::Bluetooth,
        MenuId::CpuDashboard,
        MenuId::AudioDashboard,
        MenuId::SystemUpdate,
        MenuId::Valent,
        MenuId::Weather,
        MenuId::KeepAwake,
        MenuId::Twilight,
        MenuId::Keybinds,
        MenuId::AlarmClock,
        MenuId::Dock,
        MenuId::ControlCenter,
        MenuId::SshSessions,
        MenuId::Usb,
        MenuId::DiscordVoice,
        MenuId::Crashes,
        MenuId::Dns,
        MenuId::Vpn,
        MenuId::Ai,
        MenuId::Podman,
        MenuId::Github,
        MenuId::Agents,
        MenuId::Notes,
        MenuId::Ip,
        MenuId::VpnIndicator,
        MenuId::Network,
        MenuId::Power,
        MenuId::MediaPlayer,
        MenuId::Mtune,
        MenuId::Lyrics,
        MenuId::Session,
        MenuId::Mdash,
        MenuId::MargoLayout,
        MenuId::Translate,
        MenuId::ColorPicker,
    ];

    /// The stable menu-name key this id toggles — the same `&str` the
    /// per-menu arms passed to [`Frame::toggle_menu`] before the collapse.
    pub(super) const fn menu_name(self) -> &'static str {
        match self {
            MenuId::Clock => CLOCK_MENU,
            MenuId::Clipboard => CLIPBOARD_MENU,
            MenuId::Notification => NOTIFICATION_MENU,
            MenuId::Screenshot => SCREENSHOT_MENU,
            MenuId::AppLauncher => APP_LAUNCHER_MENU,
            MenuId::Wallpaper => WALLPAPER_MENU,
            MenuId::Wizard => WIZARD_MENU,
            MenuId::Ufw => NUFW_MENU,
            MenuId::Privacy => PRIVACY_MENU,
            MenuId::Bluetooth => BLUETOOTH_MENU,
            MenuId::CpuDashboard => CPU_DASHBOARD_MENU,
            MenuId::AudioDashboard => AUDIO_DASHBOARD_MENU,
            MenuId::SystemUpdate => SYSTEM_UPDATE_MENU,
            MenuId::Valent => VALENT_MENU,
            MenuId::Weather => WEATHER_MENU,
            MenuId::KeepAwake => KEEP_AWAKE_MENU,
            MenuId::Twilight => TWILIGHT_MENU,
            MenuId::Keybinds => KEYBINDS_MENU,
            MenuId::AlarmClock => ALARMCLOCK_MENU,
            MenuId::Dock => DOCK_MENU,
            MenuId::ControlCenter => CONTROL_CENTER_MENU,
            MenuId::SshSessions => SSH_MENU,
            MenuId::Usb => USB_MENU,
            MenuId::DiscordVoice => DISCORD_MENU,
            MenuId::Crashes => CRASHES_MENU,
            MenuId::Dns => NDNS_MENU,
            MenuId::Vpn => NVPN_MENU,
            MenuId::Ai => NAI_MENU,
            MenuId::Podman => NPODMAN_MENU,
            MenuId::Github => NGITHUB_MENU,
            MenuId::Qr => NQR_MENU,
            MenuId::Agents => NAGENTS_MENU,
            MenuId::Notes => NNOTES_MENU,
            MenuId::Ip => NIP_MENU,
            MenuId::VpnIndicator => NVPN_INDICATOR_MENU,
            MenuId::Network => NNETWORK_MENU,
            MenuId::Power => NPOWER_MENU,
            MenuId::MediaPlayer => MEDIA_PLAYER_MENU,
            MenuId::Mtune => MTUNE_MENU,
            MenuId::Lyrics => LYRICS_MENU,
            MenuId::Session => SESSION_MENU,
            MenuId::Mdash => MDASH_MENU,
            MenuId::MargoLayout => MARGO_LAYOUT_MENU,
            MenuId::Translate => TRANSLATE_MENU,
            MenuId::ColorPicker => COLOR_PICKER_MENU,
        }
    }

    /// The [`MenuType`] this id builds — the argument the per-menu `init`
    /// lines passed to [`Frame::build_menu`] before the collapse, now consumed
    /// on demand by [`Frame::ensure_menu_built`].
    pub(super) fn menu_type(self) -> MenuType {
        match self {
            MenuId::Clock => MenuType::Clock,
            MenuId::Clipboard => MenuType::Clipboard,
            MenuId::Notification => MenuType::Notifications,
            MenuId::Screenshot => MenuType::Screenshot,
            MenuId::AppLauncher => MenuType::AppLauncher,
            MenuId::Wallpaper => MenuType::Wallpaper,
            MenuId::Wizard => MenuType::Wizard,
            MenuId::Ufw => MenuType::Ufw,
            MenuId::Privacy => MenuType::Privacy,
            MenuId::Bluetooth => MenuType::Bluetooth,
            MenuId::CpuDashboard => MenuType::CpuDashboard,
            MenuId::AudioDashboard => MenuType::AudioDashboard,
            MenuId::SystemUpdate => MenuType::SystemUpdate,
            MenuId::Valent => MenuType::Valent,
            MenuId::Weather => MenuType::Weather,
            MenuId::KeepAwake => MenuType::KeepAwake,
            MenuId::Twilight => MenuType::Twilight,
            MenuId::Keybinds => MenuType::Keybinds,
            MenuId::AlarmClock => MenuType::AlarmClock,
            MenuId::Dock => MenuType::Dock,
            MenuId::ControlCenter => MenuType::ControlCenter,
            MenuId::SshSessions => MenuType::SshSessions,
            MenuId::Usb => MenuType::Usb,
            MenuId::DiscordVoice => MenuType::DiscordVoice,
            MenuId::Crashes => MenuType::Crashes,
            MenuId::Dns => MenuType::Dns,
            MenuId::Vpn => MenuType::Vpn,
            MenuId::Ai => MenuType::Ai,
            MenuId::Podman => MenuType::Podman,
            MenuId::Github => MenuType::Github,
            MenuId::Qr => MenuType::Qr,
            MenuId::Agents => MenuType::Agents,
            MenuId::Notes => MenuType::Notes,
            MenuId::Ip => MenuType::Ip,
            MenuId::VpnIndicator => MenuType::VpnIndicator,
            MenuId::Network => MenuType::Network,
            MenuId::Power => MenuType::Power,
            MenuId::MediaPlayer => MenuType::MediaPlayer,
            MenuId::Mtune => MenuType::Mtune,
            MenuId::Lyrics => MenuType::Lyrics,
            MenuId::Session => MenuType::Session,
            MenuId::Mdash => MenuType::Mdash,
            MenuId::MargoLayout => MenuType::MargoLayout,
            MenuId::Translate => MenuType::Translate,
            MenuId::ColorPicker => MenuType::ColorPicker,
        }
    }
}
