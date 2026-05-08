use serde::{Deserialize, Serialize};

// ── Modifier flags (mirrors WLR_MODIFIER_*) ─────────────────────────────────
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Modifiers: u32 {
        const SHIFT = 1 << 0;
        const CAPS  = 1 << 1;
        const CTRL  = 1 << 2;
        const ALT   = 1 << 3;
        const MOD2  = 1 << 4;
        const MOD3  = 1 << 5;
        const LOGO  = 1 << 6;
        const MOD5  = 1 << 7;
    }
}

impl serde::Serialize for Modifiers {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(self.bits())
    }
}

impl<'de> serde::Deserialize<'de> for Modifiers {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let bits = u32::deserialize(d)?;
        Ok(Modifiers::from_bits_truncate(bits))
    }
}

// ── Direction / edge enums ───────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TouchSwipe {
    Up,
    Down,
    Left,
    Right,
    UpLeft,
    UpRight,
    DownLeft,
    DownRight,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeOrCorner {
    Any,
    None,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Distance {
    Any,
    Short,
    Medium,
    Long,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FoldState {
    Unfold,
    Fold,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircleDir {
    Prev,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagAnimDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HotareaCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TearingMode {
    Disabled,
    WindowHint,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShortcutsInhibit {
    Disable,
    Enable,
    DenyNew,
}

// ── Key identifier ───────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyType {
    Sym,
    Code,
}

/// Up to three physical keycodes that map to the same keysym.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MultiKeycode {
    pub code1: u32,
    pub code2: u32,
    pub code3: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySymCode {
    pub keysym: u32,
    pub keycode: MultiKeycode,
    pub key_type: KeyType,
}

// ── Argument passed to action callbacks ─────────────────────────────────────
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Arg {
    pub i: i32,
    pub i2: i32,
    pub f: f32,
    pub f2: f32,
    pub v: Option<String>,
    pub v2: Option<String>,
    pub v3: Option<String>,
    pub ui: u32,
    pub ui2: u32,
}

// ── Action name (replaces C function pointer in stored bindings) ─────────────
pub type ActionName = String;

// ── Keybinding ───────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBinding {
    pub modifiers: Modifiers,
    pub key: KeySymCode,
    pub action: ActionName,
    pub arg: Arg,
    pub mode: String,
    pub is_common_mode: bool,
    pub is_default_mode: bool,
    pub lock_apply: bool,
    pub release_apply: bool,
    pub pass_apply: bool,
}

// ── Mouse / axis / switch / gesture bindings ────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseBinding {
    pub modifiers: Modifiers,
    pub button: u32,
    pub action: ActionName,
    pub arg: Arg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisBinding {
    pub modifiers: Modifiers,
    pub direction: u32,
    pub action: ActionName,
    pub arg: Arg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchBinding {
    pub fold: FoldState,
    pub action: ActionName,
    pub arg: Arg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GestureBinding {
    pub modifiers: Modifiers,
    pub motion: u32,
    pub fingers: u32,
    pub action: ActionName,
    pub arg: Arg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchGestureBinding {
    pub swipe: TouchSwipe,
    pub edge: EdgeOrCorner,
    pub distance: Distance,
    pub fingers: u32,
    pub action: ActionName,
    pub arg: Arg,
}

// ── Window rule ──────────────────────────────────────────────────────────────
/// Matches windows by app-id or title (regex).
///
/// Match semantics (niri-compatible):
/// * `id` and `title` are positive matches — both must match (logical AND)
/// * `exclude_id` and `exclude_title` are negative — if either matches the
///   rule is skipped, even when the positive matches succeed
/// * Empty strings or `None` are treated as "no constraint"
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowRule {
    pub id: Option<String>,
    pub title: Option<String>,
    pub exclude_id: Option<String>,
    pub exclude_title: Option<String>,
    pub tags: u32,
    /// Optional size constraints (logical px). `0` = unconstrained.
    pub min_width: i32,
    pub min_height: i32,
    pub max_width: i32,
    pub max_height: i32,
    /// `open_focused = Some(false)` opens the window without grabbing focus.
    /// Niri's `open-focused` semantic (more readable than `no_focus`).
    pub open_focused: Option<bool>,
    /// Hide window content from `wlr-screencopy` / `ext-screencopy` clients.
    /// Useful for password managers, secrets, etc. Renders solid black to
    /// the screencast stream while staying visible on the actual output.
    pub block_out_from_screencast: Option<bool>,
    pub is_floating: Option<bool>,
    pub is_fullscreen: Option<bool>,
    pub is_fake_fullscreen: Option<bool>,
    pub scroller_proportion: Option<f32>,
    pub animation_type_open: Option<String>,
    pub animation_type_close: Option<String>,
    pub layer_animation_type_open: Option<String>,
    pub layer_animation_type_close: Option<String>,
    pub no_border: Option<bool>,
    pub no_shadow: Option<bool>,
    pub no_radius: Option<bool>,
    pub no_animation: Option<bool>,
    pub border_width: Option<u32>,
    pub open_silent: Option<bool>,
    pub tag_silent: Option<bool>,
    pub is_named_scratchpad: Option<bool>,
    pub is_unglobal: Option<bool>,
    pub is_global: Option<bool>,
    pub is_overlay: Option<bool>,
    pub allow_shortcuts_inhibit: Option<bool>,
    pub ignore_maximize: Option<bool>,
    pub ignore_minimize: Option<bool>,
    pub no_size_hint: Option<bool>,
    pub idle_inhibit_when_focus: Option<bool>,
    pub monitor: Option<String>,
    pub offset_x: i32,
    pub offset_y: i32,
    pub width: i32,
    pub height: i32,
    pub no_focus: Option<bool>,
    pub no_fade_in: Option<bool>,
    pub no_fade_out: Option<bool>,
    pub no_force_center: Option<bool>,
    pub is_term: Option<bool>,
    pub allow_csd: Option<bool>,
    pub force_fake_maximize: Option<bool>,
    pub force_tiled_state: Option<bool>,
    pub force_tearing: Option<bool>,
    pub no_swallow: Option<bool>,
    pub no_blur: Option<bool>,
    pub canvas_no_tile: Option<bool>,
    pub focused_opacity: Option<f32>,
    pub unfocused_opacity: Option<f32>,
    pub scroller_proportion_single: Option<f32>,
    pub pass_mod: u32,
    pub pass_keysym: u32,
}

// ── Monitor rule ─────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonitorRule {
    pub name: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub transform: i32,
    pub scale: f32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub refresh: f32,
    pub vrr: bool,
    pub custom_mode: bool,
}

// ── Tag rule ─────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TagRule {
    pub id: i32,
    pub layout_name: Option<String>,
    pub monitor_name: Option<String>,
    pub monitor_make: Option<String>,
    pub monitor_model: Option<String>,
    pub monitor_serial: Option<String>,
    pub mfact: f32,
    pub nmaster: i32,
    pub no_render_border: bool,
    pub open_as_floating: bool,
    pub no_hide: bool,
}

// ── Layer rule ───────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LayerRule {
    pub layer_name: Option<String>,
    pub animation_type_open: Option<String>,
    pub animation_type_close: Option<String>,
    pub no_blur: bool,
    pub no_anim: bool,
    pub no_shadow: bool,
}

// ── Bezier animation curve (4 control-point components) ─────────────────────
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BezierCurve(pub [f64; 4]);

impl Default for BezierCurve {
    fn default() -> Self {
        BezierCurve([0.46, 1.0, 0.29, 0.99])
    }
}

// ── Blur parameters ──────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BlurParams {
    pub num_passes: i32,
    pub radius: i32,
    pub noise: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
}

impl Default for BlurParams {
    fn default() -> Self {
        BlurParams {
            num_passes: 1,
            radius: 5,
            noise: 0.02,
            brightness: 0.9,
            contrast: 0.9,
            saturation: 1.2,
        }
    }
}

// ── RGBA colour ──────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rgba(pub [f32; 4]);

impl Rgba {
    pub fn from_hex(hex: u32) -> Self {
        Rgba([
            ((hex >> 24) & 0xff) as f32 / 255.0,
            ((hex >> 16) & 0xff) as f32 / 255.0,
            ((hex >> 8) & 0xff) as f32 / 255.0,
            (hex & 0xff) as f32 / 255.0,
        ])
    }
}

// ── XKB rule names ───────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XkbRules {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
}

// ── Input acceleration profile ───────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccelProfile {
    None,
    Flat,
    Adaptive,
}

// ── Scroll / click methods ───────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollMethod {
    NoScroll,
    TwoFinger,
    Edge,
    OnButtonDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClickMethod {
    None,
    ButtonAreas,
    Clickfinger,
}

// ── Top-level Config ─────────────────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // animations
    pub animations: bool,
    pub layer_animations: bool,
    pub animation_type_open: String,
    pub animation_type_close: String,
    pub layer_animation_type_open: String,
    pub layer_animation_type_close: String,
    pub animation_fade_in: bool,
    pub animation_fade_out: bool,
    pub tag_animation_direction: TagAnimDirection,
    pub zoom_initial_ratio: f32,
    pub zoom_end_ratio: f32,
    pub fadein_begin_opacity: f32,
    pub fadeout_begin_opacity: f32,
    pub animation_duration_move: u32,
    pub animation_duration_open: u32,
    pub animation_duration_tag: u32,
    pub animation_duration_close: u32,
    pub animation_duration_focus: u32,
    pub animation_curve_move: BezierCurve,
    pub animation_curve_open: BezierCurve,
    pub animation_curve_tag: BezierCurve,
    pub animation_curve_close: BezierCurve,
    pub animation_curve_focus: BezierCurve,
    pub animation_curve_opafadein: BezierCurve,
    pub animation_curve_opafadeout: BezierCurve,
    pub animation_duration_canvas_pan: u32,
    pub animation_curve_canvas_pan: BezierCurve,
    pub animation_duration_canvas_zoom: u32,
    pub animation_curve_canvas_zoom: BezierCurve,
    /// Animation engine for the move/tile transition. `"bezier"` (default)
    /// drives the existing baked-curve + fixed-duration model; `"spring"`
    /// switches to a critically-damped harmonic oscillator that
    /// preserves velocity across mid-flight retargets and settles in a
    /// refresh-rate-invariant way. See `animation::spring` for the
    /// integrator. Move uses the actual spring physics (with velocity
    /// carry-over); other animation types (`animation_clock_open`/
    /// `_close`/`_tag`/`_focus`/`_layer`) sample a *spring-shaped*
    /// 0→1 curve baked at config-load time — same lookup-table
    /// machinery as bezier, just with a different shape.
    pub animation_clock_move: String,
    /// Animation engine for the open transition (zoom + fade-in).
    /// `"bezier"` (default) | `"spring"`. Spring mode produces a
    /// snappier-with-tiny-overshoot pop versus bezier's smoothed
    /// approach. Both honour `animation_duration_open`.
    pub animation_clock_open: String,
    /// Animation engine for the close transition (zoom + fade-out).
    /// Same `"bezier"` / `"spring"` choice as open. Spring mode
    /// gives a more "kicked-out" feel.
    pub animation_clock_close: String,
    /// Animation engine for tag-switch slide.
    pub animation_clock_tag: String,
    /// Animation engine for focus highlight cross-fade (border
    /// colour + opacity blend).
    pub animation_clock_focus: String,
    /// Animation engine for layer-surface open/close (bar,
    /// notifications, OSDs).
    pub animation_clock_layer: String,
    /// Spring constant. Higher → snappier pull toward the target.
    /// Niri's window-movement default is 800.
    pub animation_spring_stiffness: f64,
    /// Damping ratio. 1.0 = critical (no overshoot). Set <1 for bouncy
    /// feels, >1 for sluggish settles. 0.85–1.0 is the sane window-move
    /// range.
    pub animation_spring_damping_ratio: f64,
    /// Effective mass. 1.0 is canonical.
    pub animation_spring_mass: f64,

    // scroller
    pub scroller_structs: i32,
    pub scroller_default_proportion: f32,
    pub scroller_default_proportion_single: f32,
    pub scroller_ignore_proportion_single: bool,
    pub scroller_focus_center: bool,
    pub scroller_prefer_center: bool,
    pub scroller_prefer_overspread: bool,
    pub edge_scroller_pointer_focus: bool,
    pub scroller_proportion_presets: Vec<f32>,
    pub circle_layouts: Vec<String>,

    // layout / focus
    pub new_is_master: bool,
    pub default_layout: String,
    pub default_mfact: f32,
    pub default_nmaster: u32,
    pub center_master_overspread: bool,
    pub center_when_single_stack: bool,
    /// Auto-pick the best layout for each tag based on visible-client
    /// count and the monitor's aspect ratio. Off by default — when
    /// on, `arrange_monitor` updates the per-tag layout on every
    /// pass unless the user has explicitly called `setlayout` /
    /// `switch_layout` on that tag (`Pertag::user_picked_layout`
    /// sticky bit).
    pub auto_layout: bool,
    pub focus_cross_monitor: bool,
    pub exchange_cross_monitor: bool,
    pub scratchpad_cross_monitor: bool,
    pub focus_cross_tag: bool,
    pub view_current_to_back: bool,
    pub no_border_when_single: bool,
    pub no_radius_when_single: bool,
    pub snap_distance: i32,
    pub enable_floating_snap: bool,
    pub drag_tile_to_tile: bool,
    pub swipe_min_threshold: u32,
    pub focused_opacity: f32,
    pub unfocused_opacity: f32,

    // hotarea
    pub hotarea_size: u32,
    pub hotarea_corner: HotareaCorner,
    pub enable_hotarea: bool,

    // overview
    pub ov_tab_mode: u32,
    pub overview_gap_inner: i32,
    pub overview_gap_outer: i32,

    // gaps / borders
    pub enable_gaps: bool,
    pub smartgaps: bool,
    pub gappih: u32,
    pub gappiv: u32,
    pub gappoh: u32,
    pub gappov: u32,
    pub borderpx: u32,

    // canvas
    pub canvas_tiling: bool,
    pub canvas_tiling_gap: i32,
    pub canvas_pan_on_kill: bool,
    pub canvas_anchor_animate: bool,

    // tags
    pub tag_carousel: bool,

    // scratchpad
    pub scratchpad_width_ratio: f32,
    pub scratchpad_height_ratio: f32,
    pub single_scratchpad: bool,

    // colours
    pub rootcolor: Rgba,
    pub bordercolor: Rgba,
    pub focuscolor: Rgba,
    pub maximizescreencolor: Rgba,
    pub urgentcolor: Rgba,
    pub scratchpadcolor: Rgba,
    pub globalcolor: Rgba,
    pub overlaycolor: Rgba,
    pub shadowscolor: Rgba,

    // blur / shadows / visual effects
    pub blur: bool,
    pub blur_layer: bool,
    pub blur_optimized: bool,
    pub border_radius: i32,
    pub blur_params: BlurParams,
    pub shadows: bool,
    pub shadow_only_floating: bool,
    pub layer_shadows: bool,
    pub shadows_size: u32,
    pub shadows_blur: f32,
    pub shadows_position_x: i32,
    pub shadows_position_y: i32,

    // input
    pub repeat_rate: i32,
    pub repeat_delay: i32,
    pub numlock_on: bool,
    pub capslock: bool,
    pub disable_trackpad: bool,
    pub tap_to_click: bool,
    pub tap_and_drag: bool,
    pub drag_lock: bool,
    pub mouse_natural_scrolling: bool,
    pub trackpad_natural_scrolling: bool,
    pub disable_while_typing: bool,
    pub left_handed: bool,
    pub middle_button_emulation: bool,
    pub accel_profile: AccelProfile,
    pub accel_speed: f64,
    pub scroll_method: ScrollMethod,
    pub scroll_button: u32,
    pub click_method: ClickMethod,
    pub send_events_mode: u32,
    pub button_map: u32,
    pub axis_scroll_factor: f64,
    pub axis_bind_apply_timeout: u32,
    pub tablet_map_to_mon: Option<String>,

    // touch
    pub touch_distance_threshold: f64,
    pub touch_degrees_leniency: f64,
    pub touch_timeout_ms: u32,
    pub touch_edge_size_left: f64,
    pub touch_edge_size_top: f64,
    pub touch_edge_size_right: f64,
    pub touch_edge_size_bottom: f64,

    // misc
    pub sloppyfocus: bool,
    /// When the cursor crosses into a new toplevel under sloppy focus,
    /// also re-run the layout so scroller mode re-centers on it. Off
    /// by default because in scroller layout this means every mouse
    /// move that flips focus between two columns kicks off a fresh
    /// 480 ms slide animation, and the user perceives the constant
    /// re-centering as window jitter. Enable explicitly if you really
    /// do want sloppy focus to drive the scroller.
    pub sloppyfocus_arrange: bool,
    pub warpcursor: bool,
    pub drag_corner: i32,
    pub drag_warp_cursor: bool,
    pub cursor_hide_timeout: u32,
    pub cursor_theme: Option<String>,
    pub cursor_size: u32,
    pub focus_on_activate: bool,
    pub idleinhibit_ignore_visible: bool,
    pub log_level: i32,
    pub xwayland_persistence: bool,
    pub syncobj_enable: bool,
    pub drag_tile_refresh_interval: f32,
    pub drag_floating_refresh_interval: f32,
    pub allow_tearing: TearingMode,
    pub allow_shortcuts_inhibit: ShortcutsInhibit,
    pub allow_lock_transparent: bool,
    pub key_mode: String,

    // xkb
    pub xkb_rules: XkbRules,

    // rules / bindings
    pub window_rules: Vec<WindowRule>,
    pub monitor_rules: Vec<MonitorRule>,
    pub tag_rules: Vec<TagRule>,
    pub layer_rules: Vec<LayerRule>,
    pub key_bindings: Vec<KeyBinding>,
    pub mouse_bindings: Vec<MouseBinding>,
    pub axis_bindings: Vec<AxisBinding>,
    pub switch_bindings: Vec<SwitchBinding>,
    pub gesture_bindings: Vec<GestureBinding>,
    pub touch_gesture_bindings: Vec<TouchGestureBinding>,

    // startup
    pub envs: Vec<(String, String)>,
    pub exec: Vec<String>,
    pub exec_once: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            animations: true,
            layer_animations: false,
            animation_type_open: "slide_in".into(),
            animation_type_close: "slide_out".into(),
            layer_animation_type_open: "slide_in".into(),
            layer_animation_type_close: "slide_out".into(),
            animation_fade_in: true,
            animation_fade_out: true,
            tag_animation_direction: TagAnimDirection::Horizontal,
            zoom_initial_ratio: 0.4,
            zoom_end_ratio: 0.8,
            fadein_begin_opacity: 0.5,
            fadeout_begin_opacity: 0.5,
            animation_duration_move: 500,
            animation_duration_open: 400,
            animation_duration_tag: 300,
            animation_duration_close: 300,
            animation_duration_focus: 0,
            animation_curve_move: BezierCurve::default(),
            animation_curve_open: BezierCurve::default(),
            animation_curve_tag: BezierCurve::default(),
            animation_curve_close: BezierCurve::default(),
            animation_curve_focus: BezierCurve::default(),
            animation_curve_opafadein: BezierCurve::default(),
            animation_curve_opafadeout: BezierCurve([0.5, 0.5, 0.5, 0.5]),
            animation_duration_canvas_pan: 300,
            animation_curve_canvas_pan: BezierCurve::default(),
            animation_duration_canvas_zoom: 300,
            animation_curve_canvas_zoom: BezierCurve::default(),
            animation_clock_move: "bezier".into(),
            animation_clock_open: "bezier".into(),
            animation_clock_close: "bezier".into(),
            animation_clock_tag: "bezier".into(),
            animation_clock_focus: "bezier".into(),
            animation_clock_layer: "bezier".into(),
            animation_spring_stiffness: 800.0,
            animation_spring_damping_ratio: 1.0,
            animation_spring_mass: 1.0,

            scroller_structs: 20,
            scroller_default_proportion: 0.9,
            scroller_default_proportion_single: 1.0,
            scroller_ignore_proportion_single: true,
            scroller_focus_center: false,
            scroller_prefer_center: false,
            scroller_prefer_overspread: true,
            edge_scroller_pointer_focus: true,
            scroller_proportion_presets: vec![],
            circle_layouts: vec![],

            new_is_master: true,
            default_layout: "tile".into(),
            default_mfact: 0.55,
            default_nmaster: 1,
            center_master_overspread: false,
            center_when_single_stack: true,
            auto_layout: false,
            focus_cross_monitor: false,
            exchange_cross_monitor: false,
            scratchpad_cross_monitor: false,
            focus_cross_tag: false,
            view_current_to_back: false,
            no_border_when_single: false,
            no_radius_when_single: false,
            snap_distance: 30,
            enable_floating_snap: false,
            drag_tile_to_tile: false,
            swipe_min_threshold: 1,
            focused_opacity: 1.0,
            unfocused_opacity: 1.0,

            hotarea_size: 10,
            hotarea_corner: HotareaCorner::BottomLeft,
            enable_hotarea: true,

            ov_tab_mode: 0,
            overview_gap_inner: 5,
            overview_gap_outer: 30,

            enable_gaps: true,
            smartgaps: false,
            gappih: 5,
            gappiv: 5,
            gappoh: 10,
            gappov: 10,
            borderpx: 4,

            canvas_tiling: false,
            canvas_tiling_gap: 10,
            canvas_pan_on_kill: true,
            canvas_anchor_animate: false,

            tag_carousel: false,

            scratchpad_width_ratio: 0.8,
            scratchpad_height_ratio: 0.9,
            single_scratchpad: true,

            rootcolor: Rgba([0x32_u8 as f32 / 255.0, 0x32_u8 as f32 / 255.0, 0x32_u8 as f32 / 255.0, 1.0]),
            bordercolor: Rgba([0x44_u8 as f32 / 255.0, 0x44_u8 as f32 / 255.0, 0x44_u8 as f32 / 255.0, 1.0]),
            focuscolor: Rgba([0xc6_u8 as f32 / 255.0, 0x6b_u8 as f32 / 255.0, 0x25_u8 as f32 / 255.0, 1.0]),
            maximizescreencolor: Rgba([0x89_u8 as f32 / 255.0, 0xaa_u8 as f32 / 255.0, 0x61_u8 as f32 / 255.0, 1.0]),
            urgentcolor: Rgba([0xad_u8 as f32 / 255.0, 0x40_u8 as f32 / 255.0, 0x1f_u8 as f32 / 255.0, 1.0]),
            scratchpadcolor: Rgba([0x51_u8 as f32 / 255.0, 0x6c_u8 as f32 / 255.0, 0x93_u8 as f32 / 255.0, 1.0]),
            globalcolor: Rgba([0xb1_u8 as f32 / 255.0, 0x53_u8 as f32 / 255.0, 0xa7_u8 as f32 / 255.0, 1.0]),
            overlaycolor: Rgba([0x14_u8 as f32 / 255.0, 0xa5_u8 as f32 / 255.0, 0x7c_u8 as f32 / 255.0, 1.0]),
            shadowscolor: Rgba([0.0, 0.0, 0.0, 1.0]),

            blur: false,
            blur_layer: false,
            blur_optimized: true,
            border_radius: 0,
            blur_params: BlurParams::default(),
            shadows: false,
            shadow_only_floating: true,
            layer_shadows: false,
            shadows_size: 10,
            shadows_blur: 15.0,
            shadows_position_x: 0,
            shadows_position_y: 0,

            repeat_rate: 25,
            repeat_delay: 600,
            numlock_on: false,
            capslock: false,
            disable_trackpad: false,
            tap_to_click: true,
            tap_and_drag: true,
            drag_lock: true,
            mouse_natural_scrolling: false,
            trackpad_natural_scrolling: false,
            disable_while_typing: true,
            left_handed: false,
            middle_button_emulation: false,
            accel_profile: AccelProfile::Adaptive,
            accel_speed: 0.0,
            scroll_method: ScrollMethod::TwoFinger,
            scroll_button: 274,
            click_method: ClickMethod::ButtonAreas,
            send_events_mode: 0,
            button_map: 0,
            axis_scroll_factor: 1.0,
            axis_bind_apply_timeout: 100,
            tablet_map_to_mon: None,

            touch_distance_threshold: 50.0,
            touch_degrees_leniency: 15.0,
            touch_timeout_ms: 800,
            touch_edge_size_left: 50.0,
            touch_edge_size_top: 50.0,
            touch_edge_size_right: 50.0,
            touch_edge_size_bottom: 50.0,

            sloppyfocus: true,
            sloppyfocus_arrange: false,
            warpcursor: true,
            drag_corner: 3,
            drag_warp_cursor: true,
            cursor_hide_timeout: 0,
            cursor_theme: None,
            cursor_size: 24,
            focus_on_activate: true,
            idleinhibit_ignore_visible: false,
            log_level: 0,
            xwayland_persistence: true,
            syncobj_enable: false,
            drag_tile_refresh_interval: 8.0,
            drag_floating_refresh_interval: 8.0,
            allow_tearing: TearingMode::Disabled,
            allow_shortcuts_inhibit: ShortcutsInhibit::Enable,
            allow_lock_transparent: false,
            key_mode: "default".into(),

            xkb_rules: XkbRules::default(),

            window_rules: vec![],
            monitor_rules: vec![],
            tag_rules: vec![],
            layer_rules: vec![],
            key_bindings: vec![],
            mouse_bindings: vec![],
            axis_bindings: vec![],
            switch_bindings: vec![],
            gesture_bindings: vec![],
            touch_gesture_bindings: vec![],

            envs: vec![],
            exec: vec![],
            exec_once: vec![],
        }
    }
}
