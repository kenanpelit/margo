use crate::schema::position::Orientation;
use reactive_stores::{KeyMap, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum MenuWidget {
    /// Alarm Clock menu — tabbed Alarms + Stopwatch panel. The menu
    /// content for the `alarm_clock` bar pill: an alarms list with
    /// per-alarm enable / time / repeat-day chips / delete, an
    /// add/edit row, and a stopwatch with start / pause / reset.
    AlarmClock,
    /// Control Center menu — system preferences and quick-access
    /// controls panel. The menu content for the `control_center`
    /// bar pill.
    ControlCenter,
    AppLauncher,
    /// Audio Dashboard menu — output + input mute / slider /
    /// device-picker card stack. The menu content for the
    /// `audio_dashboard` bar pill.
    AudioDashboard,
    AudioInput,
    AudioOutput,
    Bluetooth,
    Calendar,
    /// Month grid only — the same `gtk::Calendar` half of the
    /// `Calendar` widget without the primary-tinted hero band on
    /// top. Used by the dashboard menu where the hero role is
    /// already filled by the `Clock` widget; pairing the full
    /// `Calendar` here would show two clocks.
    CalendarGrid,
    Clipboard,
    Clock,
    /// Compact two-row Volume + Mic slider tile. Drops the
    /// revealer-row chrome the standalone AudioOutput +
    /// AudioInput widgets carry, surfacing just the sliders +
    /// percentages in a single card.
    CompactAudio,
    /// Compact horizontal WiFi + Bluetooth status row. Replaces
    /// the stacked Network + Bluetooth widget pair when the
    /// dashboard wants a tighter "connectivity at a glance" view.
    Connectivity,
    Container(ContainerConfig),
    /// CPU Dashboard menu — hero (CPU% + temp), per-core bars,
    /// RAM bar, load-avg footer. The menu content for the
    /// `cpu_dashboard` bar pill.
    CpuDashboard,
    Divider,
    /// Margo layout switcher — a vertical list of the 14 layouts
    /// the compositor knows about (tile / scroller / grid /
    /// monocle / deck / dwindle / etc.) with the currently-active
    /// row highlighted. Lives in the LayoutMenu surface so it
    /// opens contiguous with the bar instead of as a separate
    /// xdg_popup window.
    MargoLayout,
    /// mdock as a Frame menu — the pinned/running app strip rendered inside the
    /// bar's frame (the "layer-shell", bar-attached mdock style). Toggled by
    /// `mshellctl dock`.
    MargoDock,
    MediaPlayer,
    /// Lyrics menu content — the scrolling synced-lyrics column.
    Lyrics,
    Dns,
    /// VPN menu — the full Mullvad control surface the `mvpn` bar pill
    /// opens: status hero, connect / random / fastest, lockdown /
    /// auto-connect / quantum-resistant toggles, anti-censorship mode,
    /// and the favourites list. Shells out to the `mvpn` binary.
    Vpn,
    /// AI assistant menu — a streaming multi-provider chat panel. Config
    /// (provider / model / key) lives in Settings → AI.
    Ai,
    NetworkToggle,
    Ip,
    /// Generic-VPN detail menu — one card per active OpenVPN / WireGuard
    /// tunnel (Mullvad excluded) with type, local tunnel IP(s), and live
    /// RX/TX throughput. Opened by the `vpn_indicator` bar pill.
    VpnIndicator,
    Network,
    Notes,
    Notifications,
    Podman,
    Power,
    /// `privacy` bar pill's panel — live "in use now" rows (mic /
    /// camera / screen-share + which apps) plus a clearable access
    /// log of recent started/stopped events.
    Privacy,
    /// Dashboard "what's happening now" summary card — pulls
    /// notification count, low-battery state, and CPU thermals
    /// into one glanceable list. Lives at the top of the
    /// dashboard's left column.
    OverviewIntel,
    Ufw,
    QuickActions(QuickActionsConfig),
    Session,
    Screenshots,
    ScreenRecording,
    Spacer(SpacerConfig),
    /// Reusable §12 panel header — leading icon + a SemiBold display
    /// title + a live date + a circular settings gear. Sits at the
    /// head of a panel (the dashboard uses it in place of the Clock
    /// hero); any future panel can reuse it.
    PanelHeader(PanelHeaderConfig),
    /// Combined system-health tile — active power profile, battery
    /// %, and CPU package temperature in one compact card. Lives
    /// in the dashboard's right column where each of the three
    /// metrics was previously its own widget.
    SystemStatus,
    /// `system_update` bar pill's panel — pending updates grouped by
    /// source (repo / AUR / Flatpak) with Refresh + Update.
    SystemUpdate,
    /// `valent` bar pill's panel — paired phone status (battery,
    /// connectivity) + find / ping / browse / share / pair actions.
    Valent,
    /// `keep_awake` bar pill's panel — duration grid (30m / 1h / …
    /// / ∞), live countdown, quick-extend, turn-off.
    KeepAwake,
    /// `twilight` bar pill's panel — master toggle, current temp /
    /// phase, mode selector, temperature slider, schedule presets.
    Twilight,
    /// `keybinds` bar pill's panel — searchable cheatsheet of every
    /// shortcut parsed live from margo's `config.conf`, grouped by
    /// action category.
    Keybinds,
    /// `ssh_sessions` bar pill's panel — searchable host list parsed
    /// from `~/.ssh/config`, active connections first, click to
    /// connect in a new terminal.
    SshSessions,
    ThemePicker,
    Wallpaper,
    Weather,
}

impl PatchField for MenuWidget {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

impl MenuWidget {
    pub fn display_name(&self) -> &'static str {
        match self {
            MenuWidget::AlarmClock => "Alarm Clock",
            MenuWidget::ControlCenter => "Control Center",
            MenuWidget::AppLauncher => "App Launcher",
            MenuWidget::AudioDashboard => "Audio Dashboard",
            MenuWidget::AudioInput => "Audio Input",
            MenuWidget::AudioOutput => "Audio Output",
            MenuWidget::Bluetooth => "Bluetooth",
            MenuWidget::Calendar => "Calendar",
            MenuWidget::CalendarGrid => "Calendar Grid",
            MenuWidget::Clipboard => "Clipboard",
            MenuWidget::Clock => "Clock",
            MenuWidget::CompactAudio => "Compact Audio",
            MenuWidget::Connectivity => "Connectivity",
            MenuWidget::Container(_) => "Container",
            MenuWidget::CpuDashboard => "CPU Dashboard",
            MenuWidget::Divider => "Divider",
            MenuWidget::MargoLayout => "Margo Layout",
            MenuWidget::MargoDock => "mdock",
            MenuWidget::MediaPlayer => "Media Player",
            MenuWidget::Lyrics => "Lyrics",
            MenuWidget::Dns => "DNS / VPN",
            MenuWidget::Vpn => "VPN",
            MenuWidget::Ai => "AI Assistant",
            MenuWidget::NetworkToggle => "Network",
            MenuWidget::Ip => "Public IP",
            MenuWidget::VpnIndicator => "VPN Indicator",
            MenuWidget::Network => "Network Console",
            MenuWidget::Notes => "Notes Hub",
            MenuWidget::Notifications => "Notifications",
            MenuWidget::Podman => "Podman",
            MenuWidget::Power => "Power Profile",
            MenuWidget::Privacy => "Privacy",
            MenuWidget::OverviewIntel => "Overview Intelligence",
            MenuWidget::Ufw => "UFW Firewall",
            MenuWidget::QuickActions(_) => "Quick Actions",
            MenuWidget::Session => "Session",
            MenuWidget::Screenshots => "Screenshots",
            MenuWidget::ScreenRecording => "Screen Recording",
            MenuWidget::Spacer(_) => "Spacer",
            MenuWidget::PanelHeader(_) => "Panel Header",
            MenuWidget::SystemStatus => "System Status",
            MenuWidget::SystemUpdate => "System Updates",
            MenuWidget::Valent => "Valent Connect",
            MenuWidget::KeepAwake => "Keep Awake",
            MenuWidget::Twilight => "Twilight",
            MenuWidget::Keybinds => "Keyboard Shortcuts",
            MenuWidget::SshSessions => "SSH Sessions",
            MenuWidget::ThemePicker => "Theme Picker",
            MenuWidget::Wallpaper => "Wallpaper",
            MenuWidget::Weather => "Weather",
        }
    }

    pub fn action_name(&self) -> String {
        // GAction names only accept `[A-Za-z0-9._-]`; map every
        // other char (spaces, `/`, parens…) to `-` so detailed
        // action strings like `menuwidget.<name>` stay parseable.
        self.display_name()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }

    /// Returns all widget types with default configs
    pub fn all_defaults() -> Vec<MenuWidget> {
        vec![
            MenuWidget::AlarmClock,
            MenuWidget::ControlCenter,
            MenuWidget::AppLauncher,
            MenuWidget::AudioDashboard,
            MenuWidget::AudioInput,
            MenuWidget::AudioOutput,
            MenuWidget::Bluetooth,
            MenuWidget::Calendar,
            MenuWidget::CalendarGrid,
            MenuWidget::Clipboard,
            MenuWidget::Clock,
            MenuWidget::CompactAudio,
            MenuWidget::Connectivity,
            MenuWidget::Container(ContainerConfig::default()),
            MenuWidget::CpuDashboard,
            MenuWidget::Divider,
            MenuWidget::MargoLayout,
            MenuWidget::MediaPlayer,
            MenuWidget::Lyrics,
            MenuWidget::Dns,
            MenuWidget::Vpn,
            MenuWidget::Ai,
            MenuWidget::NetworkToggle,
            MenuWidget::Ip,
            MenuWidget::VpnIndicator,
            MenuWidget::Network,
            MenuWidget::Notes,
            MenuWidget::Notifications,
            MenuWidget::Podman,
            MenuWidget::Power,
            MenuWidget::Privacy,
            MenuWidget::OverviewIntel,
            MenuWidget::Ufw,
            MenuWidget::QuickActions(QuickActionsConfig::default()),
            MenuWidget::Session,
            MenuWidget::Screenshots,
            MenuWidget::ScreenRecording,
            MenuWidget::Spacer(SpacerConfig { size: 16 }),
            MenuWidget::PanelHeader(PanelHeaderConfig::default()),
            MenuWidget::SystemStatus,
            MenuWidget::SystemUpdate,
            MenuWidget::Valent,
            MenuWidget::KeepAwake,
            MenuWidget::Twilight,
            MenuWidget::Keybinds,
            MenuWidget::SshSessions,
            MenuWidget::ThemePicker,
            MenuWidget::Wallpaper,
            MenuWidget::Weather,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema, Default)]
pub struct QuickActionsConfig {
    pub widgets: Vec<QuickActionWidget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum QuickActionWidget {
    AirplaneMode,
    DoNotDisturb,
    ColorPicker,
    IdleInhibitor,
    Lock,
    Logout,
    Nightlight,
    Reboot,
    Wallpaper,
    Screenshot,
    Settings,
    Shutdown,
    // ── Menu-launcher buttons ──────────────────────────────────────
    // Each opens another shell menu (`mshellctl menu <name>`) and closes
    // the dashboard it lives in. Used by mdash's bottom shortcut grid.
    Network,
    Bluetooth,
    CpuDashboard,
    AudioDashboard,
    Vpn,
    ControlCenter,
    Twilight,
    Keybinds,
    Dns,
    Power,
    Session,
    Ip,
    AlarmClock,
    SystemUpdate,
}

impl QuickActionWidget {
    pub fn display_name(&self) -> &'static str {
        match self {
            QuickActionWidget::AirplaneMode => "Airplane Mode",
            QuickActionWidget::DoNotDisturb => "Do Not Disturb",
            QuickActionWidget::ColorPicker => "Color Picker",
            QuickActionWidget::IdleInhibitor => "Idle Inhibitor",
            QuickActionWidget::Lock => "Lock",
            QuickActionWidget::Logout => "Logout",
            QuickActionWidget::Nightlight => "Night Light",
            QuickActionWidget::Reboot => "Reboot",
            QuickActionWidget::Wallpaper => "Wallpaper",
            QuickActionWidget::Screenshot => "Screenshot",
            QuickActionWidget::Settings => "Settings",
            QuickActionWidget::Shutdown => "Shutdown",
            QuickActionWidget::Network => "Network",
            QuickActionWidget::Bluetooth => "Bluetooth",
            QuickActionWidget::CpuDashboard => "CPU Dashboard",
            QuickActionWidget::AudioDashboard => "Audio Dashboard",
            QuickActionWidget::Vpn => "VPN",
            QuickActionWidget::ControlCenter => "Control Center",
            QuickActionWidget::Twilight => "Twilight",
            QuickActionWidget::Keybinds => "Keybinds",
            QuickActionWidget::Dns => "DNS",
            QuickActionWidget::Power => "Power",
            QuickActionWidget::Session => "Session",
            QuickActionWidget::Ip => "IP",
            QuickActionWidget::AlarmClock => "Alarm Clock",
            QuickActionWidget::SystemUpdate => "System Update",
        }
    }

    pub fn action_name(&self) -> String {
        format!("{:?}", self).to_lowercase()
    }

    pub fn all() -> &'static [QuickActionWidget] {
        &[
            QuickActionWidget::AirplaneMode,
            QuickActionWidget::DoNotDisturb,
            QuickActionWidget::ColorPicker,
            QuickActionWidget::IdleInhibitor,
            QuickActionWidget::Lock,
            QuickActionWidget::Logout,
            QuickActionWidget::Nightlight,
            QuickActionWidget::Reboot,
            QuickActionWidget::Wallpaper,
            QuickActionWidget::Screenshot,
            QuickActionWidget::Settings,
            QuickActionWidget::Shutdown,
            QuickActionWidget::Network,
            QuickActionWidget::Bluetooth,
            QuickActionWidget::CpuDashboard,
            QuickActionWidget::AudioDashboard,
            QuickActionWidget::Vpn,
            QuickActionWidget::ControlCenter,
            QuickActionWidget::Twilight,
            QuickActionWidget::Keybinds,
            QuickActionWidget::Dns,
            QuickActionWidget::Power,
            QuickActionWidget::Session,
            QuickActionWidget::Ip,
            QuickActionWidget::AlarmClock,
            QuickActionWidget::SystemUpdate,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub struct SpacerConfig {
    pub size: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub struct PanelHeaderConfig {
    /// Display-size panel title (e.g. "Dashboard"), shown SemiBold at
    /// the head of the panel beside a live date + a settings gear
    /// (DESIGN.md §12 header). A field so the header is reusable.
    /// Ignored when `greeting` is on (the greeting replaces the title).
    #[serde(default = "default_panel_title")]
    pub title: String,
    /// When true the title is replaced by a time-aware greeting
    /// ("Good morning/afternoon/evening, <user>") that updates as the
    /// day rolls over. Used by `mdash`; defaults off so a plain
    /// `PanelHeader` keeps its static title.
    #[serde(default)]
    pub greeting: bool,
}

fn default_panel_title() -> String {
    "Dashboard".to_string()
}

impl Default for PanelHeaderConfig {
    fn default() -> Self {
        Self {
            title: default_panel_title(),
            greeting: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub struct ContainerConfig {
    pub widgets: Vec<MenuWidget>,
    pub spacing: i32,
    pub orientation: Orientation,
    pub minimum_width: i32,
    /// Force every child to the same size along the orientation
    /// axis. Used by the dashboard's 2-column body so the left and
    /// right panes get identical widths regardless of which side's
    /// content is naturally wider. Default-on-missing so older YAML
    /// still parses.
    #[serde(default)]
    pub homogeneous: bool,
    /// Make the LAST child stretch to fill the container's
    /// remaining space (children above keep natural sizes, stacked
    /// from the top). Used by the dashboard columns so the bottom
    /// anchor card (Weather / MediaPlayer) grows to fill the column
    /// — and since both columns share the same total height, the
    /// two bottom cards end up the same size. Default-off.
    #[serde(default)]
    pub fill: bool,
}

impl Default for ContainerConfig {
    fn default() -> Self {
        Self {
            widgets: Vec::new(),
            spacing: 0,
            orientation: Orientation::Horizontal,
            minimum_width: 0,
            homogeneous: false,
            fill: false,
        }
    }
}
