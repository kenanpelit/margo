use crate::schema::bar_widgets::BarWidget;
use crate::schema::clipboard::Clipboard;
use crate::schema::content_fit::ContentFit;
use crate::schema::location_query::{LocationQueryConfig, OrdF64};
use crate::schema::menu_widgets::{
    ContainerConfig, MenuWidget, PanelHeaderConfig, QuickActionWidget, QuickActionsConfig,
    SpacerConfig,
};
use crate::schema::position::{NotificationPosition, Orientation, Position};
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
    pub clipboard: Clipboard,
    pub launcher: Launcher,
    pub valent: Valent,
    pub dock: Dock,
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
#[derive(Default)]
pub struct Session {
    pub lock_command: String,
    pub logout_command: String,
    pub suspend_command: String,
    pub reboot_command: String,
    pub shutdown_command: String,
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
    /// Monospace family — drives `--font-family-monospace` (clipboard
    /// previews, the 2FA code chip, other tabular/code-ish bits). Empty
    /// = the CSS `monospace` generic.
    pub monospace: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Sizing {
    pub radius_widget: i32,
    pub radius_window: i32,
    pub border_width: i32,
    /// Hover tint strength (%) shared by every bar pill — the matugen
    /// `--primary` mixed over the bar at this opacity. One value drives
    /// all bar widgets so their hovers read identically (Settings →
    /// Bar). Sane range ~0–40.
    pub bar_hover_strength: i32,
    /// Multiplier applied to every font-size inside the Settings
    /// panel. `1.0` keeps the +1pt-bumped defaults; bigger values
    /// scale further (useful on hi-DPI displays where 15-16 px
    /// reads small), smaller values shrink. Range is enforced
    /// loosely — values outside `0.5..=2.0` will warp the layout.
    pub settings_font_scale: f64,
    /// Global UI font scale — multiplies every `--font-*` token across
    /// the whole shell (bar + menus + dashboard), unlike
    /// `settings_font_scale` which only touches the Settings panel.
    /// `1.0` = unscaled; clamped to `0.5..=2.0`.
    pub font_scale: f64,
    /// Bar-pill font scale — multiplies `--font-bar` (clock / battery /
    /// media / network labels) on top of the global `font_scale`.
    /// `1.0` = unscaled; clamped to `0.5..=2.0`.
    pub bar_font_scale: f64,
}

impl Default for Sizing {
    fn default() -> Self {
        Self {
            // 12 px is the house default for both the widget (bar pill /
            // frame chrome) and window (menu / dialog / settings panel)
            // corner radius — softer than the old 8 px, and the value the
            // Settings → Theme → Sizing "Reset to defaults" button restores.
            radius_widget: 12,
            radius_window: 12,
            border_width: 2,
            bar_hover_strength: 14,
            settings_font_scale: 1.0,
            font_scale: 1.0,
            bar_font_scale: 1.0,
        }
    }
}

// Manually implemented because the `f64` field rules out a
// derived `Eq` (NaN ≠ NaN). PartialEq still works for the
// reactive store's change detection — float comparison is exact
// here because slider widgets snap to user-typed values.
impl Eq for Sizing {}

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
                left_widgets: vec![BarWidget::MargoTags],
                center_widgets: vec![BarWidget::MargoDock],
                right_widgets: vec![
                    BarWidget::RecordingIndicator,
                    BarWidget::Tray,
                    BarWidget::Screenshot,
                    BarWidget::Wallpaper,
                    BarWidget::Clipboard,
                    BarWidget::Notifications,
                    BarWidget::AudioDashboard,
                    BarWidget::Bluetooth,
                    BarWidget::Network,
                    BarWidget::Clock,
                ],
            },
            bottom_bar: HorizontalBar::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct BarWidgets {
    pub system_update: SystemUpdateBarWidget,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct SystemUpdateBarWidget {
    /// How often to re-check pending updates, in minutes. Default
    /// 180 (= 3 h). The pill also supports right-click → immediate
    /// manual refresh for when this cadence is too lazy.
    pub check_interval_minutes: u32,
    /// Count official-repo updates (pacman `checkupdates`, or the
    /// distro fallback). Toggle in Settings → System Updates.
    pub check_repo: bool,
    /// Count AUR updates via an AUR helper (`paru -Qua` / `yay -Qua`).
    pub check_aur: bool,
    /// Count Flatpak updates (`flatpak remote-ls --updates`).
    pub check_flatpak: bool,
}

impl Default for SystemUpdateBarWidget {
    fn default() -> Self {
        Self {
            check_interval_minutes: 180,
            check_repo: true,
            check_aur: true,
            check_flatpak: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Menus {
    pub clock_menu: Menu,
    pub clipboard_menu: Menu,
    pub notification_menu: Menu,
    pub screenshot_menu: Menu,
    pub app_launcher_menu: Menu,
    pub wallpaper_menu: Menu,
    pub screenshare_menu: ScreenshareMenu,
    pub ufw_menu: Menu,
    pub dns_menu: Menu,
    pub podman_menu: Menu,
    pub notes_menu: Menu,
    pub ip_menu: Menu,
    pub network_menu: Menu,
    pub power_menu: Menu,
    // Default-on-missing so older user YAML predating these menu
    // types still parses cleanly.
    #[serde(default = "default_bluetooth_menu")]
    pub bluetooth_menu: Menu,
    #[serde(default = "default_cpu_dashboard_menu")]
    pub cpu_dashboard_menu: Menu,
    #[serde(default = "default_audio_dashboard_menu")]
    pub audio_dashboard_menu: Menu,
    #[serde(default = "default_system_update_menu")]
    pub system_update_menu: Menu,
    #[serde(default = "default_valent_menu")]
    pub valent_menu: Menu,
    #[serde(default = "default_keep_awake_menu")]
    pub keep_awake_menu: Menu,
    #[serde(default = "default_weather_menu")]
    pub weather_menu: Menu,
    #[serde(default = "default_twilight_menu")]
    pub twilight_menu: Menu,
    #[serde(default = "default_keybinds_menu")]
    pub keybinds_menu: Menu,
    #[serde(default = "default_ssh_menu")]
    pub ssh_menu: Menu,
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
    /// Margo layout switcher — replaces the legacy bar-popover
    /// variant. Anchored to whichever side the user pins it to;
    /// content is a single `MargoLayout` widget rendering a
    /// vertical list of compositor layouts.
    pub margo_layout_menu: Menu,
    pub left_menu_expansion_type: VerticalMenuExpansion,
    pub right_menu_expansion_type: VerticalMenuExpansion,
}

fn default_bluetooth_menu() -> Menu {
    Menu {
        position: Position::Top,
        widgets: vec![MenuWidget::Bluetooth],
        minimum_width: 400,
        maximum_height: 0,
    }
}

fn default_system_update_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::SystemUpdate],
        minimum_width: 460,
        maximum_height: 620,
    }
}

fn default_valent_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Valent],
        minimum_width: 460,
        maximum_height: 620,
    }
}

fn default_keep_awake_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::KeepAwake],
        minimum_width: 320,
        maximum_height: 0,
    }
}

fn default_weather_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Weather],
        minimum_width: 380,
        maximum_height: 0,
    }
}

fn default_twilight_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Twilight],
        minimum_width: 360,
        maximum_height: 0,
    }
}

fn default_keybinds_menu() -> Menu {
    Menu {
        position: Position::Top,
        widgets: vec![MenuWidget::Keybinds],
        // Wide enough for the two-column "combo | description" rows
        // without wrapping common labels; capped height so the long
        // shortcut list scrolls instead of overflowing the screen.
        minimum_width: 720,
        maximum_height: 720,
    }
}

fn default_ssh_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::SshSessions],
        // Roomy enough for "host + user@hostname:port" rows; capped
        // height so a large ~/.ssh/config scrolls.
        minimum_width: 460,
        maximum_height: 720,
    }
}

fn default_cpu_dashboard_menu() -> Menu {
    Menu {
        position: Position::Top,
        widgets: vec![MenuWidget::CpuDashboard],
        minimum_width: 380,
        maximum_height: 0,
    }
}

fn default_audio_dashboard_menu() -> Menu {
    Menu {
        position: Position::Top,
        widgets: vec![MenuWidget::AudioDashboard],
        minimum_width: 400,
        maximum_height: 0,
    }
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
                maximum_height: 0,
            },
            clipboard_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Clipboard],
                // Golden-ratio proportions (φ ≈ 1.618): the tabbed
                // ListBox reads best as a portrait golden rectangle
                // (890/550 ≈ φ) that is also the golden section of a
                // 1440-tall screen (890/1440 ≈ 1/φ). Tune in Settings
                // → Widgets → Clipboard for other monitor heights.
                minimum_width: 550,
                maximum_height: 890,
            },
            notification_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Notifications],
                minimum_width: 410,
                maximum_height: 0,
            },
            screenshot_menu: Menu {
                position: Position::TopRight,
                widgets: vec![
                    MenuWidget::Screenshots,
                    MenuWidget::Divider,
                    MenuWidget::ScreenRecording,
                ],
                minimum_width: 410,
                maximum_height: 0,
            },
            app_launcher_menu: Menu {
                position: Position::TopLeft,
                widgets: vec![MenuWidget::AppLauncher],
                minimum_width: 410,
                maximum_height: 0,
            },
            wallpaper_menu: Menu {
                position: Position::Top,
                widgets: vec![MenuWidget::ThemePicker, MenuWidget::Wallpaper],
                minimum_width: 1200,
                maximum_height: 0,
            },
            screenshare_menu: ScreenshareMenu {
                position: Position::TopRight,
            },
            ufw_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Ufw],
                minimum_width: 410,
                maximum_height: 0,
            },
            dns_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Dns],
                minimum_width: 420,
                maximum_height: 0,
            },
            podman_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Podman],
                minimum_width: 540,
                maximum_height: 0,
            },
            notes_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Notes],
                minimum_width: 480,
                maximum_height: 0,
            },
            ip_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Ip],
                minimum_width: 380,
                maximum_height: 0,
            },
            network_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Network],
                minimum_width: 460,
                maximum_height: 0,
            },
            power_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Power],
                minimum_width: 360,
                maximum_height: 0,
            },
            bluetooth_menu: default_bluetooth_menu(),
            cpu_dashboard_menu: default_cpu_dashboard_menu(),
            audio_dashboard_menu: default_audio_dashboard_menu(),
            system_update_menu: default_system_update_menu(),
            valent_menu: default_valent_menu(),
            keep_awake_menu: default_keep_awake_menu(),
            weather_menu: default_weather_menu(),
            twilight_menu: default_twilight_menu(),
            keybinds_menu: default_keybinds_menu(),
            ssh_menu: default_ssh_menu(),
            media_player_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::MediaPlayer],
                minimum_width: 380,
                maximum_height: 0,
            },
            session_menu: Menu {
                position: Position::Top,
                widgets: vec![MenuWidget::Session],
                minimum_width: 420,
                maximum_height: 0,
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
                maximum_height: 0,
            },
            dashboard_menu: Menu {
                // Rebalanced two-column dashboard:
                //
                //   ┌── Hero (compact Clock status strip) ─────┐
                //   ├── 2-col row ─────────────────────────────┤
                //   │ ┌─ LEFT (calendar/weather) ┐  ┌─ RIGHT (controls + media) ─┐ │
                //   │ │ CalendarGrid             │  │ Network                    │ │
                //   │ │ Weather                  │  │ Bluetooth                  │ │
                //   │ │                          │  │ AudioOutput                │ │
                //   │ │                          │  │ AudioInput                 │ │
                //   │ │                          │  │ PowerProfiles              │ │
                //   │ │                          │  │ MediaPlayer                │ │
                //   │ │                          │  │ QuickActions (toggle)      │ │
                //   │ │                          │  │ QuickActions (power)       │ │
                //   │ └──────────────────────────┘  └────────────────────────────┘ │
                //   └──────────────────────────────────────────┘
                //
                // Compared with the previous layout: MediaPlayer
                // moved from the left to the right-column bottom.
                // The left column is now pure "time/date context"
                // (calendar + weather) so it reads as a focused
                // information panel; the right column carries all
                // the actionable controls + the now-rich media
                // surface as the bottom anchor.
                position: Position::Top,
                widgets: vec![
                    // ── §12 panel header ──
                    // Replaces the old Clock hero: the big time was
                    // redundant with the bar clock, so the dashboard
                    // leads with a title + dim date + settings gear
                    // (DESIGN.md §12). The decorative primary underline
                    // the Clock hero carried goes away with it.
                    MenuWidget::PanelHeader(PanelHeaderConfig {
                        title: "Dashboard".to_string(),
                    }),
                    MenuWidget::Spacer(SpacerConfig { size: 8 }),
                    // ── 2-col body ──
                    MenuWidget::Container(ContainerConfig {
                        widgets: vec![
                            // Left column = pure "today at a glance"
                            // — calendar and weather as their own
                            // tiles. (Previously wrapped in a
                            // DailyOverview merged surface; user
                            // asked for them separated.)
                            MenuWidget::Container(ContainerConfig {
                                widgets: vec![
                                    MenuWidget::CalendarGrid,
                                    MenuWidget::Weather,
                                ],
                                spacing: 10,
                                orientation: Orientation::Vertical,
                                // Equalised with the right column —
                                // both sides share the same width so
                                // the dashboard reads as a symmetric
                                // two-pane layout. The parent's
                                // `homogeneous` is what actually forces
                                // the equal split; this width is the
                                // shared floor.
                                minimum_width: 400,
                                homogeneous: false,
                                // Stretch this column's tiles to fill
                                // its height so both columns end at the
                                // same bottom edge.
                                fill: true,
                            }),
                            // Right column = controls + media, with
                            // OverviewIntel pinned at the top so the
                            // urgency signals (notifications, low
                            // battery, thermal alerts) sit separate
                            // from the calendar/weather context
                            // group on the left.
                            MenuWidget::Container(ContainerConfig {
                                widgets: vec![
                                    MenuWidget::OverviewIntel,
                                    MenuWidget::Connectivity,
                                    MenuWidget::CompactAudio,
                                    // SystemStatus replaces the
                                    // standalone PowerProfiles tile
                                    // — combines profile + battery
                                    // + CPU temp in one compact card.
                                    MenuWidget::SystemStatus,
                                    MenuWidget::MediaPlayer,
                                ],
                                spacing: 8,
                                orientation: Orientation::Vertical,
                                // Equalised with the left column —
                                // 400 px on each side keeps the
                                // standalone-QS breathing room while
                                // making the dashboard symmetric.
                                minimum_width: 400,
                                homogeneous: false,
                                // Stretch this column's tiles to fill
                                // its height so both columns end at the
                                // same bottom edge.
                                fill: true,
                            }),
                        ],
                        spacing: 12,
                        orientation: Orientation::Horizontal,
                        minimum_width: 0,
                        // Force the two columns to identical widths
                        // regardless of which side's content is
                        // naturally wider — symmetric two-pane body.
                        homogeneous: true,
                        // Horizontal body: per-column fill is set on
                        // the inner vertical columns, not here.
                        fill: false,
                    }),
                    // ── Bottom centred QuickActions strip ──
                    //
                    // Both toggle (AirplaneMode / Nightlight /
                    // ColorPicker / Settings) and power (Logout /
                    // Lock / Reboot / Shutdown) buttons share one
                    // horizontal row. QuickActions widget already
                    // centres itself via `set_align: Center`, so
                    // sitting at the dashboard root puts it as a
                    // sibling of the column container — centred
                    // under the body.
                    MenuWidget::Spacer(SpacerConfig { size: 10 }),
                    MenuWidget::QuickActions(QuickActionsConfig {
                        widgets: vec![
                            QuickActionWidget::AirplaneMode,
                            QuickActionWidget::Nightlight,
                            QuickActionWidget::ColorPicker,
                            QuickActionWidget::Wallpaper,
                            QuickActionWidget::Screenshot,
                            QuickActionWidget::Settings,
                            QuickActionWidget::Logout,
                            QuickActionWidget::Lock,
                            QuickActionWidget::Reboot,
                            QuickActionWidget::Shutdown,
                        ],
                    }),
                ],
                // 400 (left) + 12 (spacing) + 400 (right) + ~40
                // (menu padding) wants ~852; round to 860 so the
                // outer menu opens at a width where both equal-
                // sized columns slot in without renegotiation.
                minimum_width: 860,
                maximum_height: 0,
            },
            margo_layout_menu: Menu {
                // Replaces the legacy `gtk::PopoverMenu` that
                // opened as a separate `xdg_popup` window. Lives
                // in the frame's menu stack now so it slides out
                // contiguous with the bar like every other menu.
                // The single `MargoLayout` widget renders the
                // full list of compositor layouts with the
                // currently-active row highlighted.
                position: Position::Top,
                widgets: vec![MenuWidget::MargoLayout],
                minimum_width: 280,
                maximum_height: 0,
            },
            left_menu_expansion_type: VerticalMenuExpansion::AlwaysExpanded,
            right_menu_expansion_type: VerticalMenuExpansion::AlwaysExpanded,
        }
    }
}

/// Launcher-wide settings (currently the `>start` script autostart
/// list). Each entry is keyed by the script's short name.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct Launcher {
    /// Scripts the user opted into running at shell startup, with a
    /// per-script delay. Names match `ScriptsProvider` short names
    /// (e.g. `start-brave-ai`).
    pub autostart_scripts: Vec<ScriptAutostart>,
}

/// Valent (KDE Connect) integration settings. `main_device_id` is the
/// sticky device the bar pill + panel default to when several phones
/// are paired; empty means "auto-pick the first reachable one".
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct Valent {
    pub main_device_id: String,
}

/// Margo dock (the running/pinned app strip). Tunables surfaced under
/// Settings → Widgets → Margo Dock.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Dock {
    /// App-icon pixel size.
    pub icon_size: u32,
    /// Hover tooltip listing the app + its open window titles.
    pub show_tooltips: bool,
    /// Include running apps that aren't pinned (off = pinned-only dock).
    pub show_running: bool,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            icon_size: 32,
            show_tooltips: true,
            show_running: true,
        }
    }
}

/// One `>start` script's autostart configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct ScriptAutostart {
    /// Script short name (e.g. `start-brave-ai`).
    pub name: String,
    /// Run this script at shell startup.
    pub enabled: bool,
    /// Seconds to wait after startup before running it.
    pub delay_secs: u32,
}


#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Notifications {
    pub notification_position: NotificationPosition,
    /// App names (substring match, case-insensitive) whose
    /// notifications are silently dropped — the per-app mute list.
    /// Applied to the wayle service's blocklist on startup and
    /// whenever it changes.
    #[serde(default)]
    pub blocklist: Vec<String>,
    /// Show the small close (✕) button in each notification's header.
    /// On by default — it's the primary manual dismiss (a horizontal
    /// swipe also dismisses). Set `false` for a cleaner toast.
    pub show_close_button: bool,
    /// Show the app-provided action buttons (View / Open / Reply / …).
    /// Off by default — these are the large buttons that clutter toasts;
    /// turn on if you act on notifications straight from the popup.
    pub show_action_buttons: bool,
    /// Group the notification history by app: two or more notifications
    /// from the same app collapse into an expandable "App (N)" header.
    /// On by default. Set `false` for a flat, chronological list.
    pub group_notifications: bool,
    /// Width (px) of the corner popup toasts. Independent of the
    /// notification *history* menu width (that lives in
    /// `menus.notification_menu.minimum_width`); this is the
    /// transient toast surface anchored to a screen corner.
    pub popup_width: i32,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            notification_position: NotificationPosition::Right,
            blocklist: Vec::new(),
            show_close_button: true,
            show_action_buttons: false,
            group_notifications: true,
            popup_width: 460,
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
    /// Maximum content height in pixels. 0 = no cap (legacy
    /// "grow to fit children" behaviour). When > 0, the menu's
    /// outer ScrolledWindow caps the visible height at this value
    /// and the inner content scrolls vertically — useful for
    /// menus with long, scrollable result lists (app launcher,
    /// clipboard history…) so the panel doesn't grow taller than
    /// the user's monitor.
    pub maximum_height: i32,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            position: Position::Left,
            widgets: Vec::new(),
            minimum_width: 410,
            maximum_height: 0,
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

#[cfg(test)]
mod schema_tests {
    use super::Config;

    /// An empty YAML map → every field falls back to its default.
    #[test]
    fn empty_map_deserializes_to_default() {
        let cfg: Config = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg, Config::default());
    }

    /// New notification button toggles: off / on by default.
    #[test]
    fn notification_button_defaults() {
        let cfg: Config = serde_yaml::from_str("{}").unwrap();
        assert!(
            !cfg.notifications.show_action_buttons,
            "action buttons should be hidden by default"
        );
        assert!(
            cfg.notifications.show_close_button,
            "close button should be shown by default"
        );
    }

    #[test]
    fn explicit_notification_buttons_parse() {
        let cfg: Config = serde_yaml::from_str(
            "notifications:\n  show_action_buttons: true\n  show_close_button: false\n",
        )
        .unwrap();
        assert!(cfg.notifications.show_action_buttons);
        assert!(!cfg.notifications.show_close_button);
    }

    /// A partial config only sets the listed fields; everything else
    /// (including untouched sub-structs) keeps its default.
    #[test]
    fn partial_yaml_keeps_other_defaults() {
        let cfg: Config =
            serde_yaml::from_str("general:\n  clock_format_24_h: true\n").unwrap();
        assert!(cfg.general.clock_format_24_h);
        // notifications section was never mentioned → defaults intact.
        assert!(cfg.notifications.show_close_button);
        assert!(!cfg.notifications.show_action_buttons);
    }

    /// Unknown keys are ignored (forward-compat: a newer config on an
    /// older binary must still load).
    #[test]
    fn unknown_top_level_key_is_ignored() {
        let parsed = serde_yaml::from_str::<Config>("a_key_from_the_future: 42\n");
        assert!(parsed.is_ok(), "unknown keys should not fail the parse");
    }

    /// Serialising the default config and reading it back is lossless.
    #[test]
    fn default_config_round_trips_through_yaml() {
        let def = Config::default();
        let yaml = serde_yaml::to_string(&def).unwrap();
        let back: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back, def);
    }
}
