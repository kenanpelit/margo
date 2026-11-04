//! `MenuId`: the uniform menus and their stack names / positions.

use super::*;

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
    Dns,
    Vpn,
    Ai,
    Podman,
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
}

impl MenuId {
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
            MenuId::Dns => NDNS_MENU,
            MenuId::Vpn => NVPN_MENU,
            MenuId::Ai => NAI_MENU,
            MenuId::Podman => NPODMAN_MENU,
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
            MenuId::Dns => MenuType::Dns,
            MenuId::Vpn => MenuType::Vpn,
            MenuId::Ai => MenuType::Ai,
            MenuId::Podman => MenuType::Podman,
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
        }
    }
}
