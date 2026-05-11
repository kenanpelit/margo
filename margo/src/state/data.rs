//! Pure compositor data types — `MargoClient`, `MargoMonitor`, and
//! the small enums/snapshots they reference. Extracted from `state.rs`
//! (roadmap Q1) so adding a per-client flag or a per-monitor field
//! doesn't recompile the rest of the 6000-LOC translation unit.
//!
//! Nothing here references `MargoState`. The only outward coupling is
//! to existing crate-level data (`crate::layout::Rect`,
//! `crate::animation::*`, `crate::render::open_close::OpenCloseKind`,
//! etc.) plus a few smithay types via re-exports — all stable.

use std::collections::VecDeque;

use margo_config::Config;
use smithay::{
    desktop::Window,
    output::Output,
    wayland::{compositor::with_states, shell::xdg::{ToplevelSurface, XdgToplevelSurfaceData}},
};

use crate::{
    animation::{ClientAnimation, OpacityAnimation},
    layout::{LayoutId, Pertag, Rect},
    protocols::{
        dwl_ipc::DwlIpcState, ext_workspace::ExtWorkspaceState,
        foreign_toplevel::ForeignToplevelHandle,
    },
    MAX_TAGS,
};

// ── Hot corner enum ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl HotCorner {
    /// Pull the matching dispatch-action string out of `Config`. Empty
    /// string = "this corner is disabled".
    pub fn action_str(self, cfg: &Config) -> &str {
        match self {
            HotCorner::TopLeft => &cfg.hot_corner_top_left,
            HotCorner::TopRight => &cfg.hot_corner_top_right,
            HotCorner::BottomLeft => &cfg.hot_corner_bottom_left,
            HotCorner::BottomRight => &cfg.hot_corner_bottom_right,
        }
    }
}

// ── Fullscreen mode ──────────────────────────────────────────────────────────

/// How "fullscreen" a client is.
///
/// Two distinct modes — both reachable from a key bind today:
///
/// ```text
/// bind = super,f,togglefullscreen            # WorkArea
/// bind = super+shift,f,togglefullscreen_exclusive  # Exclusive
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FullscreenMode {
    #[default]
    Off,
    WorkArea,
    Exclusive,
}

// ── Window-rule reapply trigger ──────────────────────────────────────────────

/// Tags the three sites that drive a post-mount window-rule reapply.
/// Lets `MargoState::reapply_rules` log the trigger reason and gives
/// future per-trigger policy (e.g. "don't move clients on `Reload`,
/// only on `InitialMap`") a single place to live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowRuleReason {
    InitialMap,
    AppIdSettled,
    Reload,
}

// ── Resize snapshot ──────────────────────────────────────────────────────────

/// Captured snapshot of a window's rendered content, used to keep the
/// pre-resize visuals on screen while the client (typically Electron:
/// Helium, Spotify, Discord) takes 50–100 ms to ack a configure and
/// commit a buffer at the new size.
pub struct ResizeSnapshot {
    pub texture: smithay::backend::renderer::gles::GlesTexture,
    pub source_size: smithay::utils::Size<i32, smithay::utils::Logical>,
    pub captured_at: std::time::Instant,
}

impl std::fmt::Debug for ResizeSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResizeSnapshot")
            .field("source_size", &self.source_size)
            .field("captured_at", &self.captured_at)
            .finish_non_exhaustive()
    }
}

// ── Per-window state ─────────────────────────────────────────────────────────

pub struct MargoClient {
    pub surface_type: crate::SurfaceType,
    pub geom: Rect,
    pub pending: Rect,
    pub float_geom: Rect,
    pub canvas_geom: [Rect; MAX_TAGS],
    pub tags: u32,
    pub old_tags: u32,
    pub is_floating: bool,
    /// Whether the client is in any fullscreen mode. Kept as a bool
    /// for backward-compat; the *kind* lives in `fullscreen_mode` and
    /// setters keep the two in lock-step.
    pub is_fullscreen: bool,
    pub fullscreen_mode: FullscreenMode,
    pub is_fake_fullscreen: bool,
    pub is_maximized_screen: bool,
    pub is_minimized: bool,
    pub is_urgent: bool,
    pub is_global: bool,
    pub is_unglobal: bool,
    pub is_overlay: bool,
    pub is_in_scratchpad: bool,
    pub is_scratchpad_show: bool,
    pub is_named_scratchpad: bool,
    pub is_term: bool,
    pub no_swallow: bool,
    pub is_killing: bool,
    pub is_tag_switching: bool,
    /// True while the pointer is hovering this client's grid slot
    /// during overview. Border layer paints with `focuscolor` instead
    /// of `bordercolor` while set.
    pub is_overview_hovered: bool,
    pub no_border: bool,
    pub no_shadow: bool,
    pub no_radius: bool,
    pub no_animation: bool,
    pub open_silent: bool,
    pub tag_silent: bool,
    pub allow_csd: bool,
    pub no_focus: bool,
    pub no_fade_in: bool,
    pub no_fade_out: bool,
    pub no_blur: bool,
    pub canvas_no_tile: bool,
    /// Set by a window rule. When true, screen-capture clients see
    /// solid black for this window's region.
    pub block_out_from_screencast: bool,
    pub min_width: i32,
    pub min_height: i32,
    pub max_width: i32,
    pub max_height: i32,
    pub canvas_floating: bool,
    pub force_fake_maximize: bool,
    pub force_tiled_state: bool,
    pub is_master: bool,
    pub border_width: u32,
    pub scroller_proportion: f32,
    pub scroller_proportion_single: f32,
    pub master_mfact_per: f64,
    pub master_inner_per: f64,
    pub stack_inner_per: f64,
    pub focused_opacity: f32,
    pub unfocused_opacity: f32,
    pub pid: u32,
    pub animation: ClientAnimation,
    pub opacity_animation: OpacityAnimation,
    /// Niri-style resize animation snapshot. Set when the layout slot
    /// size changes; the next render captures the current surface tree
    /// to a `GlesTexture` and stores it here.
    pub resize_snapshot: Option<ResizeSnapshot>,
    pub snapshot_pending: bool,
    /// True while the client is between `new_toplevel` and its first
    /// post-app_id commit. Prevents Qt-style "appears then snaps"
    /// flicker for rules keyed on app_id.
    pub is_initial_map_pending: bool,
    pub opening_animation: Option<crate::animation::OpenCloseClientAnim>,
    pub opening_texture: Option<smithay::backend::renderer::gles::GlesTexture>,
    pub opening_capture_pending: bool,
    pub animation_type_open: Option<String>,
    pub animation_type_close: Option<String>,
    pub app_id: String,
    pub title: String,
    pub monitor: usize,
    pub swallowing: Option<usize>,
    pub swallowed_by: Option<usize>,
    pub canvas_tag_geom: Vec<Rect>,
    pub window: Window,
    pub foreign_toplevel_handle: Option<ForeignToplevelHandle>,
    pub border: crate::border::ClientBorder,
    /// True if any of this client's surfaces was scanned out directly
    /// (zero-copy primary/overlay plane) on the most recent frame.
    pub last_scanout: bool,
}

impl MargoClient {
    pub fn new(window: Window, monitor: usize, tags: u32, config: &Config) -> Self {
        Self {
            surface_type: crate::SurfaceType::XdgShell,
            geom: Rect::default(),
            pending: Rect::default(),
            float_geom: Rect::default(),
            canvas_geom: [Rect::default(); MAX_TAGS],
            tags,
            old_tags: 0,
            is_floating: false,
            is_fullscreen: false,
            fullscreen_mode: FullscreenMode::Off,
            is_fake_fullscreen: false,
            is_maximized_screen: false,
            is_minimized: false,
            is_urgent: false,
            is_global: false,
            is_unglobal: false,
            is_overlay: false,
            is_in_scratchpad: false,
            is_scratchpad_show: false,
            is_named_scratchpad: false,
            is_term: false,
            no_swallow: false,
            is_killing: false,
            is_tag_switching: false,
            is_overview_hovered: false,
            no_border: false,
            no_shadow: false,
            no_radius: false,
            no_animation: false,
            open_silent: false,
            tag_silent: false,
            allow_csd: false,
            no_focus: false,
            no_fade_in: false,
            no_fade_out: false,
            no_blur: false,
            canvas_no_tile: false,
            block_out_from_screencast: false,
            min_width: 0,
            min_height: 0,
            max_width: 0,
            max_height: 0,
            canvas_floating: false,
            force_fake_maximize: false,
            force_tiled_state: false,
            is_master: false,
            border_width: config.borderpx,
            scroller_proportion: config.scroller_default_proportion,
            scroller_proportion_single: config.scroller_default_proportion_single,
            master_mfact_per: 0.0,
            master_inner_per: 0.0,
            stack_inner_per: 0.0,
            focused_opacity: config.focused_opacity,
            unfocused_opacity: config.unfocused_opacity,
            pid: 0,
            animation: ClientAnimation::default(),
            opacity_animation: OpacityAnimation::default(),
            resize_snapshot: None,
            snapshot_pending: false,
            is_initial_map_pending: false,
            opening_animation: None,
            opening_texture: None,
            opening_capture_pending: false,
            animation_type_open: None,
            animation_type_close: None,
            app_id: String::new(),
            title: String::new(),
            monitor,
            swallowing: None,
            swallowed_by: None,
            canvas_tag_geom: Vec::new(),
            window,
            foreign_toplevel_handle: None,
            border: crate::border::ClientBorder::default(),
            last_scanout: false,
        }
    }

    pub fn is_tiled(&self) -> bool {
        !self.is_floating
            && !self.is_minimized
            && !self.is_killing
            && !self.is_maximized_screen
            && !self.is_fullscreen
            && !self.is_unglobal
    }

    pub fn is_visible_on(&self, mon: usize, tagset: u32) -> bool {
        // Hidden scratchpads (in_scratchpad without `show`) are
        // unmapped from the scene but remain in `clients` so the next
        // toggle picks the same instance up. is_visible_on is the
        // single chokepoint every layout/focus/IPC path goes through,
        // so guarding it here keeps the rest of the codebase from
        // each having to learn about the scratchpad show flag.
        if self.is_in_scratchpad && !self.is_scratchpad_show {
            return false;
        }
        self.monitor == mon && (self.tags & tagset) != 0
    }
}

// ── Rule-matching helpers ───────────────────────────────────────────────────

/// Whether a `LayerRule` applies to a given layer-shell namespace.
/// Empty `layer_name` patterns match every namespace.
pub(crate) fn matches_layer_name(rule: &margo_config::LayerRule, namespace: &str) -> bool {
    rule.layer_name
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| matches_rule_text(p, namespace))
        .unwrap_or(true)
}

pub(crate) fn matches_rule_text(pattern: &str, value: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    if value.is_empty() {
        return false;
    }
    match regex::Regex::new(pattern) {
        Ok(regex) => regex.is_match(value),
        Err(_) => {
            let trimmed = pattern.trim_start_matches('^').trim_end_matches('$');
            value == trimmed || value.contains(trimmed)
        }
    }
}

pub(crate) fn read_toplevel_identity(surface: &ToplevelSurface) -> (String, String) {
    with_states(surface.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok())
            .map(|data| {
                (
                    data.app_id.clone().unwrap_or_default(),
                    data.title.clone().unwrap_or_default(),
                )
            })
            .unwrap_or_default()
    })
}

/// Clamp `(w, h)` in place against `min_*`/`max_*` constraints. Each
/// constraint is ignored if its value is `0`.
pub(crate) fn clamp_size(w: &mut i32, h: &mut i32, min_w: i32, min_h: i32, max_w: i32, max_h: i32) {
    if min_w > 0 && *w < min_w {
        *w = min_w;
    }
    if min_h > 0 && *h < min_h {
        *h = min_h;
    }
    if max_w > 0 && *w > max_w {
        *w = max_w;
    }
    if max_h > 0 && *h > max_h {
        *h = max_h;
    }
}

// ── Per-monitor state ────────────────────────────────────────────────────────

pub struct MargoMonitor {
    pub name: String,
    pub output: Output,
    pub monitor_area: Rect,
    pub work_area: Rect,
    pub seltags: usize,
    pub tagset: [u32; 2],
    pub gappih: i32,
    pub gappiv: i32,
    pub gappoh: i32,
    pub gappov: i32,
    pub pertag: Pertag,
    pub selected: Option<usize>,
    pub prev_selected: Option<usize>,
    pub is_overview: bool,
    pub overview_backup_tagset: u32,
    pub canvas_overview_visible: bool,
    pub canvas_in_overview: bool,
    pub canvas_saved_pan_x: f32,
    pub canvas_saved_pan_y: f32,
    pub canvas_saved_zoom: f32,
    pub minimap_visible: bool,
    pub dwl_ipc: DwlIpcState,
    pub ext_workspace: ExtWorkspaceState,
    pub scale: f32,
    pub transform: i32,
    pub enabled: bool,
    /// Last N focused-client indices for this monitor (MRU order,
    /// most recent first). Capped at `FOCUS_HISTORY_DEPTH`. Exposed
    /// in state.json as `focus_history`.
    pub focus_history: VecDeque<usize>,
    /// Number of u16 entries per channel in DRM `GAMMA_LUT_SIZE`.
    /// 0 → gamma control unsupported (winit backend or connector
    /// without GAMMA_LUT). Set by the udev backend on output add.
    pub gamma_size: u32,
}

impl MargoMonitor {
    pub fn current_tagset(&self) -> u32 { self.tagset[self.seltags] }
    pub fn current_layout(&self) -> LayoutId { self.pertag.ltidxs[self.pertag.curtag] }
    pub fn current_mfact(&self) -> f32 { self.pertag.mfacts[self.pertag.curtag] }
    pub fn current_nmaster(&self) -> u32 { self.pertag.nmasters[self.pertag.curtag] }
}

// ── In-flight close transitions ──────────────────────────────────────────────

/// A window in the middle of its close animation. Lives in
/// `MargoState::closing_clients` from `toplevel_destroyed` (or X11
/// `destroyed_window`) until `tick_animations` decides the curve is
/// done. The captured `texture` is what the render path draws — we
/// can't render the live `wl_surface` because it's already gone.
#[derive(Debug)]
pub struct ClosingClient {
    pub id: smithay::backend::renderer::element::Id,
    pub texture: Option<smithay::backend::renderer::gles::GlesTexture>,
    pub capture_pending: bool,
    pub geom: Rect,
    pub monitor: usize,
    pub tags: u32,
    pub time_started: u32,
    pub duration: u32,
    pub progress: f32,
    pub kind: crate::render::open_close::OpenCloseKind,
    pub extreme_scale: f32,
    pub border_radius: f32,
    pub source_surface: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
}

/// Layer surface in mid-open or mid-close transition. Mirrors
/// `ClosingClient` but stripped down — layer surfaces have no
/// per-tag visibility or monitor migration.
#[derive(Debug)]
pub struct LayerSurfaceAnim {
    pub time_started: u32,
    pub duration: u32,
    pub progress: f32,
    pub is_close: bool,
    pub texture: Option<smithay::backend::renderer::gles::GlesTexture>,
    pub capture_pending: bool,
    pub geom: Rect,
    pub kind: crate::render::open_close::OpenCloseKind,
    pub source_surface: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
}
