use crate::schema::bar_widgets::BarWidget;
use crate::schema::clipboard::Clipboard;
use crate::schema::content_fit::ContentFit;
use crate::schema::location_query::{LocationQueryConfig, OrdF64};
use crate::schema::menu_widgets::{
    ContainerConfig, MenuWidget, PanelHeaderConfig, QuickActionWidget, QuickActionsConfig,
    SectionLabelConfig, SpacerConfig,
};
use crate::schema::position::{Orientation, OsdPosition, Position};
use crate::schema::temperature::TemperatureUnitConfig;
use crate::schema::themes::{
    MatugenContrast, MatugenMode, MatugenPreference, MatugenType, Themes, WindowOpacity,
};
use crate::schema::wallpaper::{ContrastFilterStrength, ThemeFilterStrength};
use reactive_stores::{KeyMap, Patch, PatchField, Store, StorePath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod bars;
mod crashes;
mod dock;
mod launcher;
mod drive_health;
mod menus;
mod notifications;
mod session;
mod osd;
mod social;
mod system;
mod theme;
mod time;
mod valent;
pub use bars::*;
pub use crashes::*;
pub use dock::*;
pub use launcher::*;
pub use drive_health::*;
pub use menus::*;
pub use notifications::*;
pub use session::*;
pub use osd::*;
pub use social::*;
pub use system::*;
pub use theme::*;
pub use time::*;
pub use valent::*;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    pub general: General,
    pub theme: Theme,
    pub bars: Bars,
    pub menus: Menus,
    pub notifications: Notifications,
    pub crashes: CrashesConfig,
    pub drive_health: DriveHealthConfig,
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
    pub calendars: Calendars,
    pub network: NetworkConfig,
    pub login_network: LoginNetworkConfig,
    pub trusted_networks: TrustedNetworksConfig,
    pub bluetooth: BluetoothConfig,
    pub power: PowerConfig,
    pub privacy: PrivacyConfig,
    pub audio: AudioConfig,
    pub control_center: ControlCenterConfig,
    pub logging: LoggingConfig,
    pub osd: Osd,
    pub toasts: Toasts,
    pub game_mode: GameMode,
    pub pomodoro: Pomodoro,
    pub eyecare: EyeCare,
    /// Notes Hub Markdown folder + AI/git options.
    pub notes: NotesConfig,
}

/// Transient state-change toasts — a small opaque corner card that auto-
/// dismisses (DESIGN.md §20: a toast stays opaque regardless of the surface-
/// opacity knob). Distinct from the notification popups and the
/// volume/brightness OSD: these announce *system state changes* (AC power,
/// lock keys, keyboard layout, default audio device, VPN, now-playing) plus
/// a battery warning ladder. A single central producer
/// (`mshell-osd::toast_producer`) subscribes once and broadcasts to the
/// per-output toast surfaces. Each event carries its own on/off switch;
/// `enabled` is the master gate.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Toasts {
    /// Master switch for the whole toast surface.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Toast when AC power is plugged in / unplugged.
    #[serde(default = "default_true")]
    pub charging: bool,
    /// Toast when Caps Lock / Num Lock toggles.
    #[serde(default = "default_true")]
    pub lock_keys: bool,
    /// Toast when the keyboard layout changes.
    #[serde(default = "default_true")]
    pub kb_layout: bool,
    /// Toast when margo's keymode (`setkeymode`) changes. Off by default so
    /// existing setups see no new toast; turn it on to get a mode OSD.
    #[serde(default)]
    pub keymode: bool,
    /// Toast when the default audio output / input device changes.
    #[serde(default = "default_true")]
    pub audio_device: bool,
    /// Toast when the VPN tunnel connects / disconnects.
    #[serde(default = "default_true")]
    pub vpn: bool,
    /// Toast on every now-playing track change. Off by default — it's noisy.
    #[serde(default)]
    pub now_playing: bool,
    /// Toast when the primary network link connects / disconnects (Wi-Fi or wired).
    #[serde(default = "default_true")]
    pub network: bool,
    /// Toast when a Bluetooth device connects / disconnects.
    #[serde(default = "default_true")]
    pub bluetooth: bool,
    /// Toast when the power profile changes (performance / balanced / power-saver).
    #[serde(default = "default_true")]
    pub power_profile: bool,
    /// Toast when Do Not Disturb is turned on / off.
    #[serde(default = "default_true")]
    pub dnd: bool,
    /// Toast when the idle inhibitor ("keep awake") is turned on / off.
    #[serde(default = "default_true")]
    pub idle_inhibitor: bool,
    /// Toast when Game Mode is engaged / released.
    #[serde(default = "default_true")]
    pub game_mode: bool,
    /// Toast when a removable drive is plugged in / removed. A drive
    /// unplugged while still mounted always warns, regardless.
    #[serde(default = "default_true")]
    pub removable_drives: bool,
    /// Toast as the battery crosses a warning / critical level.
    #[serde(default = "default_true")]
    pub battery: bool,
    /// Battery warning thresholds (percent). A warn toast fires once as the
    /// charge crosses each level downward while discharging; the ladder
    /// re-arms on recharge. Order doesn't matter.
    #[serde(default = "default_battery_warn_levels")]
    pub battery_warn_levels: Vec<u8>,
    /// Battery critical threshold (percent) — fires a danger toast. `0`
    /// disables the critical step (warning levels still apply).
    #[serde(default = "default_battery_critical_level")]
    pub battery_critical_level: u8,
    /// Which screen edge / corner the toast card docks against (like the OSD,
    /// plus the four corners). Read when the toast surfaces are created — a
    /// shell restart picks up a change.
    #[serde(default = "default_toast_position")]
    pub position: Position,
    /// Margin from the docked edge(s) in px.
    #[serde(default = "default_toast_distance")]
    pub distance: i32,
    /// Fixed card width in px; `0` sizes the card to its content.
    #[serde(default)]
    pub width: i32,
}

impl Default for Toasts {
    fn default() -> Self {
        Self {
            enabled: true,
            charging: true,
            lock_keys: true,
            kb_layout: true,
            keymode: false,
            audio_device: true,
            vpn: true,
            now_playing: false,
            network: true,
            bluetooth: true,
            power_profile: true,
            dnd: true,
            idle_inhibitor: true,
            game_mode: true,
            removable_drives: true,
            battery: true,
            battery_warn_levels: default_battery_warn_levels(),
            battery_critical_level: default_battery_critical_level(),
            position: default_toast_position(),
            distance: default_toast_distance(),
            width: 0,
        }
    }
}

fn default_battery_warn_levels() -> Vec<u8> {
    vec![20, 10, 5]
}

fn default_battery_critical_level() -> u8 {
    3
}

fn default_toast_position() -> Position {
    Position::Top
}

fn default_toast_distance() -> i32 {
    56
}

/// Game Mode — a single toggle that drops compositor effects, silences
/// notifications, and keeps the session awake while gaming. Effects are
/// disabled via a managed `conf.d/gamemode.conf` fragment margo sources after
/// `config.conf` (so it wins) while active, and that is cleared on exit. The
/// `active` flag persists so the state survives a shell restart; the rest pick
/// *what* the mode affects.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct GameMode {
    /// Whether game mode is currently engaged.
    #[serde(default)]
    pub active: bool,
    /// Disable window + layer animations while active.
    #[serde(default = "default_true")]
    pub disable_animations: bool,
    /// Disable blur while active.
    #[serde(default = "default_true")]
    pub disable_blur: bool,
    /// Disable window + layer shadows while active.
    #[serde(default = "default_true")]
    pub disable_shadows: bool,
    /// Hold the idle inhibitor while active (no dim / lock / suspend).
    #[serde(default = "default_true")]
    pub inhibit_idle: bool,
    /// Turn on Do Not Disturb (suppress notification popups) while active.
    #[serde(default = "default_true")]
    pub enable_dnd: bool,
    /// Allow screen tearing (`allow_tearing = 1`) while active — lower latency
    /// for fullscreen games. Off by default.
    #[serde(default)]
    pub allow_tearing: bool,
    /// Switch to this power profile while active (`performance`,
    /// `balanced`, `power-saver`; power-profiles-daemon) and put the previous
    /// one back on exit. Empty (the default) leaves the profile alone.
    #[serde(default)]
    pub power_profile: String,
    /// Pause every playing media player (MPRIS) on enter. Not resumed on exit.
    #[serde(default)]
    pub pause_media: bool,
    /// Hide the bars while active; the bars that show by default come back
    /// on exit.
    #[serde(default)]
    pub hide_bars: bool,
    /// Hide the standalone (popup-style) dock while active; shown again on
    /// exit.
    #[serde(default)]
    pub hide_dock: bool,
    /// Suppress the volume / brightness / microphone OSD while active.
    #[serde(default)]
    pub disable_osd: bool,
    /// Tear down the shell's wallpaper windows while active (frees their
    /// GPU buffers); rebuilt on exit. A wallpaper margo draws is left alone.
    #[serde(default)]
    pub disable_wallpaper: bool,
    /// Dismiss the notification popups on screen on enter (the history in
    /// the notification centre is kept).
    #[serde(default)]
    pub clear_notifications: bool,
    /// Shell command run (`sh -c`) when game mode turns on. Empty = none.
    #[serde(default)]
    pub on_enter: String,
    /// Shell command run (`sh -c`) when game mode turns off. Empty = none.
    #[serde(default)]
    pub on_exit: String,
}

impl Default for GameMode {
    fn default() -> Self {
        Self {
            active: false,
            disable_animations: true,
            disable_blur: true,
            disable_shadows: true,
            inhibit_idle: true,
            enable_dnd: true,
            allow_tearing: false,
            power_profile: String::new(),
            pause_media: false,
            hide_bars: false,
            hide_dock: false,
            disable_osd: false,
            disable_wallpaper: false,
            clear_notifications: false,
            on_enter: String::new(),
            on_exit: String::new(),
        }
    }
}

/// One remote `.ics` calendar subscription (read-only). No OAuth — paste a
/// public or "secret address in iCal format" URL (Google / Outlook / any
/// CalDAV server that exposes one). Fetched + disk-cached by the `mcal` core.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct CalendarSubscription {
    /// Display name for the calendar.
    pub name: String,
    /// The `.ics` URL to fetch.
    pub url: String,
    /// Event colour as `#RRGGBB`; blank uses the theme default.
    pub color: String,
}

/// Calendar sources feeding the clock-menu agenda + grid marks. `local_dir`
/// holds `.ics` files / sub-directories; `subscriptions` are remote iCal URLs.
/// A blank `local_dir` resolves to `~/.config/margo/calendars`. Consumed by the
/// `mcal` core (see [`crate::schema::config::Calendars`]).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Calendars {
    /// Local calendar directory (blank = `~/.config/margo/calendars`).
    pub local_dir: String,
    /// Remote iCal subscriptions.
    pub subscriptions: Vec<CalendarSubscription>,
    /// Seconds between background refreshes.
    pub refresh_secs: u64,
}

impl Default for Calendars {
    fn default() -> Self {
        Self {
            local_dir: String::new(),
            subscriptions: Vec::new(),
            refresh_secs: 900,
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
    /// Hold the idle inhibitor while any MPRIS media player is playing, and
    /// restore the prior inhibitor state when playback stops. Off by default
    /// (opt-in — it overrides the idle/lock/suspend timers above while media
    /// plays). Ported from the DMS audio-inhibit plugin.
    pub inhibit_while_media: bool,
    /// While Keep Awake is on (toggle or timer), also hold a logind
    /// `handle-lid-switch` inhibitor, so closing the laptop lid (e.g. on an
    /// external monitor) doesn't suspend. Off by default. The media hold
    /// above never engages it. Released the moment Keep Awake turns off or
    /// mshell exits.
    pub inhibit_lid_switch: bool,
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
            inhibit_while_media: false,
            inhibit_lid_switch: false,
        }
    }
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
    /// IANA zones (`America/New_York`) compared in the Clock menu's World
    /// Clocks section, in display order. Edited from that section.
    pub world_clocks: Vec<String>,
    /// How many of the first `world_clocks` ride next to the bar clock
    /// (0–3; 0 = none). All of them are listed in its tooltip.
    pub world_clocks_in_bar: u32,
    /// Working day used for the "working hours" hints and the hour-strip
    /// shading, as hours `start..end` (an end before the start wraps).
    pub work_day_start: u32,
    pub work_day_end: u32,
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
            world_clocks: Vec::new(),
            world_clocks_in_bar: 0,
            work_day_start: 9,
            work_day_end: 17,
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
    /// Hand the wallpaper to margo to draw instead of painting it in a
    /// window per monitor, which holds about 100 MiB of GPU memory for a
    /// static image. Looks the same; mshell keeps its windows when margo is
    /// older or can't draw it (nested session, other compositor).
    pub compositor_draws: bool,
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
            compositor_draws: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Frame {
    pub enable_frame: bool,
    pub monitor_filter: Vec<String>,
    /// Inset gap (px) around each bar when frame drawing is OFF. With the
    /// frame disabled each bar paints its own floating panel inset from the
    /// screen edges by this much; the value is reserved in the layer-shell
    /// exclusive zone so windows tile below the floating bar (never under it).
    /// Ignored while the frame is enabled. 0 = flush to the edges.
    #[serde(default = "default_frameless_gap")]
    pub frameless_gap: i32,
}

fn default_frameless_gap() -> i32 {
    8
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            enable_frame: true,
            monitor_filter: vec![],
            frameless_gap: default_frameless_gap(),
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
        let cfg: Config = yaml_serde::from_str("{}").unwrap();
        assert_eq!(cfg, Config::default());
    }

    /// New notification button toggles: off / on by default.
    #[test]
    fn notification_button_defaults() {
        let cfg: Config = yaml_serde::from_str("{}").unwrap();
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
        let cfg: Config = yaml_serde::from_str(
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
        let cfg: Config = yaml_serde::from_str("general:\n  clock_format_24_h: true\n").unwrap();
        assert!(cfg.general.clock_format_24_h);
        // notifications section was never mentioned → defaults intact.
        assert!(cfg.notifications.show_close_button);
        assert!(!cfg.notifications.show_action_buttons);
    }

    /// Unknown keys are ignored (forward-compat: a newer config on an
    /// older binary must still load).
    #[test]
    fn unknown_top_level_key_is_ignored() {
        let parsed = yaml_serde::from_str::<Config>("a_key_from_the_future: 42\n");
        assert!(parsed.is_ok(), "unknown keys should not fail the parse");
    }

    /// Serialising the default config and reading it back is lossless.
    #[test]
    fn default_config_round_trips_through_yaml() {
        let def = Config::default();
        let yaml = yaml_serde::to_string(&def).unwrap();
        let back: Config = yaml_serde::from_str(&yaml).unwrap();
        assert_eq!(back, def);
    }

    /// An old config that only knew the original dock fields still loads, with
    /// the mdock additions defaulted.
    #[test]
    fn dock_old_config_loads_with_new_defaults() {
        let yaml = "dock:\n  icon_size: 48\n  show_running: false\n";
        let cfg: Config = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(cfg.dock.icon_size, 48);
        assert!(!cfg.dock.show_running);
        assert!(cfg.dock.in_bar);
        assert!(!cfg.dock.standalone);
        assert_eq!(cfg.dock.behavior, DockBehavior::AutoHide);
        assert_eq!(cfg.dock.position, DockPosition::Bottom);
        assert!(cfg.dock.launcher_enabled);
    }
}
