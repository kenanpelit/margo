#![allow(dead_code)]
use std::{cell::RefCell, os::unix::io::OwnedFd, path::PathBuf, rc::Rc, sync::Arc};

use anyhow::{Context, Result};
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        renderer::utils::on_commit_buffer_handler,
    },
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_dmabuf,
    delegate_layer_shell, delegate_output, delegate_primary_selection, delegate_seat, delegate_shm,
    delegate_xdg_decoration, delegate_xdg_shell, delegate_session_lock,
    delegate_idle_notify, delegate_idle_inhibit,
    desktop::{LayerSurface as DesktopLayerSurface, PopupManager, Space, Window, WindowSurface, WindowSurfaceType, layer_map_for_output},
    input::{
        Seat, SeatHandler, SeatState,
        dnd::{DndFocus, DndGrabHandler, Source},
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{
            AxisFrame, ButtonEvent, CursorImageStatus, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
            MotionEvent, PointerTarget, RelativeMotionEvent,
        },
        touch::TouchTarget,
    },
    output::Output,
    reexports::{
        calloop::{LoopHandle, LoopSignal},
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as XdgDecorationMode,
        wayland_server::{
            DisplayHandle, Resource,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Client, Display,
        },
    },
    utils::{Clock, Logical, Monotonic, Point, Rectangle, Serial, Size, SERIAL_COUNTER},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler,
            CompositorState,
        },
        output::{OutputHandler, OutputManagerState},
        seat::WaylandFocus,
        selection::{
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_focus,
                set_data_device_selection, DataDeviceHandler, DataDeviceState,
                WaylandDndGrabHandler, WlOfferData,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata,
                request_primary_client_selection, set_primary_focus, set_primary_selection,
                PrimarySelectionHandler, PrimarySelectionState,
            },
            wlr_data_control::{DataControlHandler, DataControlState},
            SelectionHandler, SelectionSource, SelectionTarget,
        },
        shell::{
            wlr_layer::{
                Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
            },
            xdg::{
                decoration::{XdgDecorationHandler, XdgDecorationState},
                PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
                XdgToplevelSurfaceData,
            },
        },
        shm::{ShmHandler, ShmState},
        session_lock::{SessionLocker, SessionLockHandler, SessionLockManagerState, LockSurface},
        viewporter::ViewporterState,
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{X11Surface, X11Wm, XWaylandClientData, XwmHandler, xwm::{Reorder, ResizeEdge, X11Window, XwmId}},
};

use margo_config::{parse_config, Config, WindowRule};

use crate::{
    animation::{AnimationCurves, AnimationType, ClientAnimation, OpacityAnimation},
    cursor::CursorManager,
    input::{GestureState, KeyboardState, PointerState, TouchState},
    layout::{self, LayoutId, Pertag, Rect},
    protocols::{
        dwl_ipc::DwlIpcState,
        ext_workspace::ExtWorkspaceState,
        foreign_toplevel::{ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState},
        layer_shell::LayerSurface,
    },
    MAX_TAGS,
};

// ── Client data attached to each Wayland client connection ───────────────────

#[derive(Default)]
pub struct MargoClientData {
    pub compositor_state: CompositorClientState,
}

impl ClientData for MargoClientData {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

// ── Focus target ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum FocusTarget {
    Window(Window),
    LayerSurface(WlrLayerSurface),
    SessionLock(LockSurface),
}

impl smithay::utils::IsAlive for FocusTarget {
    fn alive(&self) -> bool {
        match self {
            FocusTarget::Window(w) => w.alive(),
            FocusTarget::LayerSurface(l) => l.alive(),
            FocusTarget::SessionLock(s) => s.alive(),
        }
    }
}

impl WaylandFocus for FocusTarget {
    fn wl_surface(&self) -> Option<std::borrow::Cow<'_, WlSurface>> {
        match self {
            FocusTarget::Window(w) => w.wl_surface(),
            FocusTarget::LayerSurface(l) => Some(std::borrow::Cow::Borrowed(l.wl_surface())),
            FocusTarget::SessionLock(s) => Some(std::borrow::Cow::Borrowed(s.wl_surface())),
        }
    }
}

impl FocusTarget {
    fn inner_wl_surface(&self) -> Option<&WlSurface> {
        match self {
            Self::Window(w) => match w.underlying_surface() {
                WindowSurface::Wayland(s) => Some(s.wl_surface()),
                WindowSurface::X11(_) => None, // X11 focus via WaylandFocus::wl_surface
            },
            Self::LayerSurface(l) => Some(l.wl_surface()),
            Self::SessionLock(s) => Some(s.wl_surface()),
        }
    }
}

impl KeyboardTarget<MargoState> for FocusTarget {
    fn enter(
        &self,
        seat: &Seat<MargoState>,
        data: &mut MargoState,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        tracing::info!("FocusTarget::enter called for {:?}", self);
        if let Some(s) = self.inner_wl_surface() {
            tracing::info!("FocusTarget::enter forwarding to WlSurface");
            KeyboardTarget::enter(s, seat, data, keys, serial);
        }
    }
    fn leave(&self, seat: &Seat<MargoState>, data: &mut MargoState, serial: Serial) {
        tracing::info!("FocusTarget::leave called for {:?}", self);
        if let Some(s) = self.inner_wl_surface() {
            KeyboardTarget::leave(s, seat, data, serial);
        }
    }
    fn key(
        &self,
        seat: &Seat<MargoState>,
        data: &mut MargoState,
        key: KeysymHandle<'_>,
        state: smithay::backend::input::KeyState,
        serial: Serial,
        time: u32,
    ) {
        if let Some(s) = self.inner_wl_surface() {
            KeyboardTarget::key(s, seat, data, key, state, serial, time);
        }
    }
    fn modifiers(
        &self,
        seat: &Seat<MargoState>,
        data: &mut MargoState,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        if let Some(s) = self.inner_wl_surface() {
            KeyboardTarget::modifiers(s, seat, data, modifiers, serial);
        }
    }
}

impl PointerTarget<MargoState> for FocusTarget {
    fn enter(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &MotionEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::enter(s, seat, data, event); }
    }
    fn motion(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &MotionEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::motion(s, seat, data, event); }
    }
    fn relative_motion(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &RelativeMotionEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::relative_motion(s, seat, data, event); }
    }
    fn button(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &ButtonEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::button(s, seat, data, event); }
    }
    fn axis(&self, seat: &Seat<MargoState>, data: &mut MargoState, frame: AxisFrame) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::axis(s, seat, data, frame); }
    }
    fn frame(&self, seat: &Seat<MargoState>, data: &mut MargoState) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::frame(s, seat, data); }
    }
    fn leave(&self, seat: &Seat<MargoState>, data: &mut MargoState, serial: Serial, time: u32) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::leave(s, seat, data, serial, time); }
    }
    fn gesture_swipe_begin(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &GestureSwipeBeginEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::gesture_swipe_begin(s, seat, data, event); }
    }
    fn gesture_swipe_update(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &GestureSwipeUpdateEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::gesture_swipe_update(s, seat, data, event); }
    }
    fn gesture_swipe_end(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &GestureSwipeEndEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::gesture_swipe_end(s, seat, data, event); }
    }
    fn gesture_pinch_begin(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &GesturePinchBeginEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::gesture_pinch_begin(s, seat, data, event); }
    }
    fn gesture_pinch_update(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &GesturePinchUpdateEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::gesture_pinch_update(s, seat, data, event); }
    }
    fn gesture_pinch_end(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &GesturePinchEndEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::gesture_pinch_end(s, seat, data, event); }
    }
    fn gesture_hold_begin(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &GestureHoldBeginEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::gesture_hold_begin(s, seat, data, event); }
    }
    fn gesture_hold_end(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &GestureHoldEndEvent) {
        if let Some(s) = self.inner_wl_surface() { PointerTarget::gesture_hold_end(s, seat, data, event); }
    }
}

impl TouchTarget<MargoState> for FocusTarget {
    fn down(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &smithay::input::touch::DownEvent, seq: Serial) {
        if let Some(s) = self.inner_wl_surface() { TouchTarget::down(s, seat, data, event, seq); }
    }
    fn up(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &smithay::input::touch::UpEvent, seq: Serial) {
        if let Some(s) = self.inner_wl_surface() { TouchTarget::up(s, seat, data, event, seq); }
    }
    fn motion(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &smithay::input::touch::MotionEvent, seq: Serial) {
        if let Some(s) = self.inner_wl_surface() { TouchTarget::motion(s, seat, data, event, seq); }
    }
    fn frame(&self, seat: &Seat<MargoState>, data: &mut MargoState, seq: Serial) {
        if let Some(s) = self.inner_wl_surface() { TouchTarget::frame(s, seat, data, seq); }
    }
    fn cancel(&self, seat: &Seat<MargoState>, data: &mut MargoState, seq: Serial) {
        if let Some(s) = self.inner_wl_surface() { TouchTarget::cancel(s, seat, data, seq); }
    }
    fn shape(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &smithay::input::touch::ShapeEvent, seq: Serial) {
        if let Some(s) = self.inner_wl_surface() { TouchTarget::shape(s, seat, data, event, seq); }
    }
    fn orientation(&self, seat: &Seat<MargoState>, data: &mut MargoState, event: &smithay::input::touch::OrientationEvent, seq: Serial) {
        if let Some(s) = self.inner_wl_surface() { TouchTarget::orientation(s, seat, data, event, seq); }
    }
}

impl DndFocus<MargoState> for FocusTarget {
    type OfferData<S: Source> = WlOfferData<S>;

    fn enter<S: Source>(
        &self,
        data: &mut MargoState,
        dh: &DisplayHandle,
        source: Arc<S>,
        seat: &Seat<MargoState>,
        location: Point<f64, Logical>,
        serial: &Serial,
    ) -> Option<WlOfferData<S>> {
        self.inner_wl_surface()
            .and_then(|s| DndFocus::enter(s, data, dh, source, seat, location, serial))
    }

    fn motion<S: Source>(
        &self,
        data: &mut MargoState,
        offer: Option<&mut WlOfferData<S>>,
        seat: &Seat<MargoState>,
        location: Point<f64, Logical>,
        time: u32,
    ) {
        if let Some(s) = self.inner_wl_surface() {
            DndFocus::motion(s, data, offer, seat, location, time);
        }
    }

    fn leave<S: Source>(
        &self,
        data: &mut MargoState,
        offer: Option<&mut WlOfferData<S>>,
        seat: &Seat<MargoState>,
    ) {
        if let Some(s) = self.inner_wl_surface() {
            DndFocus::leave(s, data, offer, seat);
        }
    }

    fn drop<S: Source>(
        &self,
        data: &mut MargoState,
        offer: Option<&mut WlOfferData<S>>,
        seat: &Seat<MargoState>,
    ) {
        if let Some(s) = self.inner_wl_surface() {
            DndFocus::drop(s, data, offer, seat);
        }
    }
}

// ── Margo: per-window compositor state ───────────────────────────────────────

pub struct MargoClient {
    pub surface_type: crate::SurfaceType,
    pub geom: Rect,
    pub pending: Rect,
    pub float_geom: Rect,
    pub canvas_geom: [Rect; MAX_TAGS],
    pub tags: u32,
    pub old_tags: u32,
    pub is_floating: bool,
    pub is_fullscreen: bool,
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
    /// Set by a window rule. When true, screen-capture/screencast clients
    /// (via wlr-screencopy etc.) see solid black for this window's region.
    pub block_out_from_screencast: bool,
    /// Optional size constraints (logical px) — applied during
    /// `arrange_monitor` and floating geometry resolution. 0 = unconstrained.
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
}

impl MargoClient {
    pub fn new(window: Window, monitor: usize, tags: u32, config: &Config) -> Self {
        Self {
            surface_type: crate::SurfaceType::XdgShell,
            geom: Rect::default(),
            pending: Rect::default(),
            float_geom: Rect::default(),
            canvas_geom: [Rect::default(); crate::MAX_TAGS],
            tags,
            old_tags: 0,
            is_floating: false,
            is_fullscreen: false,
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
        self.monitor == mon && (self.tags & tagset) != 0
    }
}

fn matches_rule_text(pattern: &str, value: &str) -> bool {
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

fn read_toplevel_identity(surface: &ToplevelSurface) -> (String, String) {
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
/// constraint is ignored if its value is `0`. Used by both window-rule
/// floating geometry and arrange_monitor's per-rule size limits.
fn clamp_size(w: &mut i32, h: &mut i32, min_w: i32, min_h: i32, max_w: i32, max_h: i32) {
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

// ── Per-monitor state ─────────────────────────────────────────────────────────

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
    /// Number of u16 entries per channel in the DRM `GAMMA_LUT_SIZE` for
    /// this output's CRTC. 0 means gamma control is not supported (e.g. on
    /// the winit backend or on a connector without GAMMA_LUT). Updated by
    /// the udev backend when the output is created.
    pub gamma_size: u32,
}

impl MargoMonitor {
    pub fn current_tagset(&self) -> u32 { self.tagset[self.seltags] }
    pub fn current_layout(&self) -> LayoutId { self.pertag.ltidxs[self.pertag.curtag] }
    pub fn current_mfact(&self) -> f32 { self.pertag.mfacts[self.pertag.curtag] }
    pub fn current_nmaster(&self) -> u32 { self.pertag.nmasters[self.pertag.curtag] }
}

// ── Animation tick ────────────────────────────────────────────────────────────

pub fn tick_animations(clients: &mut [MargoClient], curves: &AnimationCurves, now_ms: u32) -> bool {
    let mut changed = false;
    for c in clients.iter_mut() {
        let anim = &mut c.animation;
        if !anim.running { continue; }
        changed = true;
        let elapsed = now_ms.wrapping_sub(anim.time_started);
        if elapsed >= anim.duration {
            anim.running = false;
            c.geom = anim.current;
            continue;
        }
        let t = elapsed as f64 / anim.duration as f64;
        let s = curves.sample(t, anim.action);
        c.geom.x = lerp_i32(anim.initial.x, anim.current.x, s);
        c.geom.y = lerp_i32(anim.initial.y, anim.current.y, s);
        c.geom.width = lerp_i32(anim.initial.width, anim.current.width, s);
        c.geom.height = lerp_i32(anim.initial.height, anim.current.height, s);
    }
    changed
}

#[inline]
fn lerp_i32(a: i32, b: i32, t: f64) -> i32 {
    (a as f64 + (b - a) as f64 * t) as i32
}

// ── Top-level compositor state ────────────────────────────────────────────────

pub type DmabufImportHook = Rc<RefCell<dyn FnMut(&Dmabuf) -> bool>>;

pub struct MargoState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub viewporter_state: ViewporterState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub dmabuf_import_hook: Option<DmabufImportHook>,
    pub seat_state: SeatState<MargoState>,
    pub layer_shell_state: WlrLayerShellState,
    pub output_manager_state: OutputManagerState,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub session_lock_state: smithay::wayland::session_lock::SessionLockManagerState,
    /// `ext_idle_notifier_v1`: pings clients (swayidle, noctalia) once
    /// the seat has been idle for the duration they registered.
    pub idle_notifier_state: smithay::wayland::idle_notify::IdleNotifierState<MargoState>,
    /// `zwp_idle_inhibit_manager_v1`: clients (mpv, video players,
    /// presentation tools) can request "don't go idle while my surface
    /// is on screen". The notifier is paused while the set is non-empty.
    pub idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState,
    /// Surfaces that have an active idle-inhibit object. We feed
    /// `!is_empty()` to the notifier whenever this set changes.
    pub idle_inhibitors: std::collections::HashSet<
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    >,

    pub space: Space<Window>,
    pub popups: PopupManager,

    pub seat: Seat<MargoState>,

    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, MargoState>,
    pub loop_signal: LoopSignal,
    pub clock: Clock<Monotonic>,
    pub should_quit: bool,
    repaint_requested: bool,
    config_path: Option<PathBuf>,

    pub config: Config,
    pub animation_curves: AnimationCurves,
    pub clients: Vec<MargoClient>,
    pub monitors: Vec<MargoMonitor>,

    pub input_keyboard: KeyboardState,
    pub input_pointer: PointerState,
    pub input_touch: TouchState,
    pub input_gesture: GestureState,

    pub foreign_toplevel_list: ForeignToplevelListState,
    pub layer_surfaces: Vec<LayerSurface>,
    pub lock_surfaces: Vec<(Output, smithay::wayland::session_lock::LockSurface)>,

    pub session_locked: bool,
    pub enable_gaps: bool,
    pub cursor_status: CursorImageStatus,
    pub cursor_manager: CursorManager,
    pub xwm: Option<X11Wm>,
    pub xwayland_shell_state: XWaylandShellState,
    pub libinput: Option<smithay::reexports::input::Libinput>,
    pub gamma_control_manager_state: crate::protocols::gamma_control::GammaControlManagerState,
    /// Pending gamma ramp updates drained by the udev backend each frame.
    /// Tuple is (output, ramp). `None` ramp = restore default. The udev
    /// backend pops these and applies them via DRM `GAMMA_LUT`. Winit just
    /// drops them silently.
    pub pending_gamma: Vec<(Output, Option<Vec<u16>>)>,
    pub screencopy_state: crate::protocols::screencopy::ScreencopyManagerState,
    pub libinput_devices: Vec<smithay::reexports::input::Device>,
}

impl MargoState {
    pub fn new(
        config: Config,
        display: &mut Display<MargoState>,
        loop_handle: LoopHandle<'static, MargoState>,
        loop_signal: LoopSignal,
        config_path: Option<PathBuf>,
    ) -> Self {
        let dh = display.handle();
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let shm_formats = vec![
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Argb8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Xrgb8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Xbgr8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Abgr8888,
            smithay::reexports::wayland_server::protocol::wl_shm::Format::Rgb565,
        ];
        let shm_state = ShmState::new::<Self>(&dh, shm_formats);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let dmabuf_state = DmabufState::new();
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "seat0");
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let data_control_state =
            DataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let session_lock_state = smithay::wayland::session_lock::SessionLockManagerState::new::<Self, _>(&dh, |_| true);
        let idle_notifier_state =
            smithay::wayland::idle_notify::IdleNotifierState::<Self>::new(&dh, loop_handle.clone());
        let idle_inhibit_state =
            smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<Self>(&dh);
        let space = Space::default();
        let popups = PopupManager::default();
        let animation_curves = AnimationCurves::bake(&config);
        let input_keyboard = KeyboardState::new(&config);

        // Register dwl-ipc-v2 global
        dh.create_global::<Self, crate::protocols::generated::dwl_ipc::zdwl_ipc_manager_v2::ZdwlIpcManagerV2, _>(
            2,
            crate::protocols::dwl_ipc::DwlIpcGlobalData,
        );

        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);
        let foreign_toplevel_list = ForeignToplevelListState::new::<Self>(&dh);

        // wlr_gamma_control_v1 — sunsetr / gammastep / wlsunset use this to
        // push night-light ramps to outputs. Allow all clients (no privileged
        // filter) so user services can drive it freely.
        let gamma_control_manager_state =
            crate::protocols::gamma_control::GammaControlManagerState::new::<Self, _>(
                &dh,
                |_client| true,
            );

        // wlr-screencopy-unstable-v1: lets `grim`, `wf-recorder`, `screen rec`
        // etc. capture compositor outputs.
        let screencopy_state =
            crate::protocols::screencopy::ScreencopyManagerState::new::<Self, _>(
                &dh,
                |_client| true,
            );

        Self {
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            shm_state,
            viewporter_state,
            dmabuf_state,
            dmabuf_global: None,
            dmabuf_import_hook: None,
            seat_state,
            layer_shell_state,
            output_manager_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            session_lock_state,
            space,
            popups,
            seat,
            display_handle: dh,
            loop_handle,
            loop_signal,
            clock: Clock::new(),
            should_quit: false,
            repaint_requested: true,
            config_path,
            animation_curves,
            input_keyboard,
            input_pointer: Default::default(),
            input_touch: Default::default(),
            input_gesture: Default::default(),
            foreign_toplevel_list,
            layer_surfaces: vec![],
            lock_surfaces: vec![],
            clients: vec![],
            monitors: vec![],
            session_locked: false,
            idle_notifier_state,
            idle_inhibit_state,
            idle_inhibitors: std::collections::HashSet::new(),
            enable_gaps: config.enable_gaps,
            cursor_status: CursorImageStatus::default_named(),
            cursor_manager: CursorManager::new(),
            xwm: None,
            xwayland_shell_state,
            libinput: None,
            gamma_control_manager_state,
            pending_gamma: Vec::new(),
            screencopy_state,
            libinput_devices: Vec::new(),
            config,
        }
    }

    pub fn remove_output(&mut self, output: &Output) {
        for layer in smithay::desktop::layer_map_for_output(output).layers() {
            layer.layer_surface().send_close();
        }

        self.gamma_control_manager_state.output_removed(output);
        self.screencopy_state.remove_output(output);

        if let Some(pos) = self.monitors.iter().position(|m| m.output == *output) {
            tracing::info!("removing monitor: {}", self.monitors[pos].name);
            self.monitors.remove(pos);
        }
        self.space.unmap_output(output);
        self.lock_surfaces.retain(|(o, _)| o != output);
        self.pending_gamma.retain(|(o, _)| o != output);
        self.request_repaint();
    }

    pub fn arrange_all(&mut self) {
        for mon_idx in 0..self.monitors.len() {
            self.arrange_monitor(mon_idx);
        }
        self.request_repaint();
    }

    /// Start an interactive move grab on the currently focused window.
    /// Triggered by the `moveresize,curmove` action (typically a super+
    /// left-drag mousebind). No-op if there's no focused client or no
    /// pointer button is currently pressed.
    pub fn start_interactive_move(&mut self) {
        let Some(idx) = self.focused_client_idx() else { return };
        let window = self.clients[idx].window.clone();
        let initial_loc = smithay::utils::Point::<i32, smithay::utils::Logical>::from((
            self.clients[idx].geom.x,
            self.clients[idx].geom.y,
        ));
        let Some(pointer) = self.seat.get_pointer() else { return };
        // Use the most recent serial we've seen — we're driving the grab
        // ourselves from a synthesized command, so just take the next one.
        let serial = SERIAL_COUNTER.next_serial();
        let start_data = smithay::input::pointer::GrabStartData {
            focus: None,
            button: 0x110, // BTN_LEFT
            location: smithay::utils::Point::<f64, smithay::utils::Logical>::from((
                self.input_pointer.x,
                self.input_pointer.y,
            )),
        };
        let grab = crate::input::grabs::MoveSurfaceGrab {
            start_data,
            window,
            initial_loc,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    /// Start an interactive resize grab on the focused window. Edge
    /// defaults to bottom-right (the natural drag-corner gesture). If
    /// you want a specific edge, pass it in the action arg later.
    pub fn start_interactive_resize(&mut self) {
        let Some(idx) = self.focused_client_idx() else { return };
        let c = &self.clients[idx];
        let window = c.window.clone();
        let initial_loc = smithay::utils::Point::<i32, smithay::utils::Logical>::from((
            c.geom.x, c.geom.y,
        ));
        let initial_size = smithay::utils::Size::<i32, smithay::utils::Logical>::from((
            c.geom.width.max(1),
            c.geom.height.max(1),
        ));
        let Some(pointer) = self.seat.get_pointer() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        let start_data = smithay::input::pointer::GrabStartData {
            focus: None,
            button: 0x111, // BTN_RIGHT
            location: smithay::utils::Point::<f64, smithay::utils::Logical>::from((
                self.input_pointer.x,
                self.input_pointer.y,
            )),
        };
        let grab = crate::input::grabs::ResizeSurfaceGrab {
            start_data,
            window,
            edges:
                smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge::BottomRight,
            initial_loc,
            initial_size,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    /// Dump a one-shot diagnostic summary at INFO level — outputs, focused
    /// client, layer surfaces, lock state, idle inhibitors, recent counters.
    /// Triggered by `SIGUSR1` or by the `mctl debug-dump` IPC command so a
    /// user staring at a frozen / grey screen can capture state without
    /// crashing the compositor.
    pub fn debug_dump(&self) {
        tracing::info!("─── margo debug dump ───");
        tracing::info!(
            "outputs: {} monitor(s); session_locked={} lock_surfaces={}",
            self.monitors.len(),
            self.session_locked,
            self.lock_surfaces.len()
        );
        for (i, mon) in self.monitors.iter().enumerate() {
            tracing::info!(
                "  mon[{i}] {} area={}x{}+{}+{} tagset[{}]={:#x} prev={:#x} layout={:?} selected={:?} prev_selected={:?}",
                mon.name,
                mon.monitor_area.width,
                mon.monitor_area.height,
                mon.monitor_area.x,
                mon.monitor_area.y,
                mon.seltags,
                mon.tagset[mon.seltags],
                mon.tagset[mon.seltags ^ 1],
                mon.current_layout(),
                mon.selected,
                mon.prev_selected,
            );
        }
        tracing::info!(
            "clients: {} total; focused={:?}",
            self.clients.len(),
            self.focused_client_idx()
        );
        for (i, c) in self.clients.iter().enumerate().take(32) {
            tracing::info!(
                "  client[{i}] mon={} tags={:#x} float={} fs={} app_id={:?} title={:?} geom={}x{}+{}+{}",
                c.monitor,
                c.tags,
                c.is_floating,
                c.is_fullscreen,
                c.app_id,
                c.title,
                c.geom.width,
                c.geom.height,
                c.geom.x,
                c.geom.y,
            );
        }
        if self.clients.len() > 32 {
            tracing::info!("  … and {} more (truncated)", self.clients.len() - 32);
        }
        tracing::info!("idle inhibitors: {}", self.idle_inhibitors.len());
        let kbd = self.seat.get_keyboard();
        if let Some(kb) = kbd.as_ref() {
            tracing::info!(
                "keyboard focus: {}",
                kb.current_focus()
                    .map(|t| format!("{t:?}"))
                    .unwrap_or_else(|| "<none>".to_string())
            );
        }
        let layer_count: usize = self
            .space
            .outputs()
            .map(|o| smithay::desktop::layer_map_for_output(o).layers().count())
            .sum();
        tracing::info!("layer surfaces (all outputs): {layer_count}");
        tracing::info!("─── end debug dump ───");
    }

    pub fn request_repaint(&mut self) {
        self.repaint_requested = true;
    }

    pub fn take_repaint_request(&mut self) -> bool {
        let requested = self.repaint_requested;
        self.repaint_requested = false;
        requested
    }

    pub fn reload_config(&mut self) -> Result<()> {
        let new_config = parse_config(self.config_path.as_deref())
            .with_context(|| "reload margo config")?;

        if let Some(keyboard) = self.seat.get_keyboard() {
            let xkb_options = if new_config.xkb_rules.options.is_empty() {
                None
            } else {
                Some(new_config.xkb_rules.options.clone())
            };
            keyboard
                .set_xkb_config(
                    self,
                    smithay::input::keyboard::XkbConfig {
                        rules: &new_config.xkb_rules.rules,
                        model: &new_config.xkb_rules.model,
                        layout: &new_config.xkb_rules.layout,
                        variant: &new_config.xkb_rules.variant,
                        options: xkb_options,
                    },
                )
                .map_err(|e| anyhow::anyhow!("reload xkb config: {e:?}"))?;
            keyboard.change_repeat_info(new_config.repeat_rate, new_config.repeat_delay);
        }

        self.input_keyboard.repeat_rate = new_config.repeat_rate;
        self.input_keyboard.repeat_delay = new_config.repeat_delay;

        for device in &mut self.libinput_devices {
            crate::libinput_config::apply_to_device(device, &new_config);
        }

        for mon in &mut self.monitors {
            mon.gappih = new_config.gappih as i32;
            mon.gappiv = new_config.gappiv as i32;
            mon.gappoh = new_config.gappoh as i32;
            mon.gappov = new_config.gappov as i32;
        }

        self.animation_curves = AnimationCurves::bake(&new_config);
        self.enable_gaps = new_config.enable_gaps;
        self.config = new_config;
        for idx in 0..self.clients.len() {
            self.apply_window_rules_to_client(idx);
        }
        for mon_idx in 0..self.monitors.len() {
            self.apply_tag_rules_to_monitor(mon_idx);
        }
        self.arrange_all();
        crate::protocols::dwl_ipc::broadcast_all(self);
        self.request_repaint();
        tracing::info!("config reloaded");
        Ok(())
    }

    fn refresh_output_work_area(&mut self, output: &Output) {
        let work_area = {
            let map = layer_map_for_output(output);
            map.non_exclusive_zone()
        };

        if let Some(mon_idx) = self.monitors.iter().position(|m| m.output == *output) {
            let monitor_area = self.monitors[mon_idx].monitor_area;
            self.monitors[mon_idx].work_area = crate::layout::Rect {
                x: monitor_area.x + work_area.loc.x,
                y: monitor_area.y + work_area.loc.y,
                width: work_area.size.w,
                height: work_area.size.h,
            };
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn arrange_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }

        let mon = &self.monitors[mon_idx];
        let is_overview = mon.is_overview;
        let layout = if is_overview { crate::layout::LayoutId::Grid } else { mon.current_layout() };
        let tagset = if is_overview { !0 } else { mon.current_tagset() };
        let nmaster = mon.current_nmaster();
        let mfact = mon.current_mfact();
        let work_area = mon.work_area;
        let monitor_area = mon.monitor_area;
        let mut gaps = if is_overview {
            let inner = self.config.overview_gap_inner.max(0);
            let outer = self.config.overview_gap_outer.max(0);
            layout::GapConfig {
                gappih: inner,
                gappiv: inner,
                gappoh: outer,
                gappov: outer,
            }
        } else {
            layout::GapConfig {
                gappih: if self.enable_gaps { mon.gappih } else { 0 },
                gappiv: if self.enable_gaps { mon.gappiv } else { 0 },
                gappoh: if self.enable_gaps { mon.gappoh } else { 0 },
                gappov: if self.enable_gaps { mon.gappov } else { 0 },
            }
        };
        let visible_in_pass = |c: &MargoClient| {
            c.is_visible_on(mon_idx, tagset)
                && (!is_overview || (!c.is_minimized && !c.is_killing && !c.is_in_scratchpad))
        };

        let tiled: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                visible_in_pass(c)
                    && (is_overview || c.is_tiled())
            })
            .map(|(i, _)| i)
            .collect();

        let scroller_proportions: Vec<f32> =
            tiled.iter().map(|&i| self.clients[i].scroller_proportion).collect();
        let focused_tiled_pos = self
            .focused_client_idx()
            .and_then(|focused_idx| tiled.iter().position(|&idx| idx == focused_idx));

        if !is_overview && self.config.smartgaps && tiled.len() <= 1 {
            gaps.gappoh = 0;
            gaps.gappov = 0;
        }

        let ctx = layout::ArrangeCtx {
            work_area,
            tiled: &tiled,
            nmaster,
            mfact,
            gaps: &gaps,
            scroller_proportions: &scroller_proportions,
            default_scroller_proportion: self.config.scroller_default_proportion,
            focused_tiled_pos,
            scroller_structs: self.config.scroller_structs,
            scroller_focus_center: self.config.scroller_focus_center,
            scroller_prefer_center: self.config.scroller_prefer_center,
            scroller_prefer_overspread: self.config.scroller_prefer_overspread,
        };

        let geometries = layout::arrange(layout, &ctx);
        let now = crate::utils::now_ms();
        for (client_idx, mut rect) in geometries {
            // Apply per-client size constraints from window rules. The layout
            // algorithm is constraint-agnostic; we clamp post-hoc so that
            // e.g. picture-in-picture players keep their pinned dimensions
            // even when the surrounding scroller column would prefer wider.
            let c = &self.clients[client_idx];
            if c.min_width > 0 || c.min_height > 0 || c.max_width > 0 || c.max_height > 0 {
                clamp_size(
                    &mut rect.width,
                    &mut rect.height,
                    c.min_width,
                    c.min_height,
                    c.max_width,
                    c.max_height,
                );
            }
            let old = self.clients[client_idx].geom;
            let should_animate = self.config.animations
                && !self.clients[client_idx].no_animation
                && !self.clients[client_idx].is_tag_switching
                && old.width > 0
                && old.height > 0
                && old != rect;
            if should_animate {
                self.clients[client_idx].animation = ClientAnimation {
                    should_animate: true,
                    running: true,
                    time_started: now,
                    duration: self.config.animation_duration_move.max(1),
                    initial: old,
                    current: rect,
                    action: AnimationType::Move,
                    ..Default::default()
                };
                self.clients[client_idx].geom = old;
            } else {
                self.clients[client_idx].animation.running = false;
                self.clients[client_idx].geom = rect;
            }
            self.clients[client_idx].is_tag_switching = false;
        }

        // Apply fullscreen / floating overrides outside overview. Overview
        // intentionally thumbnails every visible window in the grid.
        if !is_overview {
            for i in 0..self.clients.len() {
                let c = &self.clients[i];
                if c.monitor != mon_idx || !visible_in_pass(c) {
                    continue;
                }
                if c.is_fullscreen {
                    self.clients[i].geom = monitor_area;
                } else if c.is_floating && c.float_geom.width > 0 {
                    self.clients[i].geom = self.clients[i].float_geom;
                }
            }
        }

        // Collect windows to show/hide (avoid borrow conflict during space ops)
        let visible: Vec<(Window, Rect, Rect)> = self
            .clients
            .iter()
            .filter(|c| visible_in_pass(c))
            .map(|c| {
                let configure_geom = if c.animation.running { c.animation.current } else { c.geom };
                (c.window.clone(), c.geom, configure_geom)
            })
            .collect();

        let hidden: Vec<Window> = self
            .clients
            .iter()
            .filter(|c| c.monitor == mon_idx && !visible_in_pass(c))
            .map(|c| c.window.clone())
            .collect();

        for w in hidden {
            self.space.unmap_elem(&w);
        }

        for (window, geom, configure_geom) in visible {
            self.space.map_element(window.clone(), (geom.x, geom.y), false);

            if let WindowSurface::Wayland(toplevel) = window.underlying_surface() {
                tracing::debug!(
                    "arrange: setting toplevel size {}x{}",
                    configure_geom.width,
                    configure_geom.height
                );
                toplevel.with_pending_state(|state| {
                    state.size = Some(Size::from((configure_geom.width, configure_geom.height)));
                });
                // Only send the configure if the initial configure has already
                // gone out. The initial configure must be sent during the first
                // commit (see CompositorHandler::commit).
                let initial_sent = with_states(toplevel.wl_surface(), |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                        .unwrap_or(false)
                });
                if initial_sent {
                    toplevel.send_pending_configure();
                }
            }
        }
        crate::border::refresh(self);
        self.request_repaint();
    }

    pub fn focus_surface(&mut self, target: Option<FocusTarget>) {
        // Track focus history per-monitor so toplevel_destroyed can recall
        // the previously focused window (niri-style).
        if let Some(FocusTarget::Window(ref w)) = target {
            if let Some(new_idx) = self.clients.iter().position(|c| c.window == *w) {
                let mon_idx = self.clients[new_idx].monitor;
                if mon_idx < self.monitors.len() {
                    let cur = self.monitors[mon_idx].selected;
                    if cur != Some(new_idx) {
                        self.monitors[mon_idx].prev_selected = cur;
                        self.monitors[mon_idx].selected = Some(new_idx);
                    }
                }
            }
        }

        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, target, serial);
        }
        // Refresh border colors so the focused/unfocused distinction
        // updates without waiting for the next arrange.
        crate::border::refresh(self);
        self.request_repaint();
    }

    pub fn post_repaint(&mut self, output: &Output, time: impl Into<std::time::Duration>) {
        let time = time.into();
        let throttle = Some(std::time::Duration::from_secs(1));

        self.space.elements().for_each(|window| {
            if self.space.outputs_for_element(window).contains(output) {
                window.send_frame(output, time, throttle, |_, _| Some(output.clone()));
            }
        });

        let map = layer_map_for_output(output);
        for layer in map.layers() {
            layer.send_frame(output, time, throttle, |_, _| Some(output.clone()));
        }

        self.space.refresh();
        self.popups.cleanup();
    }

    // ── Focus helpers ─────────────────────────────────────────────────────────

    pub fn focused_client_idx(&self) -> Option<usize> {
        let keyboard = self.seat.get_keyboard()?;
        let focus = keyboard.current_focus()?;
        if let FocusTarget::Window(focused) = focus {
            self.clients.iter().position(|c| c.window == focused)
        } else {
            None
        }
    }

    pub fn focused_monitor(&self) -> usize {
        self.focused_client_idx()
            .map(|i| self.clients[i].monitor)
            .or_else(|| self.pointer_monitor())
            .unwrap_or(0)
    }

    /// For scroller layout, return the client-vector index where a newly
    /// created window should land — right after the currently focused client
    /// on the same monitor. Returns `None` if the target monitor isn't using
    /// scroller (any layout) or if there's no focused client there.
    fn scroller_insert_position(&self, target_mon: usize) -> Option<usize> {
        let mon = self.monitors.get(target_mon)?;
        if mon.current_layout() != crate::layout::LayoutId::Scroller {
            return None;
        }
        let focused_idx = self.focused_client_idx()?;
        if self.clients[focused_idx].monitor != target_mon {
            return None;
        }
        Some(focused_idx + 1)
    }

    /// Inserting a client mid-vec invalidates any monitor.selected /
    /// prev_selected indices that pointed at positions ≥ insert position.
    /// Bump them up by one so they keep referring to the same client.
    fn shift_indices_at_or_after(&mut self, insert_pos: usize) {
        for mon in self.monitors.iter_mut() {
            if let Some(s) = mon.selected.as_mut() {
                if *s >= insert_pos {
                    *s += 1;
                }
            }
            if let Some(s) = mon.prev_selected.as_mut() {
                if *s >= insert_pos {
                    *s += 1;
                }
            }
        }
    }

    /// Inverse of `shift_indices_at_or_after`: a client at `removed_pos` was
    /// just dropped. Shift any monitor index pointing at a later position
    /// down by one, and clear those that pointed exactly at the removed slot.
    fn shift_indices_after_remove(&mut self, removed_pos: usize) {
        for mon in self.monitors.iter_mut() {
            for slot in [&mut mon.selected, &mut mon.prev_selected] {
                if let Some(s) = slot.as_mut() {
                    if *s == removed_pos {
                        *slot = None;
                    } else if *s > removed_pos {
                        *s -= 1;
                    }
                }
            }
        }
    }

    fn pointer_monitor(&self) -> Option<usize> {
        self.monitor_at_point(self.input_pointer.x, self.input_pointer.y)
    }

    fn monitor_at_point(&self, x: f64, y: f64) -> Option<usize> {
        self.monitors.iter().position(|mon| {
            let area = mon.monitor_area;
            x >= area.x as f64
                && y >= area.y as f64
                && x < (area.x + area.width) as f64
                && y < (area.y + area.height) as f64
        })
    }

    pub fn clamp_pointer_to_outputs(&mut self) {
        if self.monitors.is_empty() {
            return;
        }

        let mut min_x = self.monitors[0].monitor_area.x;
        let mut min_y = self.monitors[0].monitor_area.y;
        let mut max_x = self.monitors[0].monitor_area.x + self.monitors[0].monitor_area.width;
        let mut max_y = self.monitors[0].monitor_area.y + self.monitors[0].monitor_area.height;

        for mon in &self.monitors[1..] {
            let area = mon.monitor_area;
            min_x = min_x.min(area.x);
            min_y = min_y.min(area.y);
            max_x = max_x.max(area.x + area.width);
            max_y = max_y.max(area.y + area.height);
        }

        self.input_pointer.x = self.input_pointer.x.clamp(min_x as f64, (max_x - 1) as f64);
        self.input_pointer.y = self.input_pointer.y.clamp(min_y as f64, (max_y - 1) as f64);
    }

    pub fn default_layout(&self) -> LayoutId {
        LayoutId::from_name(&self.config.default_layout).unwrap_or(LayoutId::Tile)
    }

    pub fn apply_tag_rules_to_monitor(&mut self, mon_idx: usize) {
        let Some(mon) = self.monitors.get_mut(mon_idx) else {
            return;
        };

        for rule in &self.config.tag_rules {
            if rule.id <= 0 || rule.id as usize > crate::MAX_TAGS {
                continue;
            }
            if let Some(name) = &rule.monitor_name {
                if name != &mon.name {
                    continue;
                }
            }

            let tag = rule.id as usize;
            if let Some(layout_name) = &rule.layout_name {
                if let Some(layout) = LayoutId::from_name(layout_name) {
                    mon.pertag.ltidxs[tag] = layout;
                }
            }
            if rule.mfact > 0.0 {
                mon.pertag.mfacts[tag] = rule.mfact.clamp(0.05, 0.95);
            }
            if rule.nmaster > 0 {
                mon.pertag.nmasters[tag] = rule.nmaster as u32;
            }
        }
    }

    fn focus_first_visible_or_clear(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            self.focus_surface(None);
            return;
        }

        let tagset = self.monitors[mon_idx].current_tagset();
        if let Some(idx) = self.clients.iter().position(|c| c.is_visible_on(mon_idx, tagset)) {
            self.monitors[mon_idx].selected = Some(idx);
            let window = self.clients[idx].window.clone();
            self.focus_surface(Some(FocusTarget::Window(window)));
        } else {
            self.monitors[mon_idx].selected = None;
            self.focus_surface(None);
        }
    }

    fn update_pertag_for_tagset(&mut self, mon_idx: usize, tagmask: u32) {
        let Some(mon) = self.monitors.get_mut(mon_idx) else {
            return;
        };

        mon.pertag.prevtag = mon.pertag.curtag;
        mon.pertag.curtag = if tagmask.count_ones() == 1 {
            tagmask.trailing_zeros() as usize + 1
        } else {
            0
        };
    }

    pub fn is_overview_open(&self) -> bool {
        self.monitors.iter().any(|mon| mon.is_overview)
    }

    pub fn open_overview(&mut self) {
        let mut changed = false;
        for mon in &mut self.monitors {
            if !mon.is_overview {
                mon.overview_backup_tagset = mon.current_tagset().max(1);
                mon.is_overview = true;
                changed = true;
            }
        }

        if changed {
            self.arrange_all();
            crate::protocols::dwl_ipc::broadcast_all(self);
        }
    }

    pub fn close_overview(&mut self, activate_window: Option<Window>) {
        let was_open = self.is_overview_open();
        if !was_open {
            return;
        }

        let previous_focus = self.focused_client_idx();
        let activate_idx = activate_window
            .as_ref()
            .and_then(|window| self.clients.iter().position(|client| &client.window == window));

        for mon_idx in 0..self.monitors.len() {
            if !self.monitors[mon_idx].is_overview {
                continue;
            }

            let seltags = self.monitors[mon_idx].seltags;
            let backup = self.monitors[mon_idx].overview_backup_tagset.max(1);
            let target_tagset = activate_idx
                .filter(|&idx| self.clients[idx].monitor == mon_idx)
                .map(|idx| {
                    let tags = self.clients[idx].tags;
                    let backup_intersection = tags & backup;
                    if backup_intersection != 0 {
                        backup_intersection
                    } else {
                        tags & tags.wrapping_neg()
                    }
                })
                .filter(|tagset| *tagset != 0)
                .unwrap_or(backup);

            self.monitors[mon_idx].is_overview = false;
            self.monitors[mon_idx].tagset[seltags] = target_tagset;
            self.update_pertag_for_tagset(mon_idx, target_tagset);
        }

        self.arrange_all();

        let focus_idx = activate_idx.or(previous_focus).filter(|&idx| {
            self.monitors
                .get(self.clients[idx].monitor)
                .is_some_and(|mon| {
                    self.clients[idx].is_visible_on(
                        self.clients[idx].monitor,
                        mon.current_tagset(),
                    )
                })
        });

        if let Some(idx) = focus_idx {
            let mon_idx = self.clients[idx].monitor;
            if mon_idx < self.monitors.len() {
                self.monitors[mon_idx].selected = Some(idx);
            }
            let window = self.clients[idx].window.clone();
            self.focus_surface(Some(FocusTarget::Window(window)));
        } else {
            let mon_idx = self.focused_monitor();
            self.focus_first_visible_or_clear(mon_idx);
        }

        crate::protocols::dwl_ipc::broadcast_all(self);
    }

    pub fn toggle_overview(&mut self) {
        if self.is_overview_open() {
            self.close_overview(None);
        } else {
            self.open_overview();
        }
    }

    fn apply_window_rules(&self, client: &mut MargoClient) {
        let rules = self.matching_window_rules(&client.app_id, &client.title);
        Self::apply_matched_window_rules(&self.monitors, client, &rules);
    }

    fn apply_window_rules_to_client(&mut self, idx: usize) -> bool {
        if idx >= self.clients.len() {
            return false;
        }
        let (app_id, title) = {
            let client = &self.clients[idx];
            (client.app_id.clone(), client.title.clone())
        };
        let rules = self.matching_window_rules(&app_id, &title);
        if rules.is_empty() {
            return false;
        }
        Self::apply_matched_window_rules(&self.monitors, &mut self.clients[idx], &rules);
        true
    }

    fn matching_window_rules(&self, app_id: &str, title: &str) -> Vec<WindowRule> {
        self.config
            .window_rules
            .iter()
            .filter(|rule| self.window_rule_matches(rule, app_id, title))
            .cloned()
            .collect()
    }

    fn apply_matched_window_rules(
        monitors: &[MargoMonitor],
        client: &mut MargoClient,
        rules: &[WindowRule],
    ) {
        for rule in rules {
            if rule.tags != 0 {
                client.tags = rule.tags;
            }
            if let Some(monitor_name) = &rule.monitor {
                if let Some(mon_idx) = monitors.iter().position(|mon| &mon.name == monitor_name) {
                    client.monitor = mon_idx;
                }
            }

            if let Some(value) = rule.is_floating {
                client.is_floating = value;
            }
            if let Some(value) = rule.is_fullscreen {
                client.is_fullscreen = value;
            }
            if let Some(value) = rule.is_fake_fullscreen {
                client.is_fake_fullscreen = value;
            }
            if let Some(value) = rule.no_border {
                client.no_border = value;
            }
            if let Some(value) = rule.no_shadow {
                client.no_shadow = value;
            }
            if let Some(value) = rule.no_radius {
                client.no_radius = value;
            }
            if let Some(value) = rule.no_animation {
                client.no_animation = value;
            }
            if let Some(value) = rule.border_width {
                client.border_width = value;
            }
            if let Some(value) = rule.open_silent {
                client.open_silent = value;
            }
            if let Some(value) = rule.tag_silent {
                client.tag_silent = value;
            }
            if let Some(value) = rule.is_named_scratchpad {
                client.is_named_scratchpad = value;
            }
            if let Some(value) = rule.is_unglobal {
                client.is_unglobal = value;
            }
            if let Some(value) = rule.is_global {
                client.is_global = value;
            }
            if let Some(value) = rule.is_overlay {
                client.is_overlay = value;
            }
            if let Some(value) = rule.no_focus {
                client.no_focus = value;
            }
            if let Some(value) = rule.no_fade_in {
                client.no_fade_in = value;
            }
            if let Some(value) = rule.no_fade_out {
                client.no_fade_out = value;
            }
            if let Some(value) = rule.is_term {
                client.is_term = value;
            }
            if let Some(value) = rule.allow_csd {
                client.allow_csd = value;
            }
            if let Some(value) = rule.force_fake_maximize {
                client.force_fake_maximize = value;
            }
            if let Some(value) = rule.force_tiled_state {
                client.force_tiled_state = value;
                if value {
                    client.is_floating = false;
                }
            }
            if let Some(value) = rule.no_swallow {
                client.no_swallow = value;
            }
            if let Some(value) = rule.no_blur {
                client.no_blur = value;
            }
            if let Some(value) = rule.canvas_no_tile {
                client.canvas_no_tile = value;
            }
            if let Some(value) = rule.scroller_proportion {
                client.scroller_proportion = value.clamp(0.1, 1.0);
            }
            if let Some(value) = rule.scroller_proportion_single {
                client.scroller_proportion_single = value.clamp(0.1, 1.0);
            }
            if let Some(value) = rule.focused_opacity {
                client.focused_opacity = value.clamp(0.0, 1.0);
            }
            if let Some(value) = rule.unfocused_opacity {
                client.unfocused_opacity = value.clamp(0.0, 1.0);
            }
            // Niri-style additions.
            if rule.min_width > 0 {
                client.min_width = rule.min_width;
            }
            if rule.min_height > 0 {
                client.min_height = rule.min_height;
            }
            if rule.max_width > 0 {
                client.max_width = rule.max_width;
            }
            if rule.max_height > 0 {
                client.max_height = rule.max_height;
            }
            if let Some(focused) = rule.open_focused {
                // open_focused=false → equivalent to no_focus=true
                client.no_focus = !focused;
            }
            if let Some(value) = rule.block_out_from_screencast {
                client.block_out_from_screencast = value;
            }
            if rule.width > 0 || rule.height > 0 || rule.offset_x != 0 || rule.offset_y != 0 {
                client.is_floating = true;
                client.float_geom = Self::rule_float_geometry_for(monitors, client.monitor, rule);
            }
        }
        // After all matched rules are applied, clamp the floating geometry
        // to any size constraints picked up.
        clamp_size(
            &mut client.float_geom.width,
            &mut client.float_geom.height,
            client.min_width,
            client.min_height,
            client.max_width,
            client.max_height,
        );
    }

    fn window_rule_matches(&self, rule: &WindowRule, app_id: &str, title: &str) -> bool {
        // Positive matches: every present pattern must match.
        let app_ok = rule
            .id
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(|p| matches_rule_text(p, app_id))
            .unwrap_or(true);
        let title_ok = rule
            .title
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(|p| matches_rule_text(p, title))
            .unwrap_or(true);
        if !(app_ok && title_ok) {
            return false;
        }

        // Negative matches (niri-style): if either exclude pattern matches,
        // the rule is rejected even if the positive matches succeed.
        if let Some(p) = rule.exclude_id.as_deref().filter(|p| !p.is_empty()) {
            if matches_rule_text(p, app_id) {
                return false;
            }
        }
        if let Some(p) = rule.exclude_title.as_deref().filter(|p| !p.is_empty()) {
            if matches_rule_text(p, title) {
                return false;
            }
        }
        true
    }

    fn rule_float_geometry(&self, mon_idx: usize, rule: &WindowRule) -> Rect {
        Self::rule_float_geometry_for(&self.monitors, mon_idx, rule)
    }

    fn rule_float_geometry_for(monitors: &[MargoMonitor], mon_idx: usize, rule: &WindowRule) -> Rect {
        let area = monitors
            .get(mon_idx)
            .map(|mon| mon.work_area)
            .unwrap_or_else(|| Rect::new(0, 0, 1280, 720));
        let width = if rule.width > 0 {
            rule.width.min(area.width)
        } else {
            (area.width as f32 * 0.6) as i32
        };
        let height = if rule.height > 0 {
            rule.height.min(area.height)
        } else {
            (area.height as f32 * 0.6) as i32
        };

        Rect::new(
            area.x + (area.width - width) / 2 + rule.offset_x,
            area.y + (area.height - height) / 2 + rule.offset_y,
            width,
            height,
        )
    }

    fn refresh_wayland_toplevel_identity(&mut self, window: &Window, toplevel: &ToplevelSurface) {
        let (app_id, title) = read_toplevel_identity(toplevel);
        let Some(idx) = self.clients.iter().position(|client| client.window == *window) else {
            return;
        };

        let (app_id_changed, title_changed, old_monitor, handle) = {
            let client = &mut self.clients[idx];
            let app_id_changed = client.app_id != app_id;
            let title_changed = client.title != title;
            if !app_id_changed && !title_changed {
                return;
            }

            let old_monitor = client.monitor;
            let handle = client.foreign_toplevel_handle.clone();
            client.app_id = app_id.clone();
            client.title = title.clone();
            (app_id_changed, title_changed, old_monitor, handle)
        };

        if let Some(handle) = handle {
            if app_id_changed {
                handle.send_app_id(&app_id);
            }
            if title_changed {
                handle.send_title(&title);
            }
            handle.send_done();
        }

        let title_rules_exist = self.config.window_rules.iter().any(|rule| {
            rule.title.as_ref().is_some_and(|pattern| !pattern.is_empty())
                || rule.exclude_title.as_ref().is_some_and(|pattern| !pattern.is_empty())
        });
        let should_reapply_rules = (app_id_changed && !app_id.is_empty())
            || (title_changed && !title.is_empty() && title_rules_exist);

        if should_reapply_rules && self.apply_window_rules_to_client(idx) {
            let new_monitor = self.clients[idx].monitor;
            if old_monitor != new_monitor {
                self.arrange_monitor(old_monitor);
            }
            self.arrange_monitor(new_monitor);
            crate::protocols::dwl_ipc::broadcast_all(self);
        }
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    pub fn kill_focused(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            if let WindowSurface::Wayland(toplevel) = self.clients[idx].window.underlying_surface() {
                toplevel.send_close();
            }
        }
    }

    pub fn focus_stack(&mut self, direction: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();

        let visible: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_visible_on(mon_idx, tagset))
            .map(|(i, _)| i)
            .collect();

        if visible.is_empty() {
            return;
        }

        let len = visible.len();
        let current_pos = self
            .focused_client_idx()
            .and_then(|ci| visible.iter().position(|&vi| vi == ci))
            .unwrap_or(0);

        let new_pos = if direction > 0 {
            (current_pos + 1) % len
        } else {
            (current_pos + len - 1) % len
        };

        let new_idx = visible[new_pos];
        self.monitors[mon_idx].prev_selected = self.monitors[mon_idx].selected;
        self.monitors[mon_idx].selected = Some(new_idx);
        let window = self.clients[new_idx].window.clone();
        self.focus_surface(Some(FocusTarget::Window(window)));
        self.arrange_monitor(mon_idx);
    }

    pub fn exchange_stack(&mut self, direction: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();

        let visible: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_visible_on(mon_idx, tagset))
            .map(|(i, _)| i)
            .collect();

        if visible.len() < 2 {
            return;
        }

        let Some(current_idx) = self.focused_client_idx() else {
            return;
        };
        let Some(current_pos) = visible.iter().position(|&idx| idx == current_idx) else {
            return;
        };

        let len = visible.len();
        let target_pos = if direction > 0 {
            (current_pos + 1) % len
        } else {
            (current_pos + len - 1) % len
        };
        let target_idx = visible[target_pos];
        let window = self.clients[current_idx].window.clone();
        self.clients.swap(current_idx, target_idx);
        self.arrange_monitor(mon_idx);
        self.focus_surface(Some(FocusTarget::Window(window)));
    }

    pub fn view_tag(&mut self, tagmask: u32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() || tagmask == 0 {
            return;
        }
        let seltags = self.monitors[mon_idx].seltags;
        let current = self.monitors[mon_idx].tagset[seltags];

        // dwm/mango pattern: tagset has two slots. The "active" slot is
        // tagset[seltags]; the other slot remembers the previously viewed
        // tagmask. If the user re-presses the binding for the *current*
        // tag, swap the two slots so we land on the previous tag — like
        // alt-tab for workspaces.
        let new_tagmask = if current == tagmask {
            let other = self.monitors[mon_idx].tagset[seltags ^ 1];
            if other == 0 || other == current {
                // No meaningful previous tag → no-op (don't toggle into
                // an empty/identical state).
                return;
            }
            self.monitors[mon_idx].seltags = seltags ^ 1;
            other
        } else {
            // First press of a different tag: stash current as "previous"
            // in the other slot, then write new mask into active slot.
            self.monitors[mon_idx].tagset[seltags ^ 1] = current;
            self.monitors[mon_idx].tagset[seltags] = tagmask;
            tagmask
        };

        self.update_pertag_for_tagset(mon_idx, new_tagmask);
        self.arrange_monitor(mon_idx);
        self.focus_first_visible_or_clear(mon_idx);
        crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
    }

    pub fn toggle_view_tag(&mut self, tagmask: u32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let seltags = self.monitors[mon_idx].seltags;
        let current = self.monitors[mon_idx].tagset[seltags];
        let new = current ^ tagmask;
        if new != 0 {
            self.monitors[mon_idx].tagset[seltags] = new;
            self.update_pertag_for_tagset(mon_idx, new);
            self.arrange_monitor(mon_idx);
            self.focus_first_visible_or_clear(mon_idx);
            crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
        }
    }

    pub fn view_relative(&mut self, delta: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() || delta == 0 {
            return;
        }
        let current = self.monitors[mon_idx].current_tagset();
        let current_tag = if current.count_ones() == 1 {
            current.trailing_zeros() as i32
        } else {
            0
        };
        let max = crate::MAX_TAGS as i32;
        let next = (current_tag + delta).rem_euclid(max);
        self.view_tag(1u32 << next);
    }

    pub fn tag_focused(&mut self, tagmask: u32) {
        if tagmask == 0 {
            return;
        }
        let Some(idx) = self.focused_client_idx() else {
            return;
        };

        let mon_idx = self.clients[idx].monitor;
        if mon_idx >= self.monitors.len() {
            return;
        }
        self.clients[idx].old_tags = self.clients[idx].tags;
        self.clients[idx].is_tag_switching = true;
        self.clients[idx].animation.running = false;
        self.clients[idx].tags = tagmask;
        self.arrange_monitor(mon_idx);

        if !self.clients[idx].is_visible_on(mon_idx, self.monitors[mon_idx].current_tagset()) {
            self.focus_first_visible_or_clear(mon_idx);
        }

        crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
    }

    pub fn tag_relative(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let current = self.clients[idx].tags;
        let current_tag = if current.count_ones() == 1 {
            current.trailing_zeros() as i32
        } else {
            self.monitors
                .get(self.clients[idx].monitor)
                .map(|mon| mon.current_tagset().trailing_zeros() as i32)
                .unwrap_or(0)
        };
        let max = crate::MAX_TAGS as i32;
        let next = (current_tag + delta).rem_euclid(max);
        self.tag_focused(1u32 << next);
    }

    pub fn toggle_client_tag(&mut self, tagmask: u32) {
        let Some(idx) = self.focused_client_idx() else {
            return;
        };

        let mon_idx = self.clients[idx].monitor;
        if mon_idx >= self.monitors.len() {
            return;
        }
        let new = self.clients[idx].tags ^ tagmask;
        if new != 0 {
            self.clients[idx].old_tags = self.clients[idx].tags;
            self.clients[idx].is_tag_switching = true;
            self.clients[idx].animation.running = false;
            self.clients[idx].tags = new;
            self.arrange_monitor(mon_idx);

            if !self.clients[idx].is_visible_on(mon_idx, self.monitors[mon_idx].current_tagset()) {
                self.focus_first_visible_or_clear(mon_idx);
            }

            crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
        }
    }

    pub fn set_layout(&mut self, name: &str) {
        if let Some(layout) = LayoutId::from_name(name) {
            let mon_idx = self.focused_monitor();
            if mon_idx >= self.monitors.len() {
                return;
            }
            let curtag = self.monitors[mon_idx].pertag.curtag;
            self.monitors[mon_idx].pertag.ltidxs[curtag] = layout;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn switch_layout(&mut self) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let current = self.monitors[mon_idx].current_layout().name();
        let layouts: Vec<String> = if self.config.circle_layouts.is_empty() {
            vec!["tile", "scroller", "grid", "monocle", "deck"]
                .into_iter()
                .map(str::to_string)
                .collect()
        } else {
            self.config.circle_layouts.clone()
        };
        if layouts.is_empty() {
            return;
        }
        let current_pos = layouts.iter().position(|name| name == current).unwrap_or(0);
        let next = layouts[(current_pos + 1) % layouts.len()].clone();
        self.set_layout(&next);
    }

    pub fn toggle_floating(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            self.clients[idx].is_floating = !self.clients[idx].is_floating;
            if self.clients[idx].is_floating && self.clients[idx].float_geom.width == 0 {
                self.clients[idx].float_geom = self.clients[idx].geom;
            }
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn set_focused_proportion(&mut self, proportion: f32) {
        if let Some(idx) = self.focused_client_idx() {
            self.clients[idx].scroller_proportion = proportion.clamp(0.1, 1.0);
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn switch_focused_proportion_preset(&mut self) {
        if self.config.scroller_proportion_presets.is_empty() {
            return;
        }
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let current = self.clients[idx].scroller_proportion;
        let presets = &self.config.scroller_proportion_presets;
        let current_pos = presets
            .iter()
            .position(|value| (*value - current).abs() < 0.01)
            .unwrap_or(0);
        self.clients[idx].scroller_proportion = presets[(current_pos + 1) % presets.len()];
        let mon_idx = self.clients[idx].monitor;
        self.arrange_monitor(mon_idx);
    }

    pub fn toggle_fullscreen(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            self.clients[idx].is_fullscreen = !self.clients[idx].is_fullscreen;
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn inc_nmaster(&mut self, delta: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let curtag = self.monitors[mon_idx].pertag.curtag;
        let current = self.monitors[mon_idx].pertag.nmasters[curtag] as i32;
        self.monitors[mon_idx].pertag.nmasters[curtag] = (current + delta).max(0) as u32;
        self.arrange_monitor(mon_idx);
    }

    pub fn set_mfact(&mut self, delta: f32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let curtag = self.monitors[mon_idx].pertag.curtag;
        let current = self.monitors[mon_idx].pertag.mfacts[curtag];
        self.monitors[mon_idx].pertag.mfacts[curtag] = (current + delta).clamp(0.05, 0.95);
        self.arrange_monitor(mon_idx);
    }

    pub fn toggle_gaps(&mut self) {
        self.enable_gaps = !self.enable_gaps;
        for mon_idx in 0..self.monitors.len() {
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn inc_gaps(&mut self, delta: i32) {
        let mon_idx = self.focused_monitor();
        if let Some(mon) = self.monitors.get_mut(mon_idx) {
            mon.gappih = (mon.gappih + delta).max(0);
            mon.gappiv = (mon.gappiv + delta).max(0);
            mon.gappoh = (mon.gappoh + delta).max(0);
            mon.gappov = (mon.gappov + delta).max(0);
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn move_focused(&mut self, dx: i32, dy: i32) {
        if let Some(idx) = self.focused_client_idx() {
            if self.clients[idx].float_geom.width == 0 {
                self.clients[idx].float_geom = self.clients[idx].geom;
            }
            self.clients[idx].is_floating = true;
            self.clients[idx].float_geom.x += dx;
            self.clients[idx].float_geom.y += dy;
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn resize_focused(&mut self, dw: i32, dh: i32) {
        if let Some(idx) = self.focused_client_idx() {
            if self.clients[idx].float_geom.width == 0 {
                self.clients[idx].float_geom = self.clients[idx].geom;
            }
            self.clients[idx].is_floating = true;
            self.clients[idx].float_geom.width = (self.clients[idx].float_geom.width + dw).max(50);
            self.clients[idx].float_geom.height = (self.clients[idx].float_geom.height + dh).max(50);
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
        }
    }

    pub fn zoom(&mut self) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();
        let Some(focused_idx) = self.focused_client_idx() else {
            return;
        };

        let tiled: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_visible_on(mon_idx, tagset) && c.is_tiled())
            .map(|(i, _)| i)
            .collect();

        if tiled.len() < 2 {
            return;
        }

        let focused_pos = tiled.iter().position(|&i| i == focused_idx);
        let (a, b) = if focused_pos == Some(0) {
            (tiled[0], tiled[1])
        } else if let Some(pos) = focused_pos {
            (tiled[0], tiled[pos])
        } else {
            return;
        };

        self.clients.swap(a, b);
        self.arrange_monitor(mon_idx);
    }

    pub fn focus_mon(&mut self, direction: i32) {
        if self.monitors.len() <= 1 {
            return;
        }
        let current = self.focused_monitor();
        let len = self.monitors.len();
        let next = if direction > 0 {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };

        let tagset = self.monitors[next].current_tagset();
        if let Some(idx) = self.clients.iter().position(|c| c.is_visible_on(next, tagset)) {
            let window = self.clients[idx].window.clone();
            self.focus_surface(Some(FocusTarget::Window(window)));
        }
    }

    pub fn tag_mon(&mut self, direction: i32) {
        if self.monitors.len() <= 1 {
            return;
        }
        let Some(idx) = self.focused_client_idx() else {
            return;
        };
        let current_mon = self.clients[idx].monitor;
        let len = self.monitors.len();
        let target_mon = if direction > 0 {
            (current_mon + 1) % len
        } else {
            (current_mon + len - 1) % len
        };
        let tagset = self.monitors[target_mon].current_tagset();
        self.clients[idx].monitor = target_mon;
        self.clients[idx].tags = tagset;
        self.arrange_monitor(current_mon);
        self.arrange_monitor(target_mon);
    }
}

// ── Smithay delegate: Compositor ──────────────────────────────────────────────

impl CompositorHandler for MargoState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(state) = client.get_data::<XWaylandClientData>() {
            return &state.compositor_state;
        }
        if let Some(state) = client.get_data::<MargoClientData>() {
            return &state.compositor_state;
        }
        panic!("client_compositor_state: unknown client data type")
    }
    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }

            if self.session_locked {
                if self.lock_surfaces.iter().any(|(_, s)| s.wl_surface() == &root) {
                    self.request_repaint();
                    return;
                }
            }

            let committed_window = self
                .space
                .elements()
                .find(|w| w.wl_surface().as_deref() == Some(&root))
                .cloned();
            if let Some(window) = committed_window {
                window.on_commit();
                // Send the initial configure on first commit if not yet sent.
                // xdg-shell clients perform an initial bufferless commit after
                // role assignment and then wait for this configure.
                if let WindowSurface::Wayland(toplevel) = window.underlying_surface() {
                    self.refresh_wayland_toplevel_identity(&window, &toplevel);
                    let initial_sent = with_states(toplevel.wl_surface(), |states| {
                        states
                            .data_map
                            .get::<XdgToplevelSurfaceData>()
                            .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                            .unwrap_or(false)
                    });
                    if !initial_sent {
                        tracing::debug!("sending initial configure for toplevel");
                        toplevel.send_configure();
                    } else {
                        tracing::trace!("commit on already-configured toplevel");
                    }
                }
                // Re-derive border geometry from the freshly-committed
                // window_geometry. Clients (notably Electron — Helium /
                // Spotify) sometimes commit at a smaller size than we
                // asked them to, and without this refresh the border
                // stays drawn around the larger layout-reserved rect,
                // leaving a wallpaper strip between the visible window
                // and its frame.
                crate::border::refresh(self);
            }

            let layer_output = self.space.outputs().find_map(|output| {
                let map = layer_map_for_output(output);
                if map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL).is_some() {
                    Some(output.clone())
                } else {
                    None
                }
            });

            if let Some(output) = layer_output {
                let initial_sent = with_states(&root, |states| {
                    states
                        .data_map
                        .get::<LayerSurfaceData>()
                        .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                        .unwrap_or(false)
                });

                {
                    let mut map = layer_map_for_output(&output);
                    map.arrange();
                    if !initial_sent {
                        if let Some(layer) = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL) {
                            tracing::debug!("sending initial configure for layer surface");
                            layer.layer_surface().send_configure();
                        }
                    }
                }

                self.refresh_output_work_area(&output);
            }
        }
        self.popups.commit(surface);
        self.request_repaint();
    }
}
delegate_compositor!(MargoState);

impl BufferHandler for MargoState {
    fn buffer_destroyed(
        &mut self,
        _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    ) {
    }
}

impl DmabufHandler for MargoState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        let imported = self
            .dmabuf_import_hook
            .as_ref()
            .map(|hook| {
                let mut import = hook.borrow_mut();
                (*import)(&dmabuf)
            })
            .unwrap_or(true);

        if imported {
            let _ = notifier.successful::<Self>();
        } else {
            notifier.failed();
        }
    }
}
delegate_dmabuf!(MargoState);

// ── Smithay delegate: XDG Shell ───────────────────────────────────────────────

impl XdgShellHandler for MargoState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }
    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let (app_id, title) = read_toplevel_identity(&surface);

        let window = Window::new_wayland_window(surface.clone());
        let mon_idx = self.focused_monitor();
        let initial_tags = self.monitors.get(mon_idx).map(|m| m.current_tagset()).unwrap_or(1);
        let mut client = MargoClient::new(window.clone(), mon_idx, initial_tags, &self.config);
        client.app_id = app_id.clone();
        client.title = title.clone();
        self.apply_window_rules(&mut client);
        let target_mon = client.monitor;
        let focus_new = !client.no_focus && !client.open_silent;

        let ft_handle = self.foreign_toplevel_list.new_toplevel::<Self>(&title, &app_id);
        ft_handle.send_done();
        client.foreign_toplevel_handle = Some(ft_handle);

        // Smart-insert (niri pattern): in scroller layout, place the new
        // client right after the focused one so closing it returns you near
        // your previous position. Other layouts are order-agnostic.
        let insert_at = self.scroller_insert_position(target_mon);
        let new_idx = match insert_at {
            Some(pos) => {
                self.clients.insert(pos, client);
                self.shift_indices_at_or_after(pos);
                pos
            }
            None => {
                self.clients.push(client);
                self.clients.len() - 1
            }
        };

        let map_loc = self
            .monitors
            .get(target_mon)
            .map(|m| (m.monitor_area.x, m.monitor_area.y))
            .unwrap_or((0, 0));
        self.space.map_element(window.clone(), map_loc, true);
        if focus_new {
            // Mark this as the selected client on its monitor so scroller
            // centers the new one.
            if target_mon < self.monitors.len() {
                self.monitors[target_mon].prev_selected =
                    self.monitors[target_mon].selected;
                self.monitors[target_mon].selected = Some(new_idx);
            }
            self.focus_surface(Some(FocusTarget::Window(window)));
        }

        if !self.monitors.is_empty() {
            self.arrange_monitor(target_mon);
        }

        tracing::info!("new toplevel: {app_id} monitor={target_mon} idx={new_idx}");
    }
    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let _ = self.popups.track_popup(smithay::desktop::PopupKind::Xdg(surface));
    }
    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }
    fn move_request(
        &mut self,
        surface: ToplevelSurface,
        seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        serial: Serial,
    ) {
        let Some(seat) = Seat::<MargoState>::from_resource(&seat) else { return };
        let Some(pointer) = seat.get_pointer() else { return };
        if !pointer.has_grab(serial) {
            return;
        }
        let Some(start_data) = pointer.grab_start_data() else { return };

        // Resolve the toplevel back to our MargoClient + Window so the grab
        // can manipulate float_geom directly.
        let wl_surf = surface.wl_surface().clone();
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&wl_surf))
        else {
            return;
        };
        let window = self.clients[idx].window.clone();
        let initial_loc =
            smithay::utils::Point::<i32, smithay::utils::Logical>::from((
                self.clients[idx].geom.x,
                self.clients[idx].geom.y,
            ));

        let grab = crate::input::grabs::MoveSurfaceGrab {
            start_data,
            window,
            initial_loc,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        serial: Serial,
        edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        let Some(seat) = Seat::<MargoState>::from_resource(&seat) else { return };
        let Some(pointer) = seat.get_pointer() else { return };
        if !pointer.has_grab(serial) {
            return;
        }
        let Some(start_data) = pointer.grab_start_data() else { return };

        let wl_surf = surface.wl_surface().clone();
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&wl_surf))
        else {
            return;
        };
        let c = &self.clients[idx];
        let window = c.window.clone();
        let initial_loc = smithay::utils::Point::<i32, smithay::utils::Logical>::from((
            c.geom.x, c.geom.y,
        ));
        let initial_size = smithay::utils::Size::<i32, smithay::utils::Logical>::from((
            c.geom.width.max(1),
            c.geom.height.max(1),
        ));

        let grab = crate::input::grabs::ResizeSurfaceGrab {
            start_data,
            window,
            edges,
            initial_loc,
            initial_size,
        };
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
    }

    fn grab(
        &mut self,
        _surface: PopupSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: Serial,
    ) {
    }
    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let wl_surf = surface.wl_surface().clone();
        if let Some(idx) = self.clients.iter().position(|c| {
            c.window.wl_surface().as_deref() == Some(&wl_surf)
        }) {
            if let Some(handle) = self.clients[idx].foreign_toplevel_handle.take() {
                handle.send_closed();
            }
            let window = self.clients[idx].window.clone();
            self.space.unmap_elem(&window);
            self.clients.remove(idx);
            self.shift_indices_after_remove(idx);
            // Re-focus, preferring the previous focus (niri-style focus stack
            // recall), falling back to the spatially nearest visible window.
            let mon_idx = self.focused_monitor();
            if mon_idx < self.monitors.len() {
                let tagset = self.monitors[mon_idx].current_tagset();
                let prev = self.monitors[mon_idx].prev_selected;
                let target = prev
                    .filter(|&i| {
                        i < self.clients.len() && self.clients[i].is_visible_on(mon_idx, tagset)
                    })
                    // Spatial fallback: window whose geom is closest (in vec
                    // order) to the removed slot. The slot was at `idx`; pick
                    // a remaining visible client at or before `idx`, else the
                    // first visible.
                    .or_else(|| {
                        (0..self.clients.len())
                            .rev()
                            .filter(|&i| {
                                i < idx && self.clients[i].is_visible_on(mon_idx, tagset)
                            })
                            .next()
                    })
                    .or_else(|| {
                        self.clients
                            .iter()
                            .position(|c| c.is_visible_on(mon_idx, tagset))
                    });
                match target {
                    Some(i) => {
                        let w = self.clients[i].window.clone();
                        self.monitors[mon_idx].selected = Some(i);
                        self.focus_surface(Some(FocusTarget::Window(w)));
                    }
                    None => {
                        self.monitors[mon_idx].selected = None;
                        self.focus_surface(None);
                    }
                }
            }
            // Re-arrange so the scroller centers the new focus immediately.
            if mon_idx < self.monitors.len() {
                self.arrange_monitor(mon_idx);
            }
        }
        tracing::info!("toplevel destroyed");
    }
}
delegate_xdg_shell!(MargoState);

// ── Smithay delegate: XDG decoration ─────────────────────────────────────────

impl XdgDecorationHandler for MargoState {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(XdgDecorationMode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: XdgDecorationMode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(XdgDecorationMode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(XdgDecorationMode::ServerSide);
        });
        toplevel.send_configure();
    }
}
delegate_xdg_decoration!(MargoState);

// ── Smithay delegate: SHM ────────────────────────────────────────────────────

impl ShmHandler for MargoState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
delegate_shm!(MargoState);

// ── Smithay delegate: Seat ────────────────────────────────────────────────────

impl SeatHandler for MargoState {
    type KeyboardFocus = FocusTarget;
    /// Pointer focus is a raw `WlSurface` — that lets us route events to
    /// the actual subsurface (popups, GTK file picker child surfaces, etc.)
    /// instead of always to the toplevel. Without this, pointer events on
    /// menus / file lists land on the parent surface and the client
    /// translates them as if the parent were under the cursor — exactly the
    /// "imleç başka yerde, seçim başka yerde" symptom.
    type PointerFocus = WlSurface;
    type TouchFocus = FocusTarget;

    fn seat_state(&mut self) -> &mut SeatState<MargoState> {
        &mut self.seat_state
    }
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&FocusTarget>) {
        // Route clipboard / primary-selection events to the newly focused
        // client. Without this, copy-paste between apps and clipboard
        // managers (CopyQ, cliphist, clipse) silently fail.
        let dh = &self.display_handle;
        let client = focused
            .and_then(|target| target.wl_surface())
            .and_then(|surface| dh.get_client(surface.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
        self.request_repaint();
    }
}
delegate_seat!(MargoState);

// ── Smithay delegate: Output ──────────────────────────────────────────────────

impl OutputHandler for MargoState {}
delegate_output!(MargoState);

// ── Smithay delegate: Layer Shell ─────────────────────────────────────────────

impl WlrLayerShellHandler for MargoState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }
    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let smithay_output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| {
                self.monitors
                    .get(self.focused_monitor())
                    .map(|mon| mon.output.clone())
            })
            .or_else(|| self.space.outputs().next().cloned());

        let Some(smithay_output) = smithay_output else { return };

        let desktop_layer = DesktopLayerSurface::new(surface, namespace.clone());
        {
            let mut map = layer_map_for_output(&smithay_output);
            map.map_layer(&desktop_layer).unwrap();
            map.arrange();
        }
        self.refresh_output_work_area(&smithay_output);
        tracing::info!(
            "new layer surface: namespace={namespace} output={}",
            smithay_output.name()
        );
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        // Find the monitor index that owns this layer surface
        let mut found_mon: Option<usize> = None;
        for i in 0..self.monitors.len() {
            let output = self.monitors[i].output.clone();
            let found = {
                let map = layer_map_for_output(&output);
                let mut found_layer = false;
                for l in map.layers() {
                    if l.layer_surface() == &surface {
                        found_layer = true;
                        break;
                    }
                }
                found_layer
            };
            if found {
                found_mon = Some(i);
                break;
            }
        }

        let Some(mon_idx) = found_mon else {
            tracing::info!("layer surface destroyed (not found)");
            return;
        };

        let output = self.monitors[mon_idx].output.clone();

        // Collect layer to remove
        let layer = {
            let map = layer_map_for_output(&output);
            let mut result = None;
            for l in map.layers() {
                if l.layer_surface() == &surface {
                    result = Some(l.clone());
                    break;
                }
            }
            result
        };

        if let Some(layer) = layer {
            let mut map = layer_map_for_output(&output);
            map.unmap_layer(&layer);
            map.arrange();
        }

        self.refresh_output_work_area(&output);
        tracing::info!("layer surface destroyed");
    }
}
delegate_layer_shell!(MargoState);

// ── Smithay delegate: Data Device ─────────────────────────────────────────────

impl SelectionHandler for MargoState {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(err) = xwm.new_selection(ty, source.map(|source| source.mime_types())) {
                tracing::warn!(?err, ?ty, "failed to mirror Wayland selection to XWayland");
            }
        }
    }

    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &(),
    ) {
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(err) = xwm.send_selection(ty, mime_type, fd) {
                tracing::warn!(?err, ?ty, "failed to send Wayland selection to XWayland");
            }
        }
    }
}
impl DataDeviceHandler for MargoState {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}
impl WaylandDndGrabHandler for MargoState {}
delegate_data_device!(MargoState);

impl PrimarySelectionHandler for MargoState {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}
delegate_primary_selection!(MargoState);

impl DataControlHandler for MargoState {
    fn data_control_state(&mut self) -> &mut DataControlState {
        &mut self.data_control_state
    }
}
delegate_data_control!(MargoState);

impl crate::protocols::gamma_control::GammaControlHandler for MargoState {
    fn gamma_control_manager_state(
        &mut self,
    ) -> &mut crate::protocols::gamma_control::GammaControlManagerState {
        &mut self.gamma_control_manager_state
    }

    fn get_gamma_size(&mut self, output: &Output) -> Option<u32> {
        self.monitors
            .iter()
            .find(|m| &m.output == output)
            .map(|m| m.gamma_size)
            .filter(|&s| s > 0)
    }

    fn set_gamma(&mut self, output: &Output, ramp: Option<Vec<u16>>) -> Option<()> {
        // Coalesce: if a pending entry already exists for this output, replace
        // it. Avoids unbounded queue growth if a client spams set_gamma faster
        // than the backend drains.
        if let Some(existing) = self
            .pending_gamma
            .iter_mut()
            .find(|(o, _)| o == output)
        {
            existing.1 = ramp;
        } else {
            self.pending_gamma.push((output.clone(), ramp));
        }
        self.request_repaint();
        Some(())
    }
}
crate::delegate_gamma_control!(MargoState);

impl crate::protocols::screencopy::ScreencopyHandler for MargoState {
    fn screencopy_state(
        &mut self,
    ) -> &mut crate::protocols::screencopy::ScreencopyManagerState {
        &mut self.screencopy_state
    }

    fn frame(
        &mut self,
        manager: &smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        screencopy: crate::protocols::screencopy::Screencopy,
    ) {
        // Defer the actual buffer copy to the backend's render path —
        // the queue holds the screencopy until the next frame is rendered
        // for that output.
        self.screencopy_state.push(manager, screencopy);
        self.request_repaint();
    }
}
crate::delegate_screencopy!(MargoState);

// ── Smithay delegate: DnD grab (required by X11Wm::start_wm) ─────────────────

impl DndGrabHandler for MargoState {}

// ── Smithay delegate: XWayland shell ─────────────────────────────────────────

impl XWaylandShellHandler for MargoState {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}
smithay::delegate_xwayland_shell!(MargoState);

// ── ForeignToplevelListHandler ────────────────────────────────────────────────

impl ForeignToplevelListHandler for MargoState {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_list
    }
}

smithay::delegate_foreign_toplevel_list!(MargoState);

// ── XwmHandler: X11 window management ────────────────────────────────────────

impl MargoState {
    fn find_x11_client(&self, window: &X11Surface) -> Option<usize> {
        let id = window.window_id();
        self.clients.iter().position(|c| {
            matches!(c.window.underlying_surface(), WindowSurface::X11(s) if s.window_id() == id)
        })
    }

    fn register_x11_window(&mut self, x11surface: X11Surface) {
        let window = Window::new_x11_window(x11surface);
        let mon_idx = self.focused_monitor();
        let tags = self.monitors.get(mon_idx).map(|m| m.current_tagset()).unwrap_or(1);
        let mut client = MargoClient::new(window.clone(), mon_idx, tags, &self.config);
        client.surface_type = crate::SurfaceType::X11;
        client.title = window.x11_surface().map(|s| s.title()).unwrap_or_default();
        client.app_id = window.x11_surface().map(|s| s.class()).unwrap_or_default();
        self.apply_window_rules(&mut client);
        let target_mon = client.monitor;
        let focus_new = !client.no_focus && !client.open_silent;
        let ft_handle = self.foreign_toplevel_list.new_toplevel::<Self>(&client.title, &client.app_id);
        ft_handle.send_done();
        client.foreign_toplevel_handle = Some(ft_handle);
        self.clients.push(client);
        let map_loc = self
            .monitors
            .get(target_mon)
            .map(|m| (m.monitor_area.x, m.monitor_area.y))
            .unwrap_or((0, 0));
        self.space.map_element(window.clone(), map_loc, true);
        if focus_new {
            self.focus_surface(Some(FocusTarget::Window(window)));
        }
        if !self.monitors.is_empty() {
            self.arrange_monitor(target_mon);
        }
        tracing::info!("new x11 toplevel: {} monitor={target_mon}", self.clients.last().map(|c| c.app_id.as_str()).unwrap_or(""));
    }

    fn remove_x11_window(&mut self, x11surface: &X11Surface) {
        if let Some(idx) = self.find_x11_client(x11surface) {
            if let Some(handle) = self.clients[idx].foreign_toplevel_handle.take() {
                handle.send_closed();
            }
            let window = self.clients[idx].window.clone();
            self.space.unmap_elem(&window);
            self.clients.remove(idx);
            self.shift_indices_after_remove(idx);
            let mon_idx = self.focused_monitor();
            if !self.monitors.is_empty() {
                self.arrange_monitor(mon_idx);
            }
        }
    }
}

impl XwmHandler for MargoState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("X11Wm not initialized")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        window.set_mapped(true).ok();
        self.register_x11_window(window);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let win = Window::new_x11_window(window);
        let pos = win.x11_surface()
            .map(|s| { let g = s.geometry(); (g.loc.x, g.loc.y) })
            .unwrap_or((0, 0));
        self.space.map_element(win, pos, false);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let geom = window.geometry();
        let new_geom = Rectangle::new(
            (x.unwrap_or(geom.loc.x), y.unwrap_or(geom.loc.y)).into(),
            (w.map(|v| v as i32).unwrap_or(geom.size.w), h.map(|v| v as i32).unwrap_or(geom.size.h)).into(),
        );
        window.configure(new_geom).ok();
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        if let Some(idx) = self.find_x11_client(&window) {
            self.clients[idx].geom = crate::layout::Rect {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            };
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _resize_edge: ResizeEdge,
    ) {}

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        let Some(FocusTarget::Window(window)) = keyboard.current_focus() else {
            return false;
        };
        window
            .x11_surface()
            .and_then(|surface| surface.xwm_id())
            .map(|focused_xwm| focused_xwm == xwm)
            .unwrap_or(false)
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        match selection {
            SelectionTarget::Clipboard => {
                if let Err(err) = request_data_device_client_selection(&self.seat, mime_type, fd) {
                    tracing::error!(?err, "failed to request Wayland clipboard for XWayland");
                }
            }
            SelectionTarget::Primary => {
                if let Err(err) = request_primary_client_selection(&self.seat, mime_type, fd) {
                    tracing::error!(?err, "failed to request Wayland primary selection for XWayland");
                }
            }
        }
    }

    fn new_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_types: Vec<String>,
    ) {
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mime_types, ())
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types, ())
            }
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat);
                }
            }
        }
    }
}

// ── Smithay delegate: Viewporter ───────────────────────────────────────────────

smithay::delegate_viewporter!(MargoState);

// ── Smithay delegate: Session Lock ───────────────────────────────────────────

impl SessionLockHandler for MargoState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        tracing::info!("session locked");
        confirmation.lock();
        self.session_locked = true;
        self.arrange_all();
    }

    fn unlock(&mut self) {
        tracing::info!("session unlocked");
        self.session_locked = false;
        self.lock_surfaces.clear();
        self.arrange_all();
    }

    fn new_surface(&mut self, surface: LockSurface, output: smithay::reexports::wayland_server::protocol::wl_output::WlOutput) {
        let Some(output) = Output::from_resource(&output) else {
            tracing::warn!("session_lock: new_surface for unknown output");
            return;
        };

        // CRITICAL: ext-session-lock-v1 requires the compositor to send a
        // configure WITH a non-zero size before the client will attach a
        // buffer. Without this, the lock surface stays unmapped and we
        // render solid black with just the cursor on top — exactly the
        // "alt+l → black screen" symptom.
        let size = output
            .current_mode()
            .map(|m| {
                // Apply the output's transform so portrait outputs get the
                // logical (post-transform) size.
                let transform = output.current_transform();
                let physical = transform.transform_size(m.size);
                let scale = output.current_scale().fractional_scale();
                Size::<u32, smithay::utils::Logical>::from((
                    (physical.w as f64 / scale).round().max(1.0) as u32,
                    (physical.h as f64 / scale).round().max(1.0) as u32,
                ))
            })
            .unwrap_or_else(|| Size::<u32, smithay::utils::Logical>::from((1280, 720)));

        surface.with_pending_state(|state| {
            state.size = Some(size);
        });
        surface.send_configure();

        tracing::info!(
            "session_lock: new lock surface on {} size {}x{}",
            output.name(),
            size.w,
            size.h
        );

        self.lock_surfaces.push((output, surface.clone()));
        self.focus_surface(Some(FocusTarget::SessionLock(surface)));
        self.request_repaint();
    }
}
delegate_session_lock!(MargoState);

// ── Smithay delegate: Idle notify + Idle inhibit ─────────────────────────────

impl smithay::wayland::idle_notify::IdleNotifierHandler for MargoState {
    fn idle_notifier_state(
        &mut self,
    ) -> &mut smithay::wayland::idle_notify::IdleNotifierState<Self> {
        &mut self.idle_notifier_state
    }
}
delegate_idle_notify!(MargoState);

impl smithay::wayland::idle_inhibit::IdleInhibitHandler for MargoState {
    fn inhibit(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        self.idle_inhibitors.insert(surface);
        // Pause idle timers as long as anything is inhibiting.
        let inhibited = !self.idle_inhibitors.is_empty();
        self.idle_notifier_state.set_is_inhibited(inhibited);
        tracing::debug!("idle_inhibit: active={} count={}", inhibited, self.idle_inhibitors.len());
    }

    fn uninhibit(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        self.idle_inhibitors.remove(&surface);
        let inhibited = !self.idle_inhibitors.is_empty();
        self.idle_notifier_state.set_is_inhibited(inhibited);
        tracing::debug!("idle_uninhibit: active={} count={}", inhibited, self.idle_inhibitors.len());
    }
}
delegate_idle_inhibit!(MargoState);
