//! Bar config: the bars, their widget slots and per-pill widget settings.

use super::*;

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

pub(super) fn default_bar_slide_ms() -> u32 {
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
    /// Lyrics bar pill behaviour (whether the current line shows in the bar).
    #[serde(default)]
    pub lyrics: LyricsBarWidget,
    /// Tune bar pill behaviour (which elements show, cover/label size).
    #[serde(default)]
    pub mtune: TuneBarWidget,
    /// Hidden Bar drawer behaviour (hover-expand, auto-collapse, …) for the
    /// bar's default (`!HiddenBar`) drawer.
    pub hidden_bar: HiddenBarConfig,
    /// Additional named Hidden Bar drawers, each with its own widget list and
    /// behaviour. Referenced from a bar slot via `!HiddenBarNamed <name>` and
    /// targetable from the CLI via `mshellctl hidden-bar <verb> <name>`.
    pub hidden_bars: Vec<HiddenBarConfig>,
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
    /// Raise a toast when a sensor first goes active. Independent of the global
    /// `toasts.enabled` switch — a privacy alert has its own gate.
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

/// Which cat sprite set the [`BarWidget::Catwalk`] pill animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CatStyle {
    /// The noctalia cat — 4 walk frames + 4 idle frames.
    Noctalia,
    /// RunCat — the classic macOS running cat (5 run frames + a sleeping
    /// idle pose). Ported from CatWalk by Driglu4it, originally RunCat by
    /// Kyome.
    #[default]
    RunCat,
}

impl PatchField for CatStyle {
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

/// What the catwalk pill shows: the cat, the CPU %, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CatwalkDisplay {
    /// Just the animated cat.
    #[default]
    Icon,
    /// Just the CPU percentage (severity-coloured), no cat.
    Text,
    /// The cat with the CPU percentage beside it.
    Both,
}

impl PatchField for CatwalkDisplay {
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

/// Settings for the [`BarWidget::Catwalk`] animated-cat pill (port of the
/// noctalia catwalk plugin, with the RunCat sprite set + CPU-% readout from
/// the DMS/Plasma CatWalk widgets).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct CatwalkConfig {
    /// CPU busy% below which the cat idles ("Zz"); above it walks, speeding up
    /// with load. 5–25 is sensible.
    pub minimum_threshold: u32,
    /// Drop the pill background so the cat floats on the bar.
    pub hide_background: bool,
    /// Which cat sprite set to animate.
    pub style: CatStyle,
    /// Show the cat, the CPU %, or both.
    pub display: CatwalkDisplay,
    /// Sprite size in px (12–48).
    pub size: u32,
    /// How often to re-sample CPU load, in seconds (1–10).
    pub poll_secs: u32,
}

impl Default for CatwalkConfig {
    fn default() -> Self {
        Self {
            minimum_threshold: 10,
            hide_background: false,
            style: CatStyle::default(),
            display: CatwalkDisplay::default(),
            size: 22,
            poll_secs: 1,
        }
    }
}

/// Behaviour knobs for the [`BarWidget::HiddenBar`] drawer (the widgets it
/// collapses come from each bar's `hidden_widgets` list).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct HiddenBarConfig {
    /// Drawer name. Empty for the bar's default (`!HiddenBar`) drawer; set
    /// for a named drawer (an entry in `bars.widgets.hidden_bars`, referenced
    /// from a slot via `!HiddenBarNamed <name>` and targetable by
    /// `mshellctl hidden-bar <verb> <name>`).
    pub name: String,
    /// Widgets collapsed into this drawer. Used only for named drawers; the
    /// default drawer instead reads each bar's own `hidden_widgets` list.
    pub widgets: Vec<BarWidget>,
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
            name: String::new(),
            widgets: Vec::new(),
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
    /// Comma-separated sensitive host capabilities this plugin's WASM panel is
    /// granted (`process`, `network`, `clipboard`). Empty = none (deny by
    /// default). Set from the manifest; only meaningful with `panel_entry`.
    pub panel_capabilities: String,
    /// The plugin's panel/menu surface min width + max height (a per-plugin
    /// preference edited in the plugin's own settings, applied to the shared
    /// plugin-menu surface on open). 0 = use the surface default.
    pub panel_min_width: i32,
    pub panel_max_height: i32,
    /// Secret settings the commands above reference as `${MPLUGIN_SECRET_*}`
    /// (plugin-derived widgets only). The runner reads each from the keyring
    /// and exports it to the child, so a secret is never in a command line.
    pub secret_env: Vec<PluginSecretEnv>,
}

/// One secret a plugin command needs in its environment: `var` is filled
/// from the keyring entry of setting `key` of plugin `plugin`.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct PluginSecretEnv {
    pub var: String,
    pub plugin: String,
    pub key: String,
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
    /// Secrets `exec` references (see [`CustomWidgetConfig::secret_env`]).
    pub secret_env: Vec<PluginSecretEnv>,
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
    /// Fetch the Arch Linux news feed when the panel opens and flag
    /// posts published since the last full upgrade before upgrading.
    pub check_news: bool,
    /// How the panel's Update button runs the upgrade: in a terminal
    /// (interactive, the default) or in the background through polkit
    /// with a live log + result notification.
    pub update_mode: SystemUpdateMode,
}

/// How [`SystemUpdateBarWidget`] runs an upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SystemUpdateMode {
    /// A terminal window with the usual interactive `-Syu` (prompts,
    /// PKGBUILD review and sudo work as on the command line).
    #[default]
    Terminal,
    /// Detached and non-interactive: `pkexec` asks for the password,
    /// the panel tails the log, a notification reports the result.
    Background,
}

impl PatchField for SystemUpdateMode {
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

impl Default for SystemUpdateBarWidget {
    fn default() -> Self {
        Self {
            check_interval_minutes: 180,
            check_repo: true,
            check_aur: true,
            check_flatpak: true,
            check_news: true,
            update_mode: SystemUpdateMode::default(),
        }
    }
}

/// Lyrics bar pill behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct LyricsBarWidget {
    /// Show the current synced line scrolling in the bar. When off, the pill is
    /// just an icon (tinted when lyrics are available) and the words live only
    /// in the menu.
    #[serde(default = "default_true")]
    pub show_line_in_bar: bool,
    /// Max width of the scrolling line in the bar, in characters. The label
    /// ellipsizes past this so a long line can't stretch the bar. Clamped
    /// 8..=120 when applied.
    #[serde(default = "default_lyrics_max_width_chars")]
    pub max_width_chars: i32,
}

pub(super) fn default_lyrics_max_width_chars() -> i32 {
    32
}

impl Default for LyricsBarWidget {
    fn default() -> Self {
        Self {
            show_line_in_bar: true,
            max_width_chars: default_lyrics_max_width_chars(),
        }
    }
}

/// Tune bar pill behaviour: which elements the pill shows and how large the
/// cover art / title read. The menu (`MenuType::Mtune`) has its own generic
/// position/width/height page — this only covers the pill itself.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct TuneBarWidget {
    /// Show the cover art thumbnail (falls back to a play/pause glyph when a
    /// track has none). Always shown while mtune isn't running — the pill's
    /// only affordance is then "click this glyph to launch".
    #[serde(default = "default_true")]
    pub show_cover: bool,
    /// Show the "Title — Artist" label.
    #[serde(default = "default_true")]
    pub show_track_info: bool,
    /// Show the 1-based queue position (e.g. "3").
    #[serde(default = "default_true")]
    pub show_queue_number: bool,
    /// Show the elapsed / total time readout.
    #[serde(default = "default_true")]
    pub show_time: bool,
    /// Show previous / play-pause / next buttons in the pill. Right-click
    /// anywhere on the pill still toggles play/pause either way.
    #[serde(default = "default_true")]
    pub show_transport: bool,
    /// Show the "×N" badge while repeat-each is active.
    #[serde(default = "default_true")]
    pub show_repeat_badge: bool,
    /// Show a thin playback-position line under the pill.
    #[serde(default = "default_true")]
    pub show_progress_bar: bool,
    /// Show the queue's remaining time (e.g. "-12:34"), summed across every
    /// track left in the queue — not just the current track.
    #[serde(default = "default_true")]
    pub show_playlist_remaining: bool,
    /// Cover art size in the bar, in pixels. Clamped 12..=28 when applied.
    #[serde(default = "default_tune_cover_size_px")]
    pub cover_size_px: i32,
    /// Max width of the "Title — Artist" label, in characters — the label
    /// ellipsizes past this so a long title can't stretch the bar. Clamped
    /// 10..=80 when applied.
    #[serde(default = "default_tune_label_max_width_chars")]
    pub label_max_width_chars: i32,
}

pub(super) fn default_tune_cover_size_px() -> i32 {
    16
}

pub(super) fn default_tune_label_max_width_chars() -> i32 {
    40
}

impl Default for TuneBarWidget {
    fn default() -> Self {
        Self {
            show_cover: true,
            show_track_info: true,
            show_queue_number: true,
            show_time: true,
            show_transport: true,
            show_repeat_badge: true,
            show_progress_bar: true,
            show_playlist_remaining: true,
            cover_size_px: default_tune_cover_size_px(),
            label_max_width_chars: default_tune_label_max_width_chars(),
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

pub(super) fn default_true() -> bool {
    true
}

pub(super) fn default_auto_hide_delay_ms() -> i32 {
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
