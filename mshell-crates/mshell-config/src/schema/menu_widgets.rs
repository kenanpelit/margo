use crate::schema::position::Orientation;
use reactive_stores::{KeyMap, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum MenuWidget {
    AppLauncher,
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
    Divider,
    /// Margo layout switcher — a vertical list of the 14 layouts
    /// the compositor knows about (tile / scroller / grid /
    /// monocle / deck / dwindle / etc.) with the currently-active
    /// row highlighted. Lives in the LayoutMenu surface so it
    /// opens contiguous with the bar instead of as a separate
    /// xdg_popup window.
    MargoLayout,
    MediaPlayer,
    Ndns,
    Network,
    Nip,
    Nnetwork,
    Nnotes,
    Notifications,
    Npodman,
    Npower,
    /// Dashboard "what's happening now" summary card — pulls
    /// notification count, low-battery state, and CPU thermals
    /// into one glanceable list. Lives at the top of the
    /// dashboard's left column.
    OverviewIntel,
    Nufw,
    PowerProfiles,
    QuickActions(QuickActionsConfig),
    Session,
    Screenshots,
    ScreenRecording,
    Spacer(SpacerConfig),
    /// Combined system-health tile — active power profile, battery
    /// %, and CPU package temperature in one compact card. Lives
    /// in the dashboard's right column where each of the three
    /// metrics was previously its own widget.
    SystemStatus,
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
            MenuWidget::AppLauncher => "App Launcher",
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
            MenuWidget::Divider => "Divider",
            MenuWidget::MargoLayout => "Margo Layout",
            MenuWidget::MediaPlayer => "Media Player",
            MenuWidget::Ndns => "DNS / VPN",
            MenuWidget::Network => "Network",
            MenuWidget::Nip => "Public IP",
            MenuWidget::Nnetwork => "Network Console",
            MenuWidget::Nnotes => "Notes Hub",
            MenuWidget::Notifications => "Notifications",
            MenuWidget::Npodman => "Podman",
            MenuWidget::Npower => "Power Profile Menu",
            MenuWidget::OverviewIntel => "Overview Intelligence",
            MenuWidget::Nufw => "UFW Firewall",
            MenuWidget::PowerProfiles => "Power Profiles",
            MenuWidget::QuickActions(_) => "Quick Actions",
            MenuWidget::Session => "Session",
            MenuWidget::Screenshots => "Screenshots",
            MenuWidget::ScreenRecording => "Screen Recording",
            MenuWidget::Spacer(_) => "Spacer",
            MenuWidget::SystemStatus => "System Status",
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
            MenuWidget::AppLauncher,
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
            MenuWidget::Divider,
            MenuWidget::MargoLayout,
            MenuWidget::MediaPlayer,
            MenuWidget::Ndns,
            MenuWidget::Network,
            MenuWidget::Nip,
            MenuWidget::Nnetwork,
            MenuWidget::Nnotes,
            MenuWidget::Notifications,
            MenuWidget::Npodman,
            MenuWidget::Npower,
            MenuWidget::OverviewIntel,
            MenuWidget::Nufw,
            MenuWidget::PowerProfiles,
            MenuWidget::QuickActions(QuickActionsConfig::default()),
            MenuWidget::Session,
            MenuWidget::Screenshots,
            MenuWidget::ScreenRecording,
            MenuWidget::Spacer(SpacerConfig { size: 16 }),
            MenuWidget::SystemStatus,
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
    Settings,
    Shutdown,
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
            QuickActionWidget::Settings => "Settings",
            QuickActionWidget::Shutdown => "Shutdown",
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
            QuickActionWidget::Settings,
            QuickActionWidget::Shutdown,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub struct SpacerConfig {
    pub size: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub struct ContainerConfig {
    pub widgets: Vec<MenuWidget>,
    pub spacing: i32,
    pub orientation: Orientation,
    pub minimum_width: i32,
}

impl Default for ContainerConfig {
    fn default() -> Self {
        Self {
            widgets: Vec::new(),
            spacing: 0,
            orientation: Orientation::Horizontal,
            minimum_width: 0,
        }
    }
}
