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
    Container(ContainerConfig),
    Divider,
    MediaPlayer,
    Ndns,
    Network,
    Nip,
    Nnetwork,
    Nnotes,
    Notifications,
    Npodman,
    Npower,
    Nufw,
    PowerProfiles,
    QuickActions(QuickActionsConfig),
    Session,
    Screenshots,
    ScreenRecording,
    Spacer(SpacerConfig),
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
            MenuWidget::Container(_) => "Container",
            MenuWidget::Divider => "Divider",
            MenuWidget::MediaPlayer => "Media Player",
            MenuWidget::Ndns => "DNS / VPN",
            MenuWidget::Network => "Network",
            MenuWidget::Nip => "Public IP",
            MenuWidget::Nnetwork => "Network Console",
            MenuWidget::Nnotes => "Notes Hub",
            MenuWidget::Notifications => "Notifications",
            MenuWidget::Npodman => "Podman",
            MenuWidget::Npower => "Power Profile Menu",
            MenuWidget::Nufw => "UFW Firewall",
            MenuWidget::PowerProfiles => "Power Profiles",
            MenuWidget::QuickActions(_) => "Quick Actions",
            MenuWidget::Session => "Session",
            MenuWidget::Screenshots => "Screenshots",
            MenuWidget::ScreenRecording => "Screen Recording",
            MenuWidget::Spacer(_) => "Spacer",
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
            MenuWidget::Container(ContainerConfig::default()),
            MenuWidget::Divider,
            MenuWidget::MediaPlayer,
            MenuWidget::Ndns,
            MenuWidget::Network,
            MenuWidget::Nip,
            MenuWidget::Nnetwork,
            MenuWidget::Nnotes,
            MenuWidget::Notifications,
            MenuWidget::Npodman,
            MenuWidget::Npower,
            MenuWidget::Nufw,
            MenuWidget::PowerProfiles,
            MenuWidget::QuickActions(QuickActionsConfig::default()),
            MenuWidget::Session,
            MenuWidget::Screenshots,
            MenuWidget::ScreenRecording,
            MenuWidget::Spacer(SpacerConfig { size: 16 }),
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
    HyprPicker,
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
            QuickActionWidget::HyprPicker => "Color Picker",
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
            QuickActionWidget::HyprPicker,
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
