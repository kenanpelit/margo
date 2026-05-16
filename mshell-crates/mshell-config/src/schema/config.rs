use crate::schema::bar_widgets::BarWidget;
use crate::schema::content_fit::ContentFit;
use crate::schema::location_query::{LocationQueryConfig, OrdF64};
use crate::schema::menu_widgets::{
    ContainerConfig, MenuWidget, QuickActionWidget, QuickActionsConfig, SpacerConfig,
};
use crate::schema::position::{NotificationPosition, Orientation, Position};
use crate::schema::quick_settings_icon::QuickSettingsIcon;
use crate::schema::temperature::TemperatureUnitConfig;
use crate::schema::themes::{
    MatugenContrast, MatugenMode, MatugenPreference, MatugenType, Themes, WindowOpacity,
};
use crate::schema::wallpaper::{ContrastFilterStrength, ThemeFilterStrength};
use reactive_stores::{KeyMap, Patch, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    pub general: General,
    pub theme: Theme,
    pub bars: Bars,
    pub menus: Menus,
    pub notifications: Notifications,
    pub wallpaper: Wallpaper,
    pub tempo: Tempo,
    pub idle: Idle,
    pub session: Session,
}

/// Idle manager — staged actions as the session sits idle. Each
/// stage has an enable flag and a timeout (minutes from the last
/// input). Timeouts are independent, so they should be ordered
/// dim < lock < suspend for the staging to read naturally.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Idle {
    /// Dim the screen (a translucent overlay) when idle.
    pub dim_enabled: bool,
    pub dim_timeout_minutes: u32,
    /// Activate the lock screen when idle.
    pub lock_enabled: bool,
    pub lock_timeout_minutes: u32,
    /// `systemctl suspend` when idle.
    pub suspend_enabled: bool,
    pub suspend_timeout_minutes: u32,
}

impl Default for Idle {
    fn default() -> Self {
        Self {
            dim_enabled: true,
            dim_timeout_minutes: 15,
            lock_enabled: true,
            lock_timeout_minutes: 20,
            suspend_enabled: true,
            suspend_timeout_minutes: 30,
        }
    }
}

/// Session actions (the power menu). Each field is the command
/// run for that action; an empty string falls back to the
/// built-in default (`systemctl …` / the in-process lock). Set
/// e.g. `reboot_command = "osc-safe-reboot"` to route the button
/// through your own script. Non-empty commands run via `sh -c`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Session {
    pub lock_command: String,
    pub logout_command: String,
    pub suspend_command: String,
    pub reboot_command: String,
    pub shutdown_command: String,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            lock_command: String::new(),
            logout_command: String::new(),
            suspend_command: String::new(),
            reboot_command: String::new(),
            shutdown_command: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct General {
    pub clock_format_24_h: bool,
    pub weather_location_query: LocationQueryConfig,
    pub temperature_unit: TemperatureUnitConfig,
    /// Draw rounded screen corners as a per-monitor overlay.
    /// Layer-shell windows masked at each corner so the
    /// underlying compositor's rectangular monitor edges read
    /// as soft corners. Click-through; no input region.
    ///
    /// **Off by default** — the frame's own `border-radius`
    /// (CSS variable, 24 px default) already rounds the bar's
    /// outer corners, so for most setups the extra overlay
    /// duplicates the curve at a different radius and produces
    /// a visible step where the two arcs meet. Turn this on
    /// (and set `screen_corner_radius` to match your frame
    /// border-radius, 24 px by default) only when your
    /// compositor doesn't already paint rounded corners.
    pub show_screen_corners: bool,
    /// Corner radius in pixels for the screen-corners overlay.
    /// Ignored when `show_screen_corners = false`. To avoid a
    /// visible step against the bar's own rounded corner, set
    /// this equal to the frame's `border-radius` (CSS variable
    /// `--frame-border-radius`, default `24`).
    pub screen_corner_radius: u32,
    /// Show a brief OSD when the network state changes —
    /// "Connected: <SSID>", "Disconnected", "Ethernet connected"
    /// etc. Fires only on transitions (no popup if the state
    /// hasn't changed since the last tick), so a flaky link
    /// won't spam the screen.
    ///
    /// **Off by default** because NetworkManager often surfaces
    /// the same information via desktop notifications; users on
    /// systems without NM (or who'd rather see the popup) can
    /// turn this on in Settings → General.
    pub network_osd_enabled: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            clock_format_24_h: false,
            weather_location_query: LocationQueryConfig::Coordinates {
                lat: OrdF64(0.0),
                lon: OrdF64(0.0),
            },
            temperature_unit: TemperatureUnitConfig::Metric,
            show_screen_corners: false,
            screen_corner_radius: 24,
            network_osd_enabled: false,
        }
    }
}

/// Clock-bar-widget formatting.
///
/// `clock_format` is the *initial* strftime-style string shown after
/// mshell start (chrono-format syntax: `%H:%M`, `%a %d %b %H:%M`, …).
/// `formats` is the rotating list a double-click cycles through —
/// each click bumps the index, wrap-arounds at the end. Cycling
/// state lives in-memory only, so on the next restart the widget
/// shows whatever `clock_format` says again. Leaving `formats` empty
/// disables the cycle (the widget shows `clock_format` always).
///
/// Kept in a dedicated `[tempo]` section so future clock-related
/// knobs (chime sounds, alt timezones, calendar popover toggles)
/// have a stable home; the existing `general.clock_format_24_h` flag
/// is left untouched for back-compat — it still picks 12 / 24 h when
/// `clock_format` is empty.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Tempo {
    pub clock_format: String,
    pub formats: Vec<String>,
}

impl Default for Tempo {
    fn default() -> Self {
        Self {
            clock_format: "%a %d %b %H:%M".to_string(),
            formats: vec![
                "%H:%M".to_string(),
                "%H:%M:%S".to_string(),
                "%a %d %b %H:%M".to_string(),
                "%d.%m.%Y %H:%M".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Theme {
    pub icons: Icons,
    pub theme: Themes,
    pub matugen: Matugen,
    pub css_file: String,
    pub attributes: ThemeAttributes,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            icons: Icons::default(),
            theme: Themes::Margo,
            matugen: Matugen::default(),
            css_file: String::new(),
            attributes: ThemeAttributes::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Icons {
    pub shell_icon_theme: String,
    pub app_icon_theme: String,
    pub apply_theme_filter: bool,
    pub filter_strength: ThemeFilterStrength,
    pub monochrome_strength: ThemeFilterStrength,
    pub contrast_strength: ContrastFilterStrength,
}

impl Default for Icons {
    fn default() -> Self {
        Self {
            shell_icon_theme: "MargoMaterial".to_string(),
            app_icon_theme: "MargoMaterial".to_string(),
            apply_theme_filter: false,
            filter_strength: ThemeFilterStrength::new(1.0),
            monochrome_strength: ThemeFilterStrength::new(0.0),
            contrast_strength: ContrastFilterStrength::new(1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct ThemeAttributes {
    pub font: Font,
    pub sizing: Sizing,
    pub window_opacity: WindowOpacity,
}

impl Default for ThemeAttributes {
    fn default() -> Self {
        Self {
            window_opacity: WindowOpacity::new(1.0),
            font: Font::default(),
            sizing: Sizing::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct Font {
    pub primary: String,
    pub secondary: String,
    pub tertiary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Sizing {
    pub radius_widget: i32,
    pub radius_window: i32,
    pub border_width: i32,
}

impl Default for Sizing {
    fn default() -> Self {
        Self {
            radius_widget: 8,
            radius_window: 8,
            border_width: 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Matugen {
    pub preference: MatugenPreference,
    pub scheme_type: MatugenType,
    pub mode: MatugenMode,
    pub contrast: MatugenContrast,
}

impl Default for Matugen {
    fn default() -> Self {
        Self {
            preference: MatugenPreference::Darkness,
            scheme_type: MatugenType::TonalSpot,
            mode: MatugenMode::Dark,
            contrast: MatugenContrast::new(0.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
/// Margo's mshell ships only horizontal bars — vertical Left /
/// Right bar surfaces were removed because they conflict with the
/// scroller-default column flow. The default top-bar layout below
/// is the OkShell upstream's default left-bar layout migrated onto
/// a horizontal axis:
///   * `top_widgets`    → `left_widgets`   (start cluster)
///   * `center_widgets` → `center_widgets` (middle)
///   * `bottom_widgets` → `right_widgets`  (end cluster)
/// Bottom bar starts empty; users add their own widgets via the
/// settings UI / YAML.
pub struct Bars {
    pub frame: Frame,
    pub widgets: BarWidgets,
    pub top_bar: HorizontalBar,
    pub bottom_bar: HorizontalBar,
}

impl Default for Bars {
    fn default() -> Self {
        Self {
            frame: Frame::default(),
            widgets: BarWidgets::default(),
            top_bar: HorizontalBar {
                minimum_height: 0,
                reveal_by_default: true,
                left_widgets: vec![BarWidget::QuickSettings, BarWidget::MargoTags],
                center_widgets: vec![BarWidget::MargoDock],
                right_widgets: vec![
                    BarWidget::RecordingIndicator,
                    BarWidget::Tray,
                    BarWidget::Screenshot,
                    BarWidget::Wallpaper,
                    BarWidget::Clipboard,
                    BarWidget::Notifications,
                    BarWidget::AudioInput,
                    BarWidget::AudioOutput,
                    BarWidget::Bluetooth,
                    BarWidget::Network,
                    BarWidget::Battery,
                    BarWidget::Clock,
                ],
            },
            bottom_bar: HorizontalBar::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct BarWidgets {
    pub quick_settings: QuickSettingsBarWidget,
    pub system_update: SystemUpdateBarWidget,
}

impl Default for BarWidgets {
    fn default() -> Self {
        Self {
            quick_settings: QuickSettingsBarWidget::default(),
            system_update: SystemUpdateBarWidget::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct QuickSettingsBarWidget {
    pub icon: QuickSettingsIcon,
}

impl Default for QuickSettingsBarWidget {
    fn default() -> Self {
        Self {
            icon: QuickSettingsIcon::Arch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct SystemUpdateBarWidget {
    /// How often to re-check pending updates, in minutes. Default
    /// 180 (= 3 h). The pill also supports right-click → immediate
    /// manual refresh for when this cadence is too lazy.
    pub check_interval_minutes: u32,
}

impl Default for SystemUpdateBarWidget {
    fn default() -> Self {
        Self {
            check_interval_minutes: 180,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Menus {
    pub clock_menu: Menu,
    pub clipboard_menu: Menu,
    pub quick_settings_menu: Menu,
    pub notification_menu: Menu,
    pub screenshot_menu: Menu,
    pub app_launcher_menu: Menu,
    pub wallpaper_menu: Menu,
    pub screenshare_menu: ScreenshareMenu,
    pub nufw_menu: Menu,
    pub ndns_menu: Menu,
    pub npodman_menu: Menu,
    pub nnotes_menu: Menu,
    pub nip_menu: Menu,
    pub nnetwork_menu: Menu,
    pub npower_menu: Menu,
    pub media_player_menu: Menu,
    pub session_menu: Menu,
    /// Settings panel — embeds in the frame's menu stack instead
    /// of launching a separate `gtk::Window` toplevel.
    pub settings_menu: Menu,
    /// Combined dashboard — hero (clock + weather) on top, then
    /// calendar + the full quick-settings stack underneath. Sits
    /// alongside the existing `clock_menu` and
    /// `quick_settings_menu` rather than replacing them so users
    /// who prefer the focused single-purpose menus keep them.
    pub dashboard_menu: Menu,
    pub left_menu_expansion_type: VerticalMenuExpansion,
    pub right_menu_expansion_type: VerticalMenuExpansion,
}

impl Default for Menus {
    fn default() -> Self {
        Self {
            clock_menu: Menu {
                position: Position::Top,
                widgets: vec![
                    MenuWidget::Calendar,
                    MenuWidget::Spacer(SpacerConfig { size: 20 }),
                    MenuWidget::Weather,
                ],
                minimum_width: 410,
            },
            clipboard_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Clipboard],
                minimum_width: 410,
            },
            quick_settings_menu: Menu {
                position: Position::TopLeft,
                widgets: vec![
                    MenuWidget::Clock,
                    MenuWidget::Network,
                    MenuWidget::Bluetooth,
                    MenuWidget::AudioOutput,
                    MenuWidget::AudioInput,
                    MenuWidget::PowerProfiles,
                    MenuWidget::MediaPlayer,
                    MenuWidget::Spacer(SpacerConfig { size: 20 }),
                    MenuWidget::QuickActions(QuickActionsConfig {
                        widgets: vec![
                            QuickActionWidget::AirplaneMode,
                            QuickActionWidget::Nightlight,
                            QuickActionWidget::HyprPicker,
                            QuickActionWidget::Settings,
                        ],
                    }),
                    MenuWidget::Spacer(SpacerConfig { size: 20 }),
                    MenuWidget::QuickActions(QuickActionsConfig {
                        widgets: vec![
                            QuickActionWidget::Logout,
                            QuickActionWidget::Lock,
                            QuickActionWidget::Reboot,
                            QuickActionWidget::Shutdown,
                        ],
                    }),
                ],
                minimum_width: 410,
            },
            notification_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Notifications],
                minimum_width: 410,
            },
            screenshot_menu: Menu {
                position: Position::TopRight,
                widgets: vec![
                    MenuWidget::Screenshots,
                    MenuWidget::Divider,
                    MenuWidget::ScreenRecording,
                ],
                minimum_width: 410,
            },
            app_launcher_menu: Menu {
                position: Position::TopLeft,
                widgets: vec![MenuWidget::AppLauncher],
                minimum_width: 410,
            },
            wallpaper_menu: Menu {
                position: Position::Top,
                widgets: vec![MenuWidget::ThemePicker, MenuWidget::Wallpaper],
                minimum_width: 1200,
            },
            screenshare_menu: ScreenshareMenu {
                position: Position::TopRight,
            },
            nufw_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Nufw],
                minimum_width: 410,
            },
            ndns_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Ndns],
                minimum_width: 420,
            },
            npodman_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Npodman],
                minimum_width: 540,
            },
            nnotes_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Nnotes],
                minimum_width: 480,
            },
            nip_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Nip],
                minimum_width: 380,
            },
            nnetwork_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Nnetwork],
                minimum_width: 460,
            },
            npower_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Npower],
                minimum_width: 360,
            },
            media_player_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::MediaPlayer],
                minimum_width: 380,
            },
            session_menu: Menu {
                position: Position::Top,
                widgets: vec![MenuWidget::Session],
                minimum_width: 420,
            },
            settings_menu: Menu {
                // Settings is a wide / tall panel. Top anchor with
                // a generous width — same family as the wallpaper
                // and notifications menus. `widgets` is empty
                // because the settings panel renders itself; the
                // generic MenuWidget pipeline isn't used.
                position: Position::Top,
                widgets: vec![],
                minimum_width: 780,
            },
            dashboard_menu: Menu {
                // GNOME-style compound dashboard:
                //
                //   ┌── Hero (Clock widget — big time + date) ──┐
                //   ├── 2-col row ─────────────────────────────┤
                //   │ ┌─ LEFT ─────┐  ┌─ RIGHT ──────────────┐ │
                //   │ │ Calendar   │  │ Network              │ │
                //   │ │ Weather    │  │ Bluetooth            │ │
                //   │ │ MediaPlayer│  │ AudioOutput          │ │
                //   │ │            │  │ AudioInput           │ │
                //   │ │            │  │ PowerProfiles        │ │
                //   │ │            │  │ QuickActions (toggle)│ │
                //   │ └────────────┘  └──────────────────────┘ │
                //   └── Footer (QuickActions: power row) ──────┘
                //
                // The 2-col row is a horizontal Container holding
                // two vertical Containers. Each Container's
                // children render exactly like they do in the
                // standalone quick-settings menu — same widget
                // controllers, same card styling. The hero +
                // footer rows are still ordinary stacked widgets
                // inside the menu's main vertical box.
                position: Position::Top,
                widgets: vec![
                    // ── Hero band ──
                    MenuWidget::Clock,
                    MenuWidget::Spacer(SpacerConfig { size: 10 }),
                    // ── 2-col body ──
                    MenuWidget::Container(ContainerConfig {
                        widgets: vec![
                            // Left column — uses CalendarGrid (no
                            // hero band) since the dashboard's
                            // top Clock widget already fills the
                            // "big time + date" role. Pairing
                            // Calendar (which has its own primary-
                            // tinted hero) here would render two
                            // overlapping time displays.
                            MenuWidget::Container(ContainerConfig {
                                widgets: vec![
                                    MenuWidget::CalendarGrid,
                                    MenuWidget::Weather,
                                    MenuWidget::MediaPlayer,
                                ],
                                spacing: 10,
                                orientation: Orientation::Vertical,
                                minimum_width: 320,
                            }),
                            // Right column
                            MenuWidget::Container(ContainerConfig {
                                widgets: vec![
                                    MenuWidget::Network,
                                    MenuWidget::Bluetooth,
                                    MenuWidget::AudioOutput,
                                    MenuWidget::AudioInput,
                                    MenuWidget::PowerProfiles,
                                    MenuWidget::QuickActions(QuickActionsConfig {
                                        widgets: vec![
                                            QuickActionWidget::AirplaneMode,
                                            QuickActionWidget::Nightlight,
                                            QuickActionWidget::HyprPicker,
                                            QuickActionWidget::Settings,
                                        ],
                                    }),
                                ],
                                spacing: 8,
                                orientation: Orientation::Vertical,
                                minimum_width: 360,
                            }),
                        ],
                        spacing: 12,
                        orientation: Orientation::Horizontal,
                        minimum_width: 0,
                    }),
                    MenuWidget::Spacer(SpacerConfig { size: 10 }),
                    // ── Power footer ──
                    MenuWidget::QuickActions(QuickActionsConfig {
                        widgets: vec![
                            QuickActionWidget::Lock,
                            QuickActionWidget::Logout,
                            QuickActionWidget::Reboot,
                            QuickActionWidget::Shutdown,
                        ],
                    }),
                ],
                minimum_width: 760,
            },
            left_menu_expansion_type: VerticalMenuExpansion::AlwaysExpanded,
            right_menu_expansion_type: VerticalMenuExpansion::AlwaysExpanded,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Notifications {
    pub notification_position: NotificationPosition,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            notification_position: NotificationPosition::Right,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, Store, JsonSchema,
)]
pub enum WallpaperRotationMode {
    /// Walk the directory listing in order.
    #[default]
    Sequential,
    /// Pick a random wallpaper each time.
    Random,
}

impl reactive_stores::PatchField for WallpaperRotationMode {
    fn patch_field(
        &mut self,
        new: Self,
        path: &reactive_stores::StorePath,
        notify: &mut dyn FnMut(&reactive_stores::StorePath),
        _keys: Option<&reactive_stores::KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

impl WallpaperRotationMode {
    pub fn to_index(&self) -> u32 {
        match self {
            WallpaperRotationMode::Sequential => 0,
            WallpaperRotationMode::Random => 1,
        }
    }

    pub fn from_index(idx: u32) -> Self {
        match idx {
            1 => WallpaperRotationMode::Random,
            _ => WallpaperRotationMode::Sequential,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            WallpaperRotationMode::Sequential => "Sequential",
            WallpaperRotationMode::Random => "Random",
        }
    }

    pub fn display_names() -> Vec<&'static str> {
        vec!["Sequential", "Random"]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Wallpaper {
    pub wallpaper_dir: String,
    pub content_fit: ContentFit,
    pub apply_theme_filter: bool,
    pub theme_filter_strength: ThemeFilterStrength,
    /// Auto-rotate the wallpaper on a timer.
    pub rotation_enabled: bool,
    /// Minutes between automatic rotations.
    pub rotation_interval_minutes: u32,
    /// Sequential vs random rotation order.
    pub rotation_mode: WallpaperRotationMode,
}

impl Default for Wallpaper {
    fn default() -> Self {
        Self {
            wallpaper_dir: "".to_string(),
            content_fit: ContentFit::Cover,
            apply_theme_filter: false,
            theme_filter_strength: ThemeFilterStrength::new(1.0),
            rotation_enabled: false,
            rotation_interval_minutes: 5,
            rotation_mode: WallpaperRotationMode::Sequential,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Frame {
    pub enable_frame: bool,
    pub monitor_filter: Vec<String>,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            enable_frame: true,
            monitor_filter: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Menu {
    pub position: Position,
    pub widgets: Vec<MenuWidget>,
    pub minimum_width: i32,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            position: Position::Left,
            widgets: Vec::new(),
            minimum_width: 410,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct ScreenshareMenu {
    pub position: Position,
}

impl Default for ScreenshareMenu {
    fn default() -> Self {
        Self {
            position: Position::Left,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct HorizontalBar {
    pub minimum_height: i32,
    pub reveal_by_default: bool,
    pub left_widgets: Vec<BarWidget>,
    pub center_widgets: Vec<BarWidget>,
    pub right_widgets: Vec<BarWidget>,
}

impl Default for HorizontalBar {
    fn default() -> Self {
        Self {
            minimum_height: 0,
            reveal_by_default: true,
            left_widgets: Vec::new(),
            center_widgets: Vec::new(),
            right_widgets: Vec::new(),
        }
    }
}

// NOTE: The upstream `VerticalBar` struct (used by `bars.left_bar`
// / `bars.right_bar` in OkShell) has been removed alongside the
// vertical bar surfaces themselves. Migration guidance for users
// with an old YAML config: rename `left_bar:` → `top_bar:`, and
// map the widget slots:
//   top_widgets    → left_widgets
//   center_widgets → center_widgets   (unchanged)
//   bottom_widgets → right_widgets
//
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum VerticalMenuExpansion {
    AlwaysExpanded,
    ExpandBothWays,
    ExpandUp,
    ExpandDown,
}

impl PatchField for VerticalMenuExpansion {
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

impl VerticalMenuExpansion {
    pub fn to_index(&self) -> u32 {
        match self {
            VerticalMenuExpansion::AlwaysExpanded => 0,
            VerticalMenuExpansion::ExpandBothWays => 1,
            VerticalMenuExpansion::ExpandUp => 2,
            VerticalMenuExpansion::ExpandDown => 3,
        }
    }

    pub fn from_index(idx: u32) -> Self {
        match idx {
            0 => VerticalMenuExpansion::AlwaysExpanded,
            1 => VerticalMenuExpansion::ExpandBothWays,
            2 => VerticalMenuExpansion::ExpandUp,
            _ => VerticalMenuExpansion::ExpandDown,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            VerticalMenuExpansion::AlwaysExpanded => "Always Expanded",
            VerticalMenuExpansion::ExpandBothWays => "Expand Both Ways",
            VerticalMenuExpansion::ExpandUp => "Expand Up",
            VerticalMenuExpansion::ExpandDown => "Expand Down",
        }
    }

    pub fn display_names() -> Vec<&'static str> {
        Self::all().iter().map(|p| p.display_name()).collect()
    }

    pub fn all() -> &'static [VerticalMenuExpansion] {
        &[
            VerticalMenuExpansion::AlwaysExpanded,
            VerticalMenuExpansion::ExpandBothWays,
            VerticalMenuExpansion::ExpandUp,
            VerticalMenuExpansion::ExpandDown,
        ]
    }
}
