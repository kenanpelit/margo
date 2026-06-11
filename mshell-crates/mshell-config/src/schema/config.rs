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
    pub system_tray: SystemTray,
    pub pass: Pass,
    pub alarm: AlarmConfig,
    pub network: NetworkConfig,
    pub login_network: LoginNetworkConfig,
    pub bluetooth: BluetoothConfig,
    pub power: PowerConfig,
    pub privacy: PrivacyConfig,
    pub audio: AudioConfig,
    pub control_center: ControlCenterConfig,
    pub logging: LoggingConfig,
}

/// One configured alarm. `repeat_mask` bit `i` (0 = Sunday … 6 = Saturday)
/// marks a repeating weekday; mask 0 = a one-shot alarm that disables itself
/// after firing. Snooze state is runtime-only (not persisted).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Alarm {
    pub hour: u8,
    pub minutes: u8,
    pub name: String,
    pub enabled: bool,
    pub repeat_mask: u8,
}

impl Default for Alarm {
    fn default() -> Self {
        Self {
            hour: 7,
            minutes: 0,
            name: String::new(),
            enabled: false,
            repeat_mask: 0,
        }
    }
}

/// Alarm-clock widget config: the alarm list plus ring behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct AlarmConfig {
    pub alarms: Vec<Alarm>,
    /// Minutes to snooze when the alarm notification's Snooze action is hit.
    pub snooze_minutes: u32,
    /// Pop a desktop notification (with Stop / Snooze actions) when ringing.
    pub notifications: bool,
    /// Notification urgency: `low` | `normal` | `critical`.
    pub urgency: String,
}

impl Default for AlarmConfig {
    fn default() -> Self {
        Self {
            alarms: Vec::new(),
            snooze_minutes: 5,
            notifications: true,
            urgency: "normal".to_string(),
        }
    }
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

/// A user-bookmarked weather location: a display name + the query used
/// to fetch it. The active location stays `General::weather_location_query`;
/// the weather menu's location switcher writes the chosen entry's `query`
/// into it (the live `set_location` effect then refetches). Edited via
/// Settings → Weather ("Save current as…" / Remove) or by hand in YAML.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct SavedLocation {
    pub name: String,
    pub query: LocationQueryConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct General {
    pub clock_format_24_h: bool,
    pub weather_location_query: LocationQueryConfig,
    /// Weather locations the menu's location switcher flips between
    /// (selecting one writes its `query` into `weather_location_query`).
    /// Empty by default — add bookmarks from Settings → Weather.
    pub weather_saved_locations: Vec<SavedLocation>,
    pub temperature_unit: TemperatureUnitConfig,
    /// Minutes between weather refreshes (the Open-Meteo poll interval).
    /// Normal cadence between successful weather fetches. On a failure the
    /// shell switches to `weather_retry_minutes` until a fetch succeeds again.
    #[serde(default = "default_weather_poll_minutes")]
    pub weather_poll_minutes: u32,
    /// Backoff between attempts while a weather fetch keeps failing
    /// (rate-limit / offline). Set high (e.g. 720 = 12 h) to stop hammering an
    /// endpoint that's down; returns to `weather_poll_minutes` once a fetch
    /// succeeds.
    #[serde(default = "default_weather_retry_minutes")]
    pub weather_retry_minutes: u32,
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
    /// Discrete seek step in seconds for the media-player menu's
    /// ⏪ / ⏩ buttons (the relative-seek controls ported from the
    /// mplayerplus plugin). The draggable progress bar is unaffected.
    pub media_seek_step_seconds: u32,
    /// Show a larger album cover in the media-player menu. Off keeps the
    /// compact 64 px cover; on bumps it to a roomier size for a more
    /// "now playing"-style panel.
    pub media_large_album_art: bool,
    /// Settings-panel width override in pixels. `0` = auto (a clamped
    /// fraction of the host monitor, the historical behaviour). Any
    /// positive value pins the panel to that exact width. Edited in
    /// Settings → General → "Settings panel". The panel's *position*
    /// lives separately in `menus.settings_menu.position`.
    pub settings_panel_width: i32,
    /// Settings-panel height override in pixels. `0` = auto (a clamped
    /// fraction of the host monitor). Any positive value pins the height.
    pub settings_panel_height: i32,
}

fn default_weather_poll_minutes() -> u32 {
    15
}

fn default_weather_retry_minutes() -> u32 {
    // Back off a full hour on any fetch failure (rate-limit / offline) instead
    // of hammering the endpoint. Configurable up to 12 h+ in Settings.
    60
}

impl Default for General {
    fn default() -> Self {
        Self {
            clock_format_24_h: false,
            weather_location_query: LocationQueryConfig::Coordinates {
                lat: OrdF64(0.0),
                lon: OrdF64(0.0),
            },
            weather_saved_locations: Vec::new(),
            temperature_unit: TemperatureUnitConfig::Metric,
            weather_poll_minutes: default_weather_poll_minutes(),
            weather_retry_minutes: default_weather_retry_minutes(),
            show_screen_corners: false,
            screen_corner_radius: 24,
            network_osd_enabled: false,
            media_seek_step_seconds: 10,
            media_large_album_art: false,
            settings_panel_width: 0,
            settings_panel_height: 0,
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
    /// Painted shell-surface opacity, as a percentage (`100` = opaque). Lower
    /// values frost the bar + menu/panel backgrounds so the wallpaper shows
    /// through (ashell-style). Clamped to `60..=100` so a surface never
    /// disappears. Drives `--surface-opacity` (the frame-draw fill alpha +
    /// the frameless panel backgrounds).
    pub surface_opacity: i32,
    /// Manual frame FILL colour override as a CSS hex string
    /// (`#rrggbbaa`). Empty = follow the matugen `--surface` role. Set from
    /// Settings → Bar → Frame; drives `--frame-bg` (the painted bar frame).
    pub frame_color: String,
    /// Manual frame BORDER colour override (CSS hex `#rrggbbaa`). Empty =
    /// matugen `--outline`. Drives `--frame-border`.
    pub frame_border_color: String,
    /// Manual bar SEPARATOR colour override (CSS hex `#rrggbbaa`). Empty =
    /// matugen `--outline`. Drives `--bar-separator-color`. Settings → Bar.
    pub separator_color: String,
}

impl Default for Sizing {
    fn default() -> Self {
        Self {
            // Soft, GNOME/ashell-style defaults: 16 px for the widget (bar
            // pill / frame chrome) and 18 px for the window (menu / dialog /
            // settings panel) corner radius — matches the semantic shape
            // scale's gentle rounding. The value Settings → Theme → Sizing
            // "Reset to defaults" restores.
            radius_widget: 16,
            radius_window: 18,
            border_width: 2,
            bar_hover_strength: 14,
            settings_font_scale: 1.0,
            font_scale: 1.0,
            bar_font_scale: 1.0,
            surface_opacity: 100,
            frame_color: String::new(),
            frame_border_color: String::new(),
            separator_color: String::new(),
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
    /// When true, the light/dark polarity is auto-derived from the
    /// wallpaper's average luminance (bright → Light, dark → Dark) on each
    /// wallpaper change, overriding `mode`. Only applies to the
    /// wallpaper-driven (`Themes::Wallpaper`) theme.
    pub auto_polarity: bool,
}

impl Default for Matugen {
    fn default() -> Self {
        Self {
            preference: MatugenPreference::Darkness,
            scheme_type: MatugenType::TonalSpot,
            mode: MatugenMode::Dark,
            contrast: MatugenContrast::new(0.0),
            auto_polarity: false,
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
    /// "Islands" appearance: a transparent bar where each pill floats as
    /// its own opaque rounded surface (the inverse of the default
    /// continuous-strip look). Opt-in; default off.
    #[serde(default)]
    pub islands: bool,
    /// Bar show/hide slide animation duration in milliseconds. Set this to
    /// match the compositor's window move-animation (margo
    /// `animation_duration_move`, default 500) so a bar toggle slides and the
    /// windows resize on the same clock — otherwise the window edge lags the
    /// bar. 0 disables the slide (instant).
    #[serde(default = "default_bar_slide_ms")]
    pub slide_duration_ms: u32,
}

fn default_bar_slide_ms() -> u32 {
    500
}

impl Default for Bars {
    fn default() -> Self {
        Self {
            frame: Frame::default(),
            widgets: BarWidgets::default(),
            top_bar: HorizontalBar {
                enabled: true,
                minimum_height: 0,
                reveal_by_default: true,
                auto_hide_delay_ms: default_auto_hide_delay_ms(),
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
                hidden_widgets: Vec::new(),
            },
            bottom_bar: HorizontalBar::default(),
            islands: false,
            slide_duration_ms: default_bar_slide_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct BarWidgets {
    pub system_update: SystemUpdateBarWidget,
    /// Hidden Bar drawer behaviour (hover-expand, auto-collapse, …).
    pub hidden_bar: HiddenBarConfig,
    /// Catwalk — the CPU-reactive animated cat pill.
    pub catwalk: CatwalkConfig,
    /// Privacy indicator — mic / camera / screen-share watchdog pill.
    #[serde(default)]
    pub privacy: PrivacyWidgetConfig,
    /// User-defined pills, referenced from a bar slot via `!Custom <name>`.
    pub custom_widgets: Vec<CustomWidgetConfig>,
}

/// Which themed accent the [`crate::schema::bar_widgets::BarWidget::Privacy`]
/// pill lights up with when a sensor is in use. Maps to a matugen CSS var
/// via a `privacy-accent-*` class on the pill (see `_privacy.scss`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub enum PrivacyAccent {
    Primary,
    Error,
    Secondary,
    Tertiary,
}

impl PrivacyAccent {
    /// CSS modifier class the pill carries so the stylesheet can pick the
    /// matugen var (never hardcode colours — see DESIGN.md).
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Primary => "privacy-accent-primary",
            Self::Error => "privacy-accent-error",
            Self::Secondary => "privacy-accent-secondary",
            Self::Tertiary => "privacy-accent-tertiary",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Error => "Error (red)",
            Self::Secondary => "Secondary",
            Self::Tertiary => "Tertiary",
        }
    }
    pub fn all() -> &'static [Self] {
        &[Self::Error, Self::Primary, Self::Secondary, Self::Tertiary]
    }
}

impl PatchField for PrivacyAccent {
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

/// Settings for the [`crate::schema::bar_widgets::BarWidget::Privacy`] pill
/// (port of the noctalia privacy-indicator plugin). Detects which apps are
/// using the microphone, a camera, or screen-sharing, lights up an inline
/// glyph per active sensor, keeps a clearable access log (left-click panel),
/// and optionally toasts on activation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct PrivacyWidgetConfig {
    /// Hide the pill entirely while nothing is in use (margo's quiet-bar
    /// default is the opposite of noctalia's — here we keep the indicator
    /// visible-but-dimmed so it reads as an always-on watchdog).
    pub hide_inactive: bool,
    /// Toast (notify-send) when a sensor first goes active.
    pub enable_toast: bool,
    /// Watch the microphone (apps recording audio).
    pub track_mic: bool,
    /// Watch cameras (apps holding a `/dev/video*` capture node).
    pub track_camera: bool,
    /// Watch for screen-sharing — polls `pw-dump` for screencast nodes.
    /// The heaviest of the three; disable on weak machines to drop the
    /// periodic PipeWire scan (mic + camera detection stay on).
    pub detect_screen_share: bool,
    /// Regex of microphone app names to ignore (e.g. your own always-on
    /// assistant). Empty = no filter.
    pub mic_filter: String,
    /// Regex of camera app names to ignore. Empty = no filter.
    pub cam_filter: String,
    /// Accent the active glyphs light up with.
    pub accent: PrivacyAccent,
    /// Max access-log entries kept (and persisted). 0 disables history.
    pub history_limit: u32,
}

impl Default for PrivacyWidgetConfig {
    fn default() -> Self {
        Self {
            hide_inactive: false,
            enable_toast: true,
            track_mic: true,
            track_camera: true,
            detect_screen_share: true,
            mic_filter: String::new(),
            cam_filter: String::new(),
            accent: PrivacyAccent::Error,
            history_limit: 50,
        }
    }
}

/// Settings for the [`BarWidget::Catwalk`] animated-cat pill (port of the
/// noctalia catwalk plugin).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct CatwalkConfig {
    /// CPU busy% below which the cat idles ("Zz"); above it walks, speeding up
    /// with load. 5–25 is sensible.
    pub minimum_threshold: u32,
    /// Drop the pill background so the cat floats on the bar.
    pub hide_background: bool,
}

impl Default for CatwalkConfig {
    fn default() -> Self {
        Self {
            minimum_threshold: 10,
            hide_background: false,
        }
    }
}

/// Behaviour knobs for the [`BarWidget::HiddenBar`] drawer (the widgets it
/// collapses come from each bar's `hidden_widgets` list).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct HiddenBarConfig {
    /// Start expanded on shell launch.
    pub start_expanded: bool,
    /// Reveal on hover (in addition to click). Off = click-only.
    pub auto_expand: bool,
    /// Delay before a hover reveals, in milliseconds (0 = instant).
    pub hover_delay_ms: u32,
    /// Collapse again after the pointer leaves (unless pinned).
    pub auto_collapse: bool,
    /// Delay before auto-collapse fires, in milliseconds.
    pub collapse_delay_ms: u32,
}

impl Default for HiddenBarConfig {
    fn default() -> Self {
        Self {
            start_expanded: false,
            auto_expand: true,
            hover_delay_ms: 0,
            auto_collapse: true,
            collapse_delay_ms: 1000,
        }
    }
}

/// A user-defined bar pill: an icon / image + optional label, with
/// left / right click commands and an optional `exec` poller whose stdout
/// fills the label via a `{output}` template. (See `bars.widgets.custom_widgets`.)
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct CustomWidgetConfig {
    /// Key referenced by a bar slot (`!Custom <name>`).
    pub name: String,
    /// Leading symbolic icon name (empty = none).
    pub icon: String,
    /// Leading image file path — takes precedence over `icon` (empty = none).
    pub image: String,
    /// Static label text (empty = none). Ignored when `exec` is set.
    pub label: String,
    /// Tooltip text (empty = none).
    pub tooltip: String,
    /// Command run on left click via `sh -c` (empty = no action).
    pub on_click: String,
    /// Command run on right click via `sh -c` (empty = no action).
    pub on_click_right: String,
    /// Command whose stdout becomes the label, via `sh -c`, refreshed every
    /// `interval` seconds (empty = static label).
    pub exec: String,
    /// Label template; `{output}` is replaced with the trimmed `exec`
    /// stdout. Empty = use the output verbatim.
    pub template: String,
    /// Refresh cadence for `exec`, in seconds. 0 = run once.
    pub interval: u64,
    /// Truncate the rendered label to this many characters (0 = no cap).
    pub max_chars: u32,
    /// When true and `exec` is set, the exec's stdout is read as
    /// `<image-path>\n<label…>`: the first line is a file path used as the
    /// leading image (reloaded on every poll — e.g. live album art), the rest
    /// is the label. An empty/missing first line falls back to `icon`.
    pub art: bool,
    /// Optional dropdown menu (popover of command rows). When non-empty, a
    /// left click opens this menu instead of running `on_click`.
    pub menu: Vec<CustomMenuRow>,
    /// Absolute path to a plugin's compiled WASM panel. When non-empty, a left
    /// click opens that sandboxed in-shell panel (requires a `wasm-plugins`
    /// build). Set only on plugin-derived widgets; empty for user widgets.
    pub panel_entry: String,
    /// JSON object of the plugin's resolved settings, passed to the WASM
    /// panel's `get-setting` capability. Only meaningful with `panel_entry`.
    pub panel_settings: String,
    /// The plugin's panel/menu surface min width + max height (a per-plugin
    /// preference edited in the plugin's own settings, applied to the shared
    /// plugin-menu surface on open). 0 = use the surface default.
    pub panel_min_width: i32,
    pub panel_max_height: i32,
}

/// One row of a custom widget's dropdown menu: an icon + label that runs a
/// `sh -c` command when activated.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct CustomMenuRow {
    pub label: String,
    pub icon: String,
    pub exec: String,
    /// Severity tint for the row (`"danger"` = destructive), per DESIGN.md.
    pub severity: String,
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
    pub vpn_menu: Menu,
    pub ai_menu: Menu,
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
    #[serde(default = "default_alarmclock_menu")]
    pub alarmclock_menu: Menu,
    #[serde(default = "default_dock_menu")]
    pub dock_menu: Menu,
    #[serde(default = "default_control_center_menu")]
    pub control_center_menu: Menu,
    #[serde(default = "default_ssh_menu")]
    pub ssh_menu: Menu,
    #[serde(default = "default_privacy_menu")]
    pub privacy_menu: Menu,
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
    /// First-class surface for plugin-provided WASM panels (mplugins WASM
    /// tier). One menu hosts whichever plugin panel is opened (by key), so an
    /// installed plugin's panel is as position/size-configurable as any
    /// built-in menu. Default-on-missing so older YAML parses.
    #[serde(default = "default_plugin_panel_menu")]
    pub plugin_panel_menu: Menu,
    pub left_menu_expansion_type: VerticalMenuExpansion,
    pub right_menu_expansion_type: VerticalMenuExpansion,
}

fn default_plugin_panel_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        // Content is the injected WASM panel, not config widgets.
        widgets: vec![],
        minimum_width: 420,
        maximum_height: 560,
    }
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

fn default_alarmclock_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::AlarmClock],
        // Roomy enough for the alarm rows (time + repeat-day chips +
        // delete) and the stopwatch hero; capped height so a long
        // alarm list scrolls instead of overflowing the screen.
        minimum_width: 420,
        maximum_height: 640,
    }
}

fn default_dock_menu() -> Menu {
    Menu {
        position: Position::Bottom,
        widgets: vec![MenuWidget::MargoDock],
        minimum_width: 0,
        maximum_height: 0,
    }
}

fn default_control_center_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::ControlCenter],
        minimum_width: 460,
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

fn default_privacy_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Privacy],
        // Roomy enough for the "in use now" rows + the access-log list
        // (icon + app + time + started/stopped); capped so a long log
        // scrolls instead of overflowing the screen.
        minimum_width: 380,
        maximum_height: 560,
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
                minimum_width: 800,
                maximum_height: 1080,
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
            vpn_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Vpn],
                minimum_width: 430,
                maximum_height: 0,
            },
            ai_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Ai],
                minimum_width: 460,
                maximum_height: 560,
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
            alarmclock_menu: default_alarmclock_menu(),
            dock_menu: default_dock_menu(),
            control_center_menu: default_control_center_menu(),
            ssh_menu: default_ssh_menu(),
            privacy_menu: default_privacy_menu(),
            plugin_panel_menu: default_plugin_panel_menu(),
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
                                widgets: vec![MenuWidget::CalendarGrid, MenuWidget::Weather],
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Launcher {
    /// Scripts the user opted into running at shell startup, with a
    /// per-script delay. Names match `ScriptsProvider` short names
    /// (e.g. `start-brave-ai`).
    pub autostart_scripts: Vec<ScriptAutostart>,
    /// Show the detail/preview pane beside the result list. When off,
    /// the result list always fills the launcher's full width.
    pub show_preview: bool,
    /// Compact rows (tighter padding) instead of the default
    /// comfortable density — fits more results per screen.
    pub compact_rows: bool,
    /// Lead app / window rows with a larger icon.
    pub large_app_icons: bool,
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            autostart_scripts: Vec::new(),
            show_preview: true,
            compact_rows: false,
            large_app_icons: true,
        }
    }
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

/// How the standalone mdock surface is presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DockStyle {
    /// A session-menu-style popup: opens on demand as a floating panel, closes
    /// on Esc / click-away. Not pinned to the screen edge.
    Popup,
    /// An edge-anchored dock (Always visible / Auto-hide), pinned to the screen.
    #[default]
    LayerShell,
}

/// Standalone-dock (mdock) reveal behaviour (only used in LayerShell style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DockBehavior {
    /// Always visible; reserves an exclusive zone.
    Always,
    /// Hidden; a thin edge trigger reveals it on hover (hydock style).
    #[default]
    AutoHide,
    /// Hidden; shown/hidden via `mshellctl dock toggle` / a keybind.
    Toggle,
}

/// Which screen edge the standalone mdock surface anchors to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DockPosition {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

/// mdock — the running/pinned app dock. Two modes: a bar-widget pill and a
/// standalone per-output layer-shell surface (always / auto-hide / toggle).
/// Tunables surfaced under Settings → Widgets → mdock. (Pins live in the
/// `pinned_apps_store` cache, not here.) serde key stays `dock`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Dock {
    /// App-icon pixel size.
    pub icon_size: u32,
    /// Hover tooltip listing the app + its open window titles.
    pub show_tooltips: bool,
    /// Include running apps that aren't pinned (off = pinned-only dock).
    pub show_running: bool,
    /// Per-app icon overrides — map a window class to an icon name or an
    /// absolute file path. For apps launched with a synthetic `--class` that
    /// has no matching `.desktop` (e.g. isolated browser profiles), which
    /// would otherwise fall back to a generic icon.
    pub icon_overrides: Vec<DockIconOverride>,

    // ── mdock additions ─────────────────────────────────────────────
    /// Show the dock as a bar-widget pill (the classic mode).
    pub in_bar: bool,
    /// Run the standalone dock surface.
    pub standalone: bool,
    /// How the standalone surface is presented (popup vs edge layer-shell).
    pub style: DockStyle,
    /// Standalone reveal behaviour (LayerShell style only).
    pub behavior: DockBehavior,
    /// Screen edge the standalone dock anchors to.
    pub position: DockPosition,
    /// App classes never shown in the dock (case-insensitive).
    pub ignore: Vec<String>,
    /// Spacing between dock items, px.
    pub spacing: u32,
    /// Show a small preview card on icon hover.
    pub hover_preview: bool,
    /// Show the separator between apps and the launcher button.
    pub separator: bool,
    /// Show the app-launcher button on the dock.
    pub launcher_enabled: bool,
    /// Launcher button icon (themed name or path).
    pub launcher_icon: String,
    /// Shell command the launcher button runs (empty = toggle mshell launcher).
    pub launcher_command: String,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            icon_size: 32,
            show_tooltips: true,
            show_running: true,
            icon_overrides: Vec::new(),
            in_bar: true,
            standalone: false,
            style: DockStyle::LayerShell,
            behavior: DockBehavior::AutoHide,
            position: DockPosition::Bottom,
            ignore: Vec::new(),
            spacing: 6,
            hover_preview: true,
            separator: true,
            launcher_enabled: true,
            launcher_icon: "view-app-grid-symbolic".to_string(),
            launcher_command: String::new(),
        }
    }
}

impl PatchField for DockStyle {
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

impl PatchField for DockBehavior {
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

impl PatchField for DockPosition {
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

/// A single dock icon override (`class` → `icon`).
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct DockIconOverride {
    /// Window class / app_id to match (case-insensitive).
    pub class: String,
    /// Icon name (themed) or an absolute file path / `file://` URI.
    pub icon: String,
}

/// System Tray bar widget (the StatusNotifierItem icon strip). Tunables
/// surfaced under Settings → Widgets → System Tray.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct SystemTray {
    /// Start with the tray icons revealed (expanded) instead of collapsed
    /// behind the tray button. The tray button still toggles them at runtime;
    /// this only sets the state the widget comes up in.
    pub default_expanded: bool,
}

/// GNU pass (password-store) launcher provider. Surfaced under
/// Settings → Launcher → Storage paths.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct Pass {
    /// Password-store directory. Empty = follow `$PASSWORD_STORE_DIR`,
    /// else `~/.password-store` (pass's own resolution order).
    pub store_path: String,
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
    /// When the script fires within a login session. Defaults to
    /// `EveryStart` so configs written before this field existed keep
    /// their original "runs on every restart" behaviour.
    pub trigger: AutostartTrigger,
    /// Extra arguments appended to the command (whitespace-separated).
    pub args: String,
    /// Working directory to run the script in (empty = inherit the
    /// session's). `~` is expanded at launch.
    pub working_dir: String,
}

/// How often an autostart script fires across a login session.
///
/// A *login session* is the graphical seat session — it survives
/// `systemctl --user restart mshell` but ends at logout. We tell the
/// two apart with a marker file under `$XDG_RUNTIME_DIR`, which systemd
/// tears down at logout, so `LoginOnce` runs exactly once per login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, Store, JsonSchema)]
pub enum AutostartTrigger {
    /// Run on every shell start, including in-session mshell restarts.
    /// The pre-existing behaviour, and the serde default for old configs.
    #[default]
    EveryStart,
    /// Run only on the first shell start of a login session; skipped on
    /// `systemctl --user restart mshell` until the next login.
    LoginOnce,
}

impl PatchField for AutostartTrigger {
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

impl AutostartTrigger {
    pub fn to_index(self) -> u32 {
        match self {
            AutostartTrigger::EveryStart => 0,
            AutostartTrigger::LoginOnce => 1,
        }
    }

    pub fn from_index(index: u32) -> Self {
        match index {
            1 => AutostartTrigger::LoginOnce,
            _ => AutostartTrigger::EveryStart,
        }
    }
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
    /// Max number of (most-recent) notifications the history menu renders.
    /// Persisted history can grow into the hundreds; rendering all of them
    /// rebuilds a large list model on every open. 0 = unlimited.
    #[serde(default = "default_history_limit")]
    pub history_limit: u32,
    /// Show a shrinking bar across the top of each popup toast counting
    /// down its remaining on-screen time. On by default.
    #[serde(default = "default_true")]
    pub show_timeout_bar: bool,
    /// How long (ms) a popup toast stays on screen before auto-dismiss
    /// (an app-supplied `expire_timeout` shorter than this still wins).
    /// Also the duration the timeout bar animates over.
    #[serde(default = "default_popup_duration_ms")]
    pub popup_duration_ms: u32,
}

fn default_history_limit() -> u32 {
    200
}

fn default_popup_duration_ms() -> u32 {
    5000
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
            history_limit: default_history_limit(),
            show_timeout_bar: true,
            popup_duration_ms: default_popup_duration_ms(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, Store, JsonSchema)]
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
    /// Auto-fetch a daily image-of-the-day (Bing / NASA) on login + periodically.
    pub daily_wallpaper_enabled: bool,
    /// Daily-wallpaper source: `"bing"` or `"nasa"`.
    pub daily_wallpaper_source: String,
    /// Bing market locale for the daily wallpaper (e.g. `en-US`); unused for NASA.
    pub daily_wallpaper_locale: String,
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
            daily_wallpaper_enabled: false,
            daily_wallpaper_source: "bing".to_string(),
            daily_wallpaper_locale: "".to_string(),
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
    /// Master on/off. When `false` the bar is fully inert — never shown,
    /// not even on edge-hover (a true "disable", distinct from
    /// `reveal_by_default = false` which is auto-hide).
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub minimum_height: i32,
    /// `true` → always visible. `false` → auto-hide: hidden, slides in
    /// when the pointer reaches the bar's screen edge.
    pub reveal_by_default: bool,
    /// Auto-hide only: ms the pointer must leave the bar before it slides
    /// back out (debounce so it doesn't snap away the instant you move
    /// off it).
    #[serde(default = "default_auto_hide_delay_ms")]
    pub auto_hide_delay_ms: i32,
    pub left_widgets: Vec<BarWidget>,
    pub center_widgets: Vec<BarWidget>,
    pub right_widgets: Vec<BarWidget>,
    /// Widgets collapsed into the [`BarWidget::HiddenBar`] drawer on this
    /// bar. The Hidden Bar pill (placed in one of the slots above) renders
    /// these inside a slide revealer; everything else stays visible.
    pub hidden_widgets: Vec<BarWidget>,
}

fn default_true() -> bool {
    true
}

fn default_auto_hide_delay_ms() -> i32 {
    400
}

impl Default for HorizontalBar {
    fn default() -> Self {
        Self {
            enabled: true,
            minimum_height: 0,
            reveal_by_default: true,
            auto_hide_delay_ms: default_auto_hide_delay_ms(),
            left_widgets: Vec::new(),
            center_widgets: Vec::new(),
            right_widgets: Vec::new(),
            hidden_widgets: Vec::new(),
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

/// Proxy mode for the network settings.
///
/// `None` disables the proxy (removes the environment.d file).
/// `Manual` writes explicit `http_proxy` / `https_proxy` / `all_proxy` /
/// `no_proxy` env vars to `~/.config/environment.d/99-margo-proxy.conf`.
/// `Automatic` stores a PAC URL for reference; margo has no runtime PAC
/// interpreter — apps launched after setting this mode need to support
/// `auto_proxy`/`WPAD` on their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, Store, JsonSchema)]
pub enum ProxyMode {
    /// No proxy — the environment.d file is removed.
    #[default]
    None,
    /// Manual proxy — host:port strings are written as env vars.
    Manual,
    /// Automatic (PAC URL) — stored for reference only (not applied as env).
    Automatic,
}

impl PatchField for ProxyMode {
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

impl ProxyMode {
    pub fn to_index(self) -> u32 {
        match self {
            ProxyMode::None => 0,
            ProxyMode::Manual => 1,
            ProxyMode::Automatic => 2,
        }
    }

    pub fn from_index(idx: u32) -> Self {
        match idx {
            1 => ProxyMode::Manual,
            2 => ProxyMode::Automatic,
            _ => ProxyMode::None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ProxyMode::None => "None",
            ProxyMode::Manual => "Manual",
            ProxyMode::Automatic => "Automatic (PAC)",
        }
    }

    pub fn display_names() -> Vec<&'static str> {
        vec!["None", "Manual", "Automatic (PAC)"]
    }
}

/// Network-level settings — currently proxy only.
///
/// Proxy fields are written to
/// `~/.config/environment.d/99-margo-proxy.conf` so apps launched in the
/// next session (and the current session via `set_var`) inherit them.
/// This is a best-effort applier: margo has no runtime system-wide proxy
/// daemon; running apps keep their current proxy environment.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct NetworkConfig {
    /// Proxy mode selection.
    pub proxy_mode: ProxyMode,
    /// HTTP proxy in `host:port` form (no scheme prefix).
    pub proxy_http: String,
    /// HTTPS proxy in `host:port` form.
    pub proxy_https: String,
    /// SOCKS5 proxy in `host:port` form.
    pub proxy_socks: String,
    /// Comma-separated list of hosts/domains that bypass the proxy.
    pub proxy_ignore: String,
    /// PAC URL used when `proxy_mode == Automatic`.
    pub proxy_pac_url: String,
}

/// Home-network login automation (Settings → Network · Network Console menu).
///
/// Native replacement for the external `home-net-vpn` login script: at login,
/// bring up a saved Wi-Fi connection then connect Mullvad (with Blocky as the
/// no-VPN DNS fallback). The same engine backs the menu's "Connect home network
/// now" button. Off by default.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct LoginNetworkConfig {
    /// Master switch for the at-login reconcile.
    pub enabled: bool,
    /// NetworkManager connection NAME to bring up (e.g. "Ken_5"). Empty = skip
    /// the Wi-Fi step (only do the VPN part).
    pub wifi_connection: String,
    /// After Wi-Fi is up, connect Mullvad.
    pub connect_vpn: bool,
    /// Couple Blocky as the DNS fallback — stop it while the VPN is up
    /// (needs passwordless sudo for `systemctl`; skipped with a toast otherwise).
    pub couple_blocky: bool,
    /// Seconds to wait after shell start before reconciling (let NM settle).
    pub delay_secs: u32,
}

impl Default for LoginNetworkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            wifi_connection: String::new(),
            connect_vpn: true,
            couple_blocky: true,
            delay_secs: 4,
        }
    }
}

/// Shell file-logging (Settings → Logging). Mirrors margo's compositor knobs:
/// the shell writes per-session files to ~/.local/state/margo/logs (mshell-*.log,
/// last `keep_sessions` kept), driven by the shared `margo-logging` engine.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct LoggingConfig {
    /// Write log files at all. On by default so the last few sessions are
    /// always on disk for diagnosis.
    pub enabled: bool,
    /// Level: error | warn | info | debug | trace.
    pub level: String,
    /// How many session files to keep on disk.
    pub keep_sessions: u32,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "info".to_string(),
            keep_sessions: 3,
        }
    }
}

/// Bluetooth auto-connect + audio-routing settings (Settings → Bluetooth).
///
/// Replaces the external `bt-autoconnect.service` + `bt-autoconnect-once`
/// + `bluetooth_toggle` scripts: at login the shell waits
/// `autoconnect_delay_secs`, then tries each device in `devices` in order
/// until one connects, and (optionally) routes audio to it. The same
/// engine backs the `mshellctl bluetooth toggle` keybind action.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct BluetoothConfig {
    /// Master switch for auto-connecting at login.
    pub autoconnect_enabled: bool,
    /// Seconds to wait after shell start before the first connect attempt —
    /// lets the adapter + PipeWire/WirePlumber settle. Default 6.
    pub autoconnect_delay_secs: u32,
    /// Devices to try, in order, until one connects. First success wins.
    pub devices: Vec<BluetoothDevice>,
    /// On connect, make the device the default audio OUTPUT (sink).
    pub route_audio_output: bool,
    /// Also make it the default INPUT (mic). Off by default: forcing a
    /// headset mic on usually drops the codec from A2DP to HSP/HFP and
    /// noticeably degrades playback quality.
    pub route_audio_input: bool,
    /// Pop desktop notifications on connect / disconnect / routing.
    pub notifications: bool,
}

impl Default for BluetoothConfig {
    fn default() -> Self {
        Self {
            autoconnect_enabled: false,
            autoconnect_delay_secs: 6,
            devices: Vec::new(),
            route_audio_output: true,
            route_audio_input: false,
            notifications: true,
        }
    }
}

/// One auto-connect Bluetooth device: a MAC address plus a friendly label
/// (the label is display-only; matching is by MAC, case-insensitive).
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct BluetoothDevice {
    /// Device MAC address, `AA:BB:CC:DD:EE:FF` (case-insensitive).
    pub mac: String,
    /// Friendly name shown in Settings (e.g. `SL4P`).
    pub name: String,
}

/// Power-management settings surfaced on the Settings → Power page.
///
/// `low_battery_warning` enables a desktop-notification toast when the
/// battery falls at or below `low_battery_threshold` percent while on
/// battery power. The threshold is a percent value (1–100); the default
/// of 15 matches common platform defaults.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct PowerConfig {
    /// Pop a toast notification when battery drops to or below
    /// `low_battery_threshold` while on battery.
    pub low_battery_warning: bool,
    /// Percent threshold for the low-battery toast (1–100, default 15).
    pub low_battery_threshold: u32,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            low_battery_warning: true,
            low_battery_threshold: 15,
        }
    }
}

/// Privacy settings surfaced on the Settings → Privacy page.
///
/// `remember_recent` controls whether recently-used files are retained in the
/// GTK `RecentManager` list. When `false` the list is cleared immediately on
/// the page being shown and when the setting is toggled off. This is
/// best-effort: individual apps may record their own recent-file lists
/// independently of GtkRecentManager.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct PrivacyConfig {
    /// Remember recently-used files in GtkRecentManager. Default `true`.
    pub remember_recent: bool,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            remember_recent: true,
        }
    }
}

/// Audio settings. Currently controls the optional HDMI / DisplayPort output
/// filter — when `hide_hdmi_outputs` is `true`, sinks whose node name or
/// description contains "hdmi", "displayport", or "display port" are hidden
/// from the output-device list, the output-switch cycle, and the audio
/// dashboard menu's device picker. Default `false` so the behaviour is
/// unchanged for users who haven't opted in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct AudioConfig {
    /// Hide HDMI / DisplayPort audio sinks from the output list and switcher.
    /// Matched by node name or description; default `false`.
    pub hide_hdmi_outputs: bool,
    /// Restore the default output + input volumes to the levels below on shell
    /// startup. PipeWire doesn't persist volumes across reboots, so without
    /// this the levels drift; with it, every login lands on your chosen
    /// defaults. Off by default (no surprise volume changes).
    pub restore_volume_on_start: bool,
    /// Default output (speaker) level as a percentage `0..=100`. Applied at
    /// startup when `restore_volume_on_start` is on.
    pub default_output_volume: i32,
    /// Default input (microphone) level as a percentage `0..=100`.
    pub default_input_volume: i32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            hide_hdmi_outputs: false,
            restore_volume_on_start: false,
            default_output_volume: 50,
            default_input_volume: 50,
        }
    }
}

/// Control Center tile visibility and order.
///
/// All tiles are shown by default (`true`). Set a tile to `false` via the
/// Control Center's edit mode (the pencil icon in the header) to hide it
/// from the normal grid view. `tile_order` controls the order tiles appear in
/// the grid (first entry = top-left, reading left-to-right). Tiles not present
/// in `tile_order` (e.g. added in a future release) append at the end so
/// nothing silently disappears after an upgrade.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct ControlCenterConfig {
    pub wifi: bool,
    pub bluetooth: bool,
    pub audio_out: bool,
    pub mic: bool,
    pub battery: bool,
    pub keep_awake: bool,
    pub dnd: bool,
    pub dark_mode: bool,
    pub night_light: bool,
    pub color_picker: bool,
    pub disk: bool,
    pub airplane_mode: bool,
    pub vpn: bool,
    pub valent: bool,
    pub ufw: bool,
    pub podman: bool,
    /// Display order of tiles in the grid. Each entry is a tile-id string
    /// (e.g. `"wifi"`, `"bluetooth"`, …). Unknown ids are skipped; ids
    /// not listed here are appended at the end in canonical order.
    #[serde(default = "default_cc_tile_order")]
    pub tile_order: Vec<String>,
    /// Tile ids that should span 2 columns in the grid (GNOME-style wide).
    /// Default is empty (all tiles are 1 column = uniform width). A tile
    /// whose id appears here spans both columns and starts on a fresh row.
    #[serde(default)]
    pub wide_tiles: Vec<String>,
    /// Header battery chip (icon + %). On by default; hidden anyway when
    /// the machine has no battery.
    #[serde(default = "default_true")]
    pub show_battery_chip: bool,
    /// Power-profile segmented control (Saver / Balanced / Performance)
    /// above the sliders. On by default.
    #[serde(default = "default_true")]
    pub show_power_profile: bool,
    /// Compact media player (title + transport) above the tile grid,
    /// shown only while a player is active. On by default.
    #[serde(default = "default_true")]
    pub show_media: bool,
}

fn default_cc_tile_order() -> Vec<String> {
    vec![
        "wifi".to_string(),
        "bluetooth".to_string(),
        "audio_out".to_string(),
        "mic".to_string(),
        "vpn".to_string(),
        "valent".to_string(),
        "battery".to_string(),
        "keep_awake".to_string(),
        "dnd".to_string(),
        "airplane_mode".to_string(),
        "dark_mode".to_string(),
        "night_light".to_string(),
        "color_picker".to_string(),
        "disk".to_string(),
        "ufw".to_string(),
        "podman".to_string(),
    ]
}

impl Default for ControlCenterConfig {
    fn default() -> Self {
        Self {
            wifi: true,
            bluetooth: true,
            audio_out: true,
            mic: true,
            battery: true,
            keep_awake: true,
            dnd: true,
            dark_mode: true,
            night_light: true,
            color_picker: true,
            disk: true,
            airplane_mode: true,
            vpn: true,
            valent: true,
            ufw: true,
            podman: true,
            tile_order: default_cc_tile_order(),
            wide_tiles: Vec::new(),
            show_battery_chip: true,
            show_power_profile: true,
            show_media: true,
        }
    }
}

#[cfg(test)]
mod schema_tests {
    use super::Config;
    use super::{DockBehavior, DockPosition};

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
        let cfg: Config = serde_yaml::from_str("general:\n  clock_format_24_h: true\n").unwrap();
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

    /// An old config that only knew the original dock fields still loads, with
    /// the mdock additions defaulted.
    #[test]
    fn dock_old_config_loads_with_new_defaults() {
        let yaml = "dock:\n  icon_size: 48\n  show_running: false\n";
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.dock.icon_size, 48);
        assert!(!cfg.dock.show_running);
        assert!(cfg.dock.in_bar);
        assert!(!cfg.dock.standalone);
        assert_eq!(cfg.dock.behavior, DockBehavior::AutoHide);
        assert_eq!(cfg.dock.position, DockPosition::Bottom);
        assert!(cfg.dock.launcher_enabled);
    }
}
