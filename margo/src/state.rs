#![allow(dead_code)]
use std::{cell::RefCell, os::unix::io::OwnedFd, path::PathBuf, rc::Rc, sync::Arc};

use anyhow::{Context, Result};
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        renderer::utils::on_commit_buffer_handler,
    },
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_dmabuf,
    delegate_input_method_manager, delegate_layer_shell, delegate_output,
    delegate_pointer_constraints, delegate_primary_selection, delegate_relative_pointer,
    delegate_seat, delegate_shm, delegate_text_input_manager,
    delegate_presentation, delegate_xdg_activation, delegate_xdg_decoration, delegate_xdg_shell,
    delegate_session_lock, delegate_idle_notify, delegate_idle_inhibit,
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
        calloop::{ping::Ping, LoopHandle, LoopSignal},
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
        input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface as InputMethodPopupSurface},
        pointer_constraints::{
            with_pointer_constraint, PointerConstraintsHandler, PointerConstraintsState,
        },
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        text_input::TextInputManagerState,
        xdg_activation::{
            XdgActivationHandler, XdgActivationState, XdgActivationToken,
            XdgActivationTokenData,
        },
        viewporter::ViewporterState,
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        drm_syncobj::{DrmSyncobjHandler, DrmSyncobjState},
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

// One-line tag for focus targets; only used in tracing so we don't have to
// pull `Debug` through whatever wrapped surface a target carries.
fn focus_target_label(t: &FocusTarget) -> String {
    match t {
        FocusTarget::Window(w) => format!("Window({:?})", w.wl_surface().map(|s| s.id())),
        FocusTarget::LayerSurface(_) => "LayerSurface".to_string(),
        FocusTarget::SessionLock(s) => format!("SessionLock({:?})", s.wl_surface().id()),
    }
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

/// Captured snapshot of a window's rendered content, used to keep the
/// pre-resize visuals on screen while the client (typically Electron:
/// Helium, Spotify, Discord) takes 50–100 ms to ack a configure and
/// commit a buffer at the new size. The snapshot is rendered scaled
/// to the (interpolated) layout slot during the move animation,
/// instead of the live surface — that's the niri-style resize
/// transition that hides Helium's 50 ms reflow flicker.
pub struct ResizeSnapshot {
    /// The captured window contents, allocated as an offscreen
    /// `GlesTexture` by `crate::render::window_capture::capture_window`.
    pub texture: smithay::backend::renderer::gles::GlesTexture,
    /// Logical size of the window at capture time. Used by the render
    /// path to decide if the snapshot is still relevant or if the live
    /// buffer has caught up enough to take over.
    pub source_size: smithay::utils::Size<i32, smithay::utils::Logical>,
    /// Wall-clock instant at which the snapshot was created. Combined
    /// with the move animation duration, the render path knows when
    /// to stop using this texture and switch back to the live
    /// surface.
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
    /// Niri-style resize animation snapshot. Set when the layout slot
    /// size changes; the next render captures the current surface
    /// tree to a `GlesTexture` and stores it here. Subsequent renders
    /// draw this texture scaled to the (interpolated) slot until the
    /// animation expires or the client commits a fresh buffer at the
    /// new size, at which point we clear it and go back to drawing
    /// the live surface.
    pub resize_snapshot: Option<ResizeSnapshot>,
    /// One-shot flag set by `arrange_monitor` whenever the slot size
    /// crosses a meaningful threshold. Drained by the udev backend at
    /// the next frame, which uses it to populate
    /// `resize_snapshot`. The two-step dance is necessary because
    /// `arrange_monitor` runs in many event paths that don't have the
    /// renderer in scope, so the actual GPU work has to be deferred
    /// to the render thread.
    pub snapshot_pending: bool,
    /// True while the client is between `new_toplevel` and its first
    /// post-app_id commit. We deliberately don't map the window into
    /// the smithay space or run window rules during this window
    /// because Qt clients (CopyQ, KeePassXC, …) routinely create
    /// the xdg_toplevel role *before* sending `set_app_id`, so a
    /// rule keyed on `appid:^copyq$` wouldn't match yet and the
    /// window would briefly appear at the layout's default position
    /// before snapping to its rule-driven floating geometry — the
    /// "super+v copyq açtığımda pencere bir kaybolup tekrar geliyor"
    /// flicker. Cleared on the first commit that satisfies our
    /// "ready to map" criteria (app_id is set OR we've waited long
    /// enough); at that point we apply rules, place the window, and
    /// hand it focus.
    pub is_initial_map_pending: bool,
    /// Open transition state. Set in `finalize_initial_map` if open
    /// animations are enabled; cleared when the curve settles. While
    /// `Some`, the renderer captures a `GlesTexture` snapshot on the
    /// first frame after the client commits a buffer, then renders
    /// that texture scaled + faded around the slot's centre. The live
    /// `wl_surface` is hidden during the transition so the user never
    /// sees the unanimated "instant pop" frame underneath.
    pub opening_animation: Option<crate::animation::OpenCloseClientAnim>,
    /// Captured surface texture for the open animation. Populated on
    /// the first render after `opening_animation` becomes `Some` and
    /// the surface has a usable buffer; dropped along with
    /// `opening_animation` when the curve settles.
    pub opening_texture: Option<smithay::backend::renderer::gles::GlesTexture>,
    /// Set when `opening_animation` is created; cleared by the renderer
    /// once it actually captures a texture into `opening_texture`. The
    /// two-step dance mirrors `snapshot_pending` — `finalize_initial_map`
    /// runs from a commit handler that doesn't have a `GlesRenderer`
    /// in scope, so the GPU work has to defer to the next render.
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

/// Per-call parameters for [`tick_animations`]. Bundles the move-animation
/// duration (used for both bezier ticks and resize-snapshot expiry) with
/// the spring physics configuration, so the call site doesn't have to
/// thread four scalars individually.
#[derive(Debug, Clone, Copy)]
pub struct AnimTickSpec {
    /// Total bezier duration in `now_ms` units. Also bounds resize
    /// snapshot life-time regardless of which clock is in use.
    pub duration_move: u32,
    /// `true` → spring physics integrator drives the move animation;
    /// `false` → original bezier sampling.
    pub use_spring: bool,
    /// Pre-built spring (stiffness/damping/mass already resolved from
    /// the damping ratio). Ignored when `use_spring` is false.
    pub spring: crate::animation::spring::Spring,
}

pub fn tick_animations(
    clients: &mut [MargoClient],
    curves: &AnimationCurves,
    now_ms: u32,
    spec: AnimTickSpec,
    closing_clients: &mut Vec<ClosingClient>,
    layer_animations: &mut std::collections::HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        LayerSurfaceAnim,
    >,
) -> bool {
    let mut changed = false;
    // Advance focus highlight (border colour + opacity) crossfades.
    // `OpacityAnimation` does double duty: focused_opacity ↔ unfocused_opacity
    // for the alpha, focuscolor ↔ bordercolor for the border. Both
    // sample the `Focus` curve. Border refresh reads the current
    // colour from this struct on every refresh so the cross-fade
    // shows even between renders.
    for c in clients.iter_mut() {
        let oa = &mut c.opacity_animation;
        if !oa.running {
            continue;
        }
        let elapsed = now_ms.wrapping_sub(oa.time_started);
        if elapsed >= oa.duration {
            oa.running = false;
            oa.current_opacity = oa.target_opacity;
            oa.current_border_color = oa.target_border_color;
            changed = true;
            continue;
        }
        let t = elapsed as f64 / oa.duration as f64;
        let s = curves.sample(t, AnimationType::Focus) as f32;
        oa.current_opacity = oa.initial_opacity + (oa.target_opacity - oa.initial_opacity) * s;
        for i in 0..4 {
            let a = oa.initial_border_color[i];
            let b = oa.target_border_color[i];
            oa.current_border_color[i] = a + (b - a) * s;
        }
        changed = true;
    }

    // Advance opening animations on each client. Settles drop both
    // the animation state and the captured texture so the live
    // wl_surface takes over on the next frame.
    for c in clients.iter_mut() {
        if let Some(anim) = c.opening_animation.as_mut() {
            let elapsed = now_ms.wrapping_sub(anim.time_started);
            if elapsed >= anim.duration {
                c.opening_animation = None;
                c.opening_texture = None;
                c.opening_capture_pending = false;
                changed = true;
            } else {
                let raw = elapsed as f64 / anim.duration as f64;
                anim.progress = curves.sample(raw, AnimationType::Open) as f32;
                changed = true;
            }
        }
    }

    // Advance layer-surface open/close animations. Settled entries
    // are removed from the map; the open path then falls back to
    // unmodulated layer rendering, the close path stops drawing the
    // texture (the underlying smithay layer was already unmapped at
    // `layer_destroyed` time).
    {
        let mut to_drop: Vec<smithay::reexports::wayland_server::backend::ObjectId> = Vec::new();
        for (id, anim) in layer_animations.iter_mut() {
            let elapsed = now_ms.wrapping_sub(anim.time_started);
            if elapsed >= anim.duration {
                to_drop.push(id.clone());
                continue;
            }
            let raw = elapsed as f64 / anim.duration as f64;
            let action = if anim.is_close {
                AnimationType::Close
            } else {
                AnimationType::Open
            };
            anim.progress = curves.sample(raw, action) as f32;
            changed = true;
        }
        for id in to_drop {
            layer_animations.remove(&id);
            changed = true;
        }
    }

    // Advance close animations and pop entries that have settled.
    // Iterate in reverse so we can `swap_remove` cleanly without
    // resampling indices. (Order doesn't matter visually — closing
    // clients don't interact with each other beyond stacking, which
    // we don't preserve in this list anyway.)
    let mut i = 0;
    while i < closing_clients.len() {
        let cc = &mut closing_clients[i];
        let elapsed = now_ms.wrapping_sub(cc.time_started);
        if elapsed >= cc.duration {
            closing_clients.swap_remove(i);
            changed = true;
            continue;
        }
        let raw = elapsed as f64 / cc.duration as f64;
        cc.progress = curves.sample(raw, AnimationType::Close) as f32;
        changed = true;
        i += 1;
    }

    for c in clients.iter_mut() {
        // Resize-snapshot expiry. Bezier mode caps the snapshot's life
        // at `duration_move` (its crossfade alpha is fully transparent
        // by then anyway). Spring mode has no fixed duration — the
        // snapshot is dropped when the spring settles, inside the
        // settle branch below — so we only run the wall-clock cap on
        // bezier here. Otherwise a long spring overshoot would lose
        // its snapshot mid-flight and the live surface (still at the
        // pre-resize size) would suddenly pop into view.
        if !spec.use_spring {
            if let Some(snapshot) = c.resize_snapshot.as_ref() {
                let dur = std::time::Duration::from_millis(spec.duration_move as u64);
                if snapshot.captured_at.elapsed() >= dur {
                    c.resize_snapshot = None;
                    changed = true;
                }
            }
        }

        let anim = &mut c.animation;
        if !anim.running { continue; }
        changed = true;

        if spec.use_spring {
            // Spring path — niri-style analytical solution.
            //
            // The animation already has a precomputed `duration` from
            // arrange_monitor (`Spring::clamped_duration`). We sample
            // the closed-form oscillator at `elapsed` and lerp from
            // initial → current using its [0, 1] progress. This
            // guarantees the animation ends at exactly `duration` ms;
            // the previous numerical integrator could leave the
            // running flag set indefinitely when c.geom rounded onto
            // its target while velocity was still above the velocity-
            // epsilon, producing a CPU-bound tick→render→tick loop.
            let elapsed_ms = now_ms.wrapping_sub(anim.time_started);
            if elapsed_ms >= anim.duration {
                // Hard end. Snap to the exact target — `value_at` may
                // miss it by a fraction of a pixel, and we don't want
                // the difference surviving into the next frame.
                anim.running = false;
                c.geom = anim.current;
                c.resize_snapshot = None;
                continue;
            }
            // 1D progress spring goes 0 → 1 over `duration`. Apply that
            // single progress to all four channels so x/y/w/h move
            // together — for window movement that's exactly what we
            // want (the user perceives a single object travelling, not
            // four independent ones).
            let progress_spring = crate::animation::spring::Spring {
                from: 0.0,
                to: 1.0,
                initial_velocity: 0.0,
                params: crate::animation::spring::SpringParams {
                    damping: spec.spring.params.damping,
                    mass: spec.spring.params.mass,
                    stiffness: spec.spring.params.stiffness,
                    epsilon: spec.spring.params.epsilon,
                },
            };
            let t = std::time::Duration::from_millis(elapsed_ms as u64);
            let p = progress_spring.value_at(t).clamp(0.0, 1.0);
            c.geom.x = lerp_i32(anim.initial.x, anim.current.x, p);
            c.geom.y = lerp_i32(anim.initial.y, anim.current.y, p);
            c.geom.width = lerp_i32(anim.initial.width, anim.current.width, p);
            c.geom.height = lerp_i32(anim.initial.height, anim.current.height, p);
        } else {
            // Bezier path (original behaviour).
            let elapsed = now_ms.wrapping_sub(anim.time_started);
            if elapsed >= anim.duration {
                anim.running = false;
                c.geom = anim.current;
                // Slot animation settled: also drop any lingering
                // snapshot (defensive — the expiry check above usually
                // catches it first).
                c.resize_snapshot = None;
                continue;
            }
            let t = elapsed as f64 / anim.duration as f64;
            let s = curves.sample(t, anim.action);
            c.geom.x = lerp_i32(anim.initial.x, anim.current.x, s);
            c.geom.y = lerp_i32(anim.initial.y, anim.current.y, s);
            c.geom.width = lerp_i32(anim.initial.width, anim.current.width, s);
            c.geom.height = lerp_i32(anim.initial.height, anim.current.height, s);
        }
    }
    changed
}

#[inline]
fn lerp_i32(a: i32, b: i32, t: f64) -> i32 {
    (a as f64 + (b - a) as f64 * t) as i32
}

// ── Top-level compositor state ────────────────────────────────────────────────

pub type DmabufImportHook = Rc<RefCell<dyn FnMut(&Dmabuf) -> bool>>;

/// A window in the middle of its close animation. Lives in
/// [`MargoState::closing_clients`] from `toplevel_destroyed` (or X11
/// `destroyed_window`) until `tick_animations` decides the curve is
/// done. The captured `texture` is what the render path draws — we
/// can't render the live `wl_surface` because it's already gone.
#[derive(Debug)]
pub struct ClosingClient {
    /// Stable scene-graph ID, derived from the original window so
    /// smithay's damage tracker keeps tracking the slot consistently
    /// across frames of the close animation.
    pub id: smithay::backend::renderer::element::Id,
    /// Captured surface tree as a single texture. `None` while the
    /// render path hasn't run yet; the renderer fills it in on the
    /// first frame after destruction. If the capture fails (surface
    /// was already torn down), the entry is dropped without
    /// rendering — better to skip the animation than crash.
    pub texture: Option<smithay::backend::renderer::gles::GlesTexture>,
    /// True until [`texture`] is populated. The renderer drains all
    /// pending captures before building elements each frame.
    pub capture_pending: bool,
    /// Logical-pixel rect the window occupied when the close started.
    /// Used as the "stable" target for the OpenCloseRenderElement —
    /// the scale/alpha curves animate around this rect's centre.
    pub geom: Rect,
    /// Monitor the window was on. Used by the renderer to decide
    /// which output draws this entry (multi-monitor setups).
    pub monitor: usize,
    /// Tag bitmask the window was visible on. The render path skips
    /// this entry on monitors whose current tagset doesn't intersect.
    pub tags: u32,
    /// Animation start time in `now_ms` units.
    pub time_started: u32,
    /// Total animation duration in milliseconds.
    pub duration: u32,
    /// 0..=1 progress through the close curve. 0 = just started
    /// closing (still fully visible), 1 = fully gone. The render
    /// element flips its alpha/scale curve on `is_close = true`.
    pub progress: f32,
    /// Animation flavour (Zoom / Fade / Slide).
    pub kind: crate::render::open_close::OpenCloseKind,
    /// Final scale at progress = 1. Pulled from
    /// [`margo_config::Config::zoom_end_ratio`] when the animation
    /// fires; baked here so config changes mid-flight don't snap.
    pub extreme_scale: f32,
    /// Corner radius (logical px) so the closing snapshot still has
    /// rounded corners during fade-out. 0 = no clipping.
    pub border_radius: f32,
    /// We need to fetch the wl_surface buffer once to capture; after
    /// that this surface reference is dropped. Held only while
    /// `capture_pending == true`.
    pub source_surface: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
}

/// Layer surface in mid-open or mid-close transition. Mirrors
/// [`ClosingClient`] but stripped down — layer surfaces don't have
/// the same lifecycle complexity as toplevels (no per-tag visibility,
/// no monitor migration, smithay's `LayerMap` owns them).
#[derive(Debug)]
pub struct LayerSurfaceAnim {
    pub time_started: u32,
    pub duration: u32,
    pub progress: f32,
    /// `true` for close transition (texture-driven slide-out),
    /// `false` for open (live surface fade-in).
    pub is_close: bool,
    /// Snapshot for the close path. `None` for open and while a close
    /// capture is pending; populated by the renderer the first frame
    /// after the layer is destroyed.
    pub texture: Option<smithay::backend::renderer::gles::GlesTexture>,
    pub capture_pending: bool,
    /// Where the layer was when its close animation kicked off.
    /// Render path needs this because by close time the smithay
    /// `LayerMap` no longer knows the layer's geometry.
    pub geom: Rect,
    /// Slide direction derived from the layer's anchor. Layers
    /// anchored to the top edge slide up on close (and slide in
    /// from above on open); right-anchored slide right; etc. Pure
    /// fade for layers with no useful anchor (centred dialogs).
    pub kind: crate::render::open_close::OpenCloseKind,
    /// Held only while `capture_pending` so the close-side capture
    /// has a surface to read from.
    pub source_surface: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
}

pub struct MargoState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub viewporter_state: ViewporterState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub dmabuf_import_hook: Option<DmabufImportHook>,
    /// `wp_linux_drm_syncobj_v1` global state. `None` until the udev
    /// backend opens the primary DRM node and confirms it supports
    /// `syncobj_eventfd` — older kernels (< 5.18) and devices without
    /// `DRM_CAP_SYNCOBJ_TIMELINE` can't drive explicit-sync, so we
    /// don't expose the protocol there. Modern Chromium / Firefox
    /// prefers explicit sync when the global is advertised: per-
    /// surface `wp_linux_drm_syncobj_surface_v1` carries acquire +
    /// release fences alongside the dmabuf, eliminating the implicit
    /// fence wait that otherwise drops frames under GPU load.
    pub drm_syncobj_state: Option<DrmSyncobjState>,
    pub seat_state: SeatState<MargoState>,
    pub layer_shell_state: WlrLayerShellState,
    pub output_manager_state: OutputManagerState,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub session_lock_state: smithay::wayland::session_lock::SessionLockManagerState,
    /// `wp_text_input_v3` global. Qt clients (Quickshell/noctalia, KDE,
    /// QtWidgets apps) probe for this when a TextInput field becomes
    /// active — without it, Qt's QML password fields silently drop
    /// keystrokes on the lock screen even when wl_keyboard.enter is
    /// delivered. We don't drive an IME ourselves; smithay routes the
    /// protocol traffic correctly with just the global registered.
    pub text_input_state: TextInputManagerState,
    /// `zwp_input_method_v2` global. Goes hand-in-hand with text_input —
    /// Qt's text-input plugin won't activate without both.
    pub input_method_state: InputMethodManagerState,
    /// `zwp_pointer_constraints_v1` global. Lets clients request that
    /// the pointer be locked (held in place, FPS games / Blender's
    /// rotate-around-camera) or confined to a region (Krita canvas
    /// drag, remote-desktop client). Activated through
    /// `PointerConstraintsHandler::new_constraint`; honoured in
    /// `handle_pointer_motion`.
    pub pointer_constraints_state: PointerConstraintsState,
    /// `zwp_relative_pointer_manager_v1` global. Required complement
    /// to pointer constraints — when the pointer is locked, clients
    /// still need to know the cursor *would have moved by Δ*. Each
    /// pointer-motion event already calls `pointer.relative_motion`,
    /// so all this state needs to do is exist so clients can bind
    /// the global and get a `wp_relative_pointer_v1` per pointer.
    pub relative_pointer_state: RelativePointerManagerState,
    /// `xdg_activation_v1` global. The polite focus-stealing
    /// channel: launchers (rofi, wofi, xdg-desktop-portal-wlr's
    /// activate request), notification daemons (notify-send action
    /// buttons), and chained-launcher flows (browser handles a
    /// mailto: by activating the running mail client) hand a token
    /// to the target surface; we honour or reject it. We accept
    /// when the request comes with a valid recent keyboard
    /// interaction serial (ie. the user was actively typing on the
    /// requesting client when it generated the token), reject
    /// otherwise — that's the spec-recommended anti-focus-steal
    /// gate.
    pub xdg_activation_state: XdgActivationState,
    /// `wlr_output_management_v1` state. Lets `kanshi`,
    /// `wlr-randr`, `way-displays` etc. discover the output
    /// topology and apply scale / transform / position changes
    /// at runtime. Mode and enable changes are still rejected;
    /// see `protocols::output_management::apply_output_pending`.
    pub output_management_state:
        crate::protocols::output_management::OutputManagementManagerState,
    /// `wp_presentation` global. Lets clients (kitty, mpv, native
    /// Wayland Vulkan games via DXVK / VKD3D, video conferencing
    /// apps that adapt their pacing to the actual display refresh)
    /// register `wp_presentation_feedback` per-frame and learn the
    /// real `presented` timestamp + refresh interval. Without this
    /// they're stuck guessing — kitty falls back to a 60 Hz tick,
    /// mpv ships its own debouncer, vsync-sensitive games stutter.
    pub presentation_state: PresentationState,
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
    /// Set whenever something dirties the scene. Drained by the udev/winit
    /// backend before each render. Source-of-truth for "does anything need
    /// to be redrawn this iteration"; the [`repaint_ping`] is only the
    /// wake-up mechanism, not the state.
    repaint_requested: bool,
    /// On-demand wake source for the redraw scheduler. The udev backend
    /// installs a calloop `Ping` source whose callback runs the render
    /// path; here we keep a sender so [`MargoState::request_repaint`] can
    /// poke it from anywhere (input handlers, commit hooks, animation
    /// ticks, IPC dispatch). Idle = no pings = loop stays asleep instead
    /// of waking 60 Hz from a polling timer.
    repaint_ping: Option<Ping>,
    /// Number of `queue_frame()` calls awaiting their matching
    /// `DrmEvent::VBlank`. Acts as a rate limiter for the redraw
    /// scheduler: while >0, [`request_repaint`] still flags the scene
    /// dirty but does *not* ping — the post-dispatch animation tick
    /// would otherwise re-arm a repaint on every loop iteration and the
    /// ping callback would fire immediately, rendering on the CPU as
    /// fast as the loop can spin (between vblanks). The vblank handler
    /// decrements this and, if zero, re-emits the deferred ping.
    pending_vblanks: u32,
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
    /// Windows that have been requested to close but are still on screen
    /// for the duration of the close animation. Each entry carries a
    /// captured `GlesTexture` of the window's last visible frame plus
    /// the geometry / monitor / tags it was on, so the renderer can
    /// keep painting it after the live `wl_surface` is gone.
    /// `tick_animations` advances each entry's progress and pops it
    /// when the curve settles. Pending captures (the wl_surface was
    /// still alive at destruction time but we hadn't rendered yet)
    /// live as `None` in `texture` until the next render fills them in.
    pub closing_clients: Vec<ClosingClient>,
    /// Layer surfaces in their open / close animation. Keyed by the
    /// layer's wl_surface object id so the render path can look up
    /// the per-layer animation state without an O(n) scan. Each
    /// entry tracks both directions: `is_close` flips at
    /// `layer_destroyed`. After settling, open entries get popped
    /// in `tick_animations` (cleared from the map); close entries
    /// also drop the captured texture along with the entry.
    pub layer_animations: std::collections::HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        LayerSurfaceAnim,
    >,
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
        let text_input_state = TextInputManagerState::new::<Self>(&dh);
        let input_method_state = InputMethodManagerState::new::<Self, _>(&dh, |_client| true);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        let relative_pointer_state = RelativePointerManagerState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let output_management_state =
            crate::protocols::output_management::OutputManagementManagerState::new::<
                Self,
                _,
            >(&dh, |_client| true);
        // Clock id 1 = CLOCK_MONOTONIC. That's the same domain
        // `monotonic_now()` in the udev backend uses, so the
        // timestamps we publish are consistent with the ones
        // clients see in their own `clock_gettime(CLOCK_MONOTONIC)`.
        let presentation_state = PresentationState::new::<Self>(&dh, 1);
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
            drm_syncobj_state: None,
            seat_state,
            layer_shell_state,
            output_manager_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            session_lock_state,
            text_input_state,
            input_method_state,
            pointer_constraints_state,
            relative_pointer_state,
            xdg_activation_state,
            output_management_state,
            presentation_state,
            space,
            popups,
            seat,
            display_handle: dh,
            loop_handle,
            loop_signal,
            clock: Clock::new(),
            should_quit: false,
            repaint_requested: true,
            repaint_ping: None,
            pending_vblanks: 0,
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
            closing_clients: Vec::new(),
            layer_animations: std::collections::HashMap::new(),
            config,
        }
    }

    /// Rebuild the wlr-output-management snapshot from the current
    /// monitor list and publish it to all bound clients (kanshi,
    /// wlr-randr, way-displays, …). Cheap when nothing's changed:
    /// `snapshot_changed` early-returns on equal snapshots.
    pub fn publish_output_topology(&mut self) {
        let mut snap = std::collections::HashMap::new();
        for mon in &self.monitors {
            let pos = (mon.monitor_area.x, mon.monitor_area.y);
            snap.insert(
                mon.name.clone(),
                crate::protocols::output_management::snapshot_from_output(
                    &mon.output,
                    mon.enabled,
                    pos,
                ),
            );
        }
        self.output_management_state.snapshot_changed(snap);
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
    /// Force-tear-down a stuck session lock from the compositor side.
    ///
    /// Use case: the user pressed alt+l, noctalia's lock screen came up,
    /// and now the password input is unresponsive — they can't type to
    /// unlock. Without this, recovery means switching to a TTY and
    /// killing the locker process. With this they can hit a hard-coded
    /// keybind (the action is whitelisted in `handle_keyboard` even
    /// while `session_locked`) and get back to the desktop.
    ///
    /// We don't try to nicely tell the locker to release; we just clear
    /// our state, drop the lock surfaces, and re-show toplevels. The
    /// noctalia process will see its surfaces destroyed and recover on
    /// its own.
    pub fn force_unlock(&mut self) {
        if !self.session_locked && self.lock_surfaces.is_empty() {
            tracing::info!("force_unlock: nothing to do (already unlocked)");
            return;
        }
        tracing::warn!(
            "force_unlock: tearing down stuck lock (lock_surfaces={})",
            self.lock_surfaces.len()
        );
        self.session_locked = false;
        self.lock_surfaces.clear();
        self.arrange_all();
        self.refresh_keyboard_focus();
        let _ = crate::utils::spawn(&[
            "notify-send",
            "-a",
            "margo",
            "-i",
            "preferences-system",
            "-u",
            "critical",
            "-t",
            "3000",
            "Margo",
            "Lock force-cleared",
        ]);
    }

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
        // Wake the redraw scheduler so the loop drains the flag this
        // iteration. Coalesces: many request_repaint() calls between two
        // dispatches still produce a single Ping event (eventfd semantics
        // — see calloop ping source), so we don't need to track whether
        // a wake is already pending.
        //
        // Suppress the ping while a previously-queued frame is still
        // waiting for its vblank. The DRM compositor only accepts one
        // pending page-flip per output, and the post-dispatch animation
        // tick re-arms repaint every iteration; without this gate the
        // ping callback would fire between vblanks and either render an
        // identical scene (wasted work) or hit `queue_frame` "frame
        // already pending" errors. The vblank handler re-emits the ping
        // once it counts back down to zero.
        if self.pending_vblanks == 0 {
            if let Some(ping) = &self.repaint_ping {
                ping.ping();
            }
        }
    }

    /// Called by the udev backend after a successful `queue_frame`.
    /// Pushes the redraw scheduler into "frame in flight" mode so further
    /// repaint requests stay deferred until the page-flip completes.
    pub fn note_frame_queued(&mut self) {
        self.pending_vblanks += 1;
    }

    /// Called by the udev backend's `DrmEvent::VBlank` handler after
    /// `frame_submitted`. If this was the last in-flight frame and the
    /// scene is dirty, re-arm the redraw scheduler.
    pub fn note_vblank(&mut self) {
        self.pending_vblanks = self.pending_vblanks.saturating_sub(1);
        if self.pending_vblanks == 0 && self.repaint_requested {
            if let Some(ping) = &self.repaint_ping {
                ping.ping();
            }
        }
    }

    /// Install the wake handle the udev/winit backend created. Call once
    /// at startup; subsequent [`request_repaint`] calls will wake the
    /// loop via the supplied [`Ping`] sender.
    pub fn set_repaint_ping(&mut self, ping: Ping) {
        self.repaint_ping = Some(ping);
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
            // Skip clients that haven't gone through their deferred
            // initial map yet — they exist in `self.clients` but
            // haven't been placed in `space` and don't have rules
            // applied. Including them in arrange would map them at
            // the layout's default position, which is exactly the
            // pre-rule flicker we deferred to avoid.
            !c.is_initial_map_pending
                && c.is_visible_on(mon_idx, tagset)
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

            // If we're already animating toward exactly this target,
            // leave the in-flight animation alone. arrange_monitor gets
            // called from many event sources (title change → window-
            // rule reapply, focus shift, output resize, scroller pan
            // recompute, …) and a long-running browser like Helium can
            // tick those off every frame while it's playing video. The
            // old behaviour was: each call saw `old != rect` (because
            // `old = c.geom` is the *interpolated* mid-flight value, not
            // the target), restarted the move animation with `initial
            // = old`, and reset `time_started = now`. Result: the
            // animation never finishes — every 16 ms it inches a few
            // pixels toward the target and then resets, producing the
            // exact 1-pixel-per-frame oscillation we kept seeing in the
            // arrange traces (-1794 → -1795 → -1794 → …).
            let already_animating_to_target =
                self.clients[client_idx].animation.running
                    && self.clients[client_idx].animation.current == rect;

            let should_animate = self.config.animations
                && self.config.animation_duration_move > 0
                && !self.clients[client_idx].no_animation
                && !self.clients[client_idx].is_tag_switching
                && old.width > 0
                && old.height > 0
                && old != rect
                && !already_animating_to_target;

            // Diagnostic: every layout decision per visible client.
            let actual_geom = self.clients[client_idx].window.geometry().size;
            tracing::info!(
                "arrange[{}]: client_idx={} old={}x{}+{}+{} slot={}x{}+{}+{} actual_buf={}x{} animate={} already_to_target={}",
                self.clients[client_idx].app_id.as_str(),
                client_idx,
                old.width,
                old.height,
                old.x,
                old.y,
                rect.width,
                rect.height,
                rect.x,
                rect.y,
                actual_geom.w,
                actual_geom.h,
                should_animate,
                already_animating_to_target,
            );
            if should_animate {
                // Animate the slot fully — both position AND size lerp
                // from `old` to `rect` over `animation_duration_move`.
                // Combined with the niri-style crossfade that runs in
                // parallel (snapshot rendered on top with fading
                // alpha, scaled to the *current* interpolated slot),
                // this gives the smooth resize transition the user
                // sees from niri/Hyprland's animated layouts: the
                // pre-resize content scales down while the post-
                // resize content fades up.
                //
                // Earlier we used to snap the size to the target on
                // frame 0 (initial.width = rect.width) so the buffer
                // and the slot would always match dimensions — but
                // that left the snapshot fixed at the new slot size
                // for the entire animation, which meant the snapshot
                // was rendered at a *different* size from the captured
                // content for 150 ms and the user saw a stretched/
                // squished version of the pre-resize image. The
                // crossfade infrastructure makes the size-snap
                // unnecessary: we always render BOTH layers at the
                // interpolated slot, and the buffer/slot mismatch on
                // the live layer is hidden under the snapshot until
                // alpha drops.
                let initial = old;
                // niri-style resize transition: if the slot size
                // changes (not just the position), flag a snapshot so
                // the next render captures the *current* surface tree
                // to a `GlesTexture`. While the move animation
                // interpolates the slot from old to new, the render
                // path draws that snapshot scaled to the live slot
                // instead of the live surface — the OLD content stays
                // pinned visually until the client (Electron, slow
                // ack) commits a buffer at the new size, which drops
                // the snapshot. Without this, Helium's 50–100 ms
                // ack-and-reflow window leaks the buffer-vs-slot
                // mismatch onto the screen.
                let slot_size_changed =
                    old.width != rect.width || old.height != rect.height;
                if slot_size_changed
                    && self.clients[client_idx].resize_snapshot.is_none()
                {
                    self.clients[client_idx].snapshot_pending = true;
                }
                // Spring retarget: if the previous animation was still
                // running, carry its per-channel velocity forward.
                // Without this, the integrator would re-start from rest
                // every time the layout reshuffled mid-flight and the
                // window would visibly hitch — the whole point of the
                // spring clock is that retargets stay continuous.
                // Bezier ignores this field; harmless if it's set.
                // Decide the animation's hard duration. With bezier
                // we honour the user's `animation_duration_move`; with
                // spring we let the physics tell us how long it'll
                // take to settle to within `epsilon` of the target,
                // capped between a sane floor and ceiling so a single
                // bad config value can't produce a 10-second slide.
                let use_spring = self
                    .config
                    .animation_clock_move
                    .eq_ignore_ascii_case("spring");
                let duration_ms = if use_spring {
                    let max_disp = ((rect.x - initial.x).abs())
                        .max((rect.y - initial.y).abs())
                        .max((rect.width - initial.width).abs())
                        .max((rect.height - initial.height).abs())
                        as f64;
                    if max_disp <= 0.5 {
                        // Already at target (sub-pixel). Take the
                        // bezier-style fallback so we still log a
                        // meaningful animation start, but the tick
                        // will settle on the very next frame.
                        self.config.animation_duration_move.max(1)
                    } else {
                        let spring = crate::animation::spring::Spring {
                            from: 0.0,
                            to: max_disp,
                            initial_velocity: 0.0,
                            params: crate::animation::spring::SpringParams::new(
                                self.config.animation_spring_damping_ratio,
                                self.config.animation_spring_stiffness,
                                0.5, // half-pixel epsilon
                            ),
                        };
                        let dur = spring
                            .clamped_duration()
                            .map(|d| d.as_millis() as u32)
                            // Pathological overdamped → fall back.
                            .unwrap_or(self.config.animation_duration_move.max(1));
                        // Clamp: 60 ms floor (one vblank), 1500 ms
                        // ceiling (anything longer is almost certainly
                        // a misconfiguration).
                        dur.clamp(60, 1500)
                    }
                } else {
                    self.config.animation_duration_move.max(1)
                };
                self.clients[client_idx].animation = ClientAnimation {
                    should_animate: true,
                    running: true,
                    time_started: now,
                    last_tick_ms: now,
                    duration: duration_ms,
                    initial,
                    current: rect,
                    action: AnimationType::Move,
                    ..Default::default()
                };
                self.clients[client_idx].geom = initial;
            } else if already_animating_to_target {
                // Existing animation still converging on the right
                // target — leave its `time_started`, `initial`, and the
                // current interpolated `c.geom` exactly where they are.
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
        self.enforce_z_order();
        crate::border::refresh(self);
        self.request_repaint();
    }

    /// Smithay's `Space::map_element` always inserts the touched
    /// element at the top of the stack — there's no way to map at an
    /// explicit z. So every time `arrange_monitor` re-maps a tile-
    /// layer window during a layout change or a move animation, that
    /// tile silently leaps above any floating window (CopyQ,
    /// pavucontrol, picker dialogs) that happened to be on screen.
    ///
    /// To keep "floating sits on top of tiled" actually true, run
    /// this after every `map_element` storm. We re-`raise_element`
    /// floats first, then overlays/scratchpads, in `clients`-vec
    /// forward order — `raise_element` itself moves to top, so the
    /// last raise per band wins, which means the most-recently-
    /// created float of each band ends up at the top of its band
    /// (sane default for "newly opened picker shows on top").
    pub fn enforce_z_order(&mut self) {
        let floats: Vec<smithay::desktop::Window> = self
            .clients
            .iter()
            .filter(|c| (c.is_floating || c.is_in_scratchpad) && !c.is_overlay)
            .map(|c| c.window.clone())
            .collect();
        for w in &floats {
            self.space.raise_element(w, false);
        }
        let overlays: Vec<smithay::desktop::Window> = self
            .clients
            .iter()
            .filter(|c| c.is_overlay)
            .map(|c| c.window.clone())
            .collect();
        for w in &overlays {
            self.space.raise_element(w, false);
        }
    }

    pub fn focus_surface(&mut self, target: Option<FocusTarget>) {
        // Capture the *previously* focused client BEFORE we rewrite
        // the keyboard focus — we need the old + new pair to drive
        // the border-colour cross-fade animation below.
        let prev_focus_idx = self.focused_client_idx();

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

        // Focus highlight cross-fade. When focus moves between two
        // windows, animate both: the outgoing window's border colour
        // fades from `focuscolor` toward `bordercolor`, the incoming
        // one fades the other way. `tick_animations` drives the
        // sample; `border::refresh` reads the in-flight colour from
        // `opacity_animation.current_border_color` and renders that
        // instead of the static color_for() value.
        //
        // Per-client `opacity` (focused_opacity / unfocused_opacity)
        // is animated through the same struct so an unfocused window
        // also dims smoothly instead of snapping to its dimmer alpha
        // — same trick mango/dwl uses but with the right curve.
        let new_focus_idx = self.focused_client_idx();
        if prev_focus_idx != new_focus_idx
            && self.config.animations
            && self.config.animation_duration_focus > 0
        {
            let now = crate::utils::now_ms();
            let dur = self.config.animation_duration_focus;
            let bordercolor = self.config.bordercolor.0;
            let focuscolor = self.config.focuscolor.0;
            // Outgoing: drop focus highlight back to bordercolor +
            // dim opacity to unfocused.
            if let Some(idx) = prev_focus_idx {
                if idx < self.clients.len() {
                    let initial_color =
                        self.clients[idx].opacity_animation.current_border_color;
                    let initial_color = if initial_color == [0.0, 0.0, 0.0, 0.0] {
                        focuscolor
                    } else {
                        initial_color
                    };
                    let initial_opacity = self.clients[idx].focused_opacity;
                    self.clients[idx].opacity_animation = OpacityAnimation {
                        running: true,
                        initial_opacity,
                        target_opacity: self.clients[idx].unfocused_opacity,
                        current_opacity: initial_opacity,
                        time_started: now,
                        duration: dur,
                        initial_border_color: initial_color,
                        target_border_color: bordercolor,
                        current_border_color: initial_color,
                    };
                }
            }
            // Incoming: ramp up to focuscolor + brighten opacity.
            if let Some(idx) = new_focus_idx {
                if idx < self.clients.len() {
                    let initial_color =
                        self.clients[idx].opacity_animation.current_border_color;
                    let initial_color = if initial_color == [0.0, 0.0, 0.0, 0.0] {
                        bordercolor
                    } else {
                        initial_color
                    };
                    let initial_opacity = self.clients[idx].unfocused_opacity;
                    self.clients[idx].opacity_animation = OpacityAnimation {
                        running: true,
                        initial_opacity,
                        target_opacity: self.clients[idx].focused_opacity,
                        current_opacity: initial_opacity,
                        time_started: now,
                        duration: dur,
                        initial_border_color: initial_color,
                        target_border_color: focuscolor,
                        current_border_color: initial_color,
                    };
                }
            }
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

    /// Centralised "what should keyboard focus be right now?" — the niri
    /// pattern. We can't rely on transitional events (layer_destroyed
    /// alone, set_focus from new_surface) because real clients change
    /// focus state in ways those events don't fire for:
    ///
    ///   * **noctalia's launcher / settings panels** don't create or
    ///     destroy a layer surface when they open/close. They keep one
    ///     `MainScreen` `WlrLayershell` per output and just toggle its
    ///     `keyboardFocus` between `Exclusive` and `None`. The transition
    ///     surfaces only as a `wl_surface.commit` with a different
    ///     cached `keyboard_interactivity` — no destroy callback, no
    ///     unmap. Without recomputing focus on every layer commit we
    ///     never notice the panel closed and the key events keep going
    ///     into the void.
    ///   * **session lock with multiple outputs**. Quickshell creates one
    ///     `WlSessionLockSurface` per screen; only the surface on the
    ///     output the user is looking at should hold focus, and that has
    ///     to track cursor motion across outputs.
    ///
    /// This method picks a target by priority and pushes it through the
    /// existing `focus_surface` plumbing only if it differs from the
    /// current focus, so it's cheap to call after every relevant event.
    pub fn refresh_keyboard_focus(&mut self) {
        let desired = self.compute_desired_focus();

        let current = self.seat.get_keyboard().and_then(|kb| kb.current_focus());
        if current.as_ref() == desired.as_ref() {
            tracing::debug!(
                "refresh_keyboard_focus: noop (locked={}, current={:?})",
                self.session_locked,
                current.as_ref().map(focus_target_label),
            );
            return;
        }
        tracing::info!(
            "refresh_keyboard_focus: locked={} current={:?} -> desired={:?}",
            self.session_locked,
            current.as_ref().map(focus_target_label),
            desired.as_ref().map(focus_target_label),
        );
        self.focus_surface(desired);
    }

    fn compute_desired_focus(&self) -> Option<FocusTarget> {
        if self.session_locked {
            // Lock surface on the output under the cursor wins, with
            // graceful fallbacks: focused-monitor's surface, then any
            // surface (so we never end up locked with no focus at all).
            let pointer_output = self
                .monitor_at_point(self.input_pointer.x, self.input_pointer.y)
                .and_then(|i| self.monitors.get(i).map(|m| m.output.clone()));

            if let Some(out) = pointer_output {
                if let Some((_, s)) =
                    self.lock_surfaces.iter().find(|(o, _)| o == &out)
                {
                    return Some(FocusTarget::SessionLock(s.clone()));
                }
            }
            return self
                .lock_surfaces
                .first()
                .map(|(_, s)| FocusTarget::SessionLock(s.clone()));
        }

        // Highest-priority Exclusive layer on Top/Overlay anywhere.
        for layer in self.layer_shell_state.layer_surfaces().rev() {
            let exclusive = layer.with_cached_state(|data| {
                data.keyboard_interactivity
                    == smithay::wayland::shell::wlr_layer::KeyboardInteractivity::Exclusive
                    && matches!(
                        data.layer,
                        smithay::wayland::shell::wlr_layer::Layer::Top
                            | smithay::wayland::shell::wlr_layer::Layer::Overlay
                    )
            });
            if !exclusive {
                continue;
            }
            let mapped = self.space.outputs().find_map(|output| {
                let map = layer_map_for_output(output);
                let found = map
                    .layers()
                    .find(|m| m.layer_surface() == &layer)
                    .map(|m| m.layer_surface().clone());
                found
            });
            if let Some(s) = mapped {
                return Some(FocusTarget::LayerSurface(s));
            }
        }

        // Otherwise: monitor's last-selected client (focus history),
        // falling back to the topmost visible client on the same monitor.
        let mon_idx = self.pointer_monitor().or_else(|| {
            self.focused_client_idx().map(|i| self.clients[i].monitor)
        })?;
        if mon_idx >= self.monitors.len() {
            return None;
        }
        let tagset = self.monitors[mon_idx].current_tagset();
        if let Some(idx) = self.monitors[mon_idx].selected.filter(|&i| {
            i < self.clients.len()
                && self.clients[i].monitor == mon_idx
                && self.clients[i].is_visible_on(mon_idx, tagset)
        }) {
            return Some(FocusTarget::Window(self.clients[idx].window.clone()));
        }
        let idx = self
            .clients
            .iter()
            .position(|c| c.monitor == mon_idx && c.is_visible_on(mon_idx, tagset))?;
        Some(FocusTarget::Window(self.clients[idx].window.clone()))
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

    /// Look up the "home monitor" for a given tag bitmask, by matching
    /// any single bit in the mask against `tagrule = id:N,monitor_name:X`
    /// entries. Returns the monitor index if exactly one tag is set in
    /// the mask AND a tagrule pins it. Used by `view_tag` and
    /// `new_toplevel` to route cross-monitor.
    pub fn tag_home_monitor(&self, tagmask: u32) -> Option<usize> {
        if tagmask == 0 {
            return None;
        }
        // Translate single-bit mask to 1-indexed tag id.
        let id = if tagmask.is_power_of_two() {
            (tagmask.trailing_zeros() + 1) as i32
        } else {
            // Multi-tag mask — use the lowest set bit.
            ((tagmask & tagmask.wrapping_neg()).trailing_zeros() + 1) as i32
        };
        let name = self
            .config
            .tag_rules
            .iter()
            .find(|r| r.id == id && r.monitor_name.is_some())
            .and_then(|r| r.monitor_name.clone())?;
        self.monitors.iter().position(|m| m.name == name)
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

    /// Move keyboard focus + cursor "home" onto the given monitor. Does
    /// NOT change the monitor's current tagset — the caller (view_tag,
    /// focus_mon) is responsible for that. Used by view_tag's tag-home
    /// redirect: if the user presses super+N for a tag pinned to another
    /// monitor, we warp here first so the upcoming view operation
    /// happens in the right place.
    pub fn warp_focus_to_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        let area = self.monitors[mon_idx].monitor_area;
        // Center the pointer on the target monitor so subsequent
        // sloppy-focus / focus-under lookups land on this output.
        self.input_pointer.x = (area.x + area.width / 2) as f64;
        self.input_pointer.y = (area.y + area.height / 2) as f64;
        self.focus_first_visible_or_clear(mon_idx);
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
        if tagmask == 0 {
            return;
        }
        // If a tagrule pins this tag to a specific monitor, jump focus
        // there first so multi-monitor users get niri-style "tag 7 is on
        // eDP-1, super+7 from anywhere takes me to it" behaviour. We
        // skip the redirect when the user is already on the home
        // monitor or the tagmask is the all-tags special value.
        let current_mon = self.focused_monitor();
        let mon_idx = if tagmask != u32::MAX {
            if let Some(home) = self.tag_home_monitor(tagmask) {
                if home != current_mon && home < self.monitors.len() {
                    self.warp_focus_to_monitor(home);
                }
                home
            } else {
                current_mon
            }
        } else {
            current_mon
        };
        if mon_idx >= self.monitors.len() {
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

        // ── Tag transition animation ──────────────────────────────
        //
        // Before flipping the tagset we:
        //
        //   * Capture every client that's about to become invisible
        //     into a `ClosingClient` with `kind = Slide(direction)`
        //     and `is_close = true`. The renderer will draw them
        //     sliding off-screen for `animation_duration_tag` ms;
        //     when settled they pop off the list. (Outgoing windows
        //     stay rendered through the transition so the user sees
        //     them leaving, instead of winking out instantly.)
        //
        //   * Stage every client that's about to become visible at
        //     an off-screen geom so `arrange_monitor` (called below)
        //     starts a Move animation from off-screen → target slot.
        //     That gives the inbound slide for free; we don't need
        //     a second render path.
        //
        // Direction: derived from the bit-position delta of the tag
        // mask. Going to a higher tag → enter from the right / bottom;
        // going to a lower tag → enter from the left / top. Niri does
        // the same; mango's vertical mode swaps the axis.
        let do_anim = self.config.animations
            && self.config.animation_duration_tag > 0
            && current != new_tagmask;
        let direction = self.config.tag_animation_direction;
        let mon_geom = self.monitors[mon_idx].monitor_area;
        let new_idx = current.trailing_zeros() as i32;
        let old_idx_target = new_tagmask.trailing_zeros() as i32;
        let going_forward = old_idx_target > new_idx;
        // Offscreen *staging* origin for the inbound slide. We only set
        // x/y here; the size is taken from the client's previous c.geom
        // below so the animation is a pure translate (no size change,
        // no resize-snapshot capture, no scaling artefacts). The
        // previous version of this code stored a 1×1 rect here, which
        // forced arrange_monitor to start a `1×1 → target.size` move
        // animation; arrange flagged `slot_size_changed` and the
        // renderer ran the resize-snapshot crossfade scaled from a
        // tiny rect up to the slot. That's the "border kadar hızlı
        // hareket etmiyor, sonra yerine oturuyor" symptom — the
        // border tracked the interpolated *slot*, but the snapshot
        // visually expanded from a point because the start size was
        // degenerate.
        let (off_in_xy, off_out_xy): ((i32, i32), (i32, i32)) = match (direction, going_forward) {
            (margo_config::TagAnimDirection::Horizontal, true) => (
                (mon_geom.x + mon_geom.width + 50, mon_geom.y),
                (mon_geom.x - mon_geom.width - 50, mon_geom.y),
            ),
            (margo_config::TagAnimDirection::Horizontal, false) => (
                (mon_geom.x - mon_geom.width - 50, mon_geom.y),
                (mon_geom.x + mon_geom.width + 50, mon_geom.y),
            ),
            (margo_config::TagAnimDirection::Vertical, true) => (
                (mon_geom.x, mon_geom.y + mon_geom.height + 50),
                (mon_geom.x, mon_geom.y - mon_geom.height - 50),
            ),
            (margo_config::TagAnimDirection::Vertical, false) => (
                (mon_geom.x, mon_geom.y - mon_geom.height - 50),
                (mon_geom.x, mon_geom.y + mon_geom.height + 50),
            ),
        };
        let _ = off_out_xy;
        let slide_dir = match (direction, going_forward) {
            (margo_config::TagAnimDirection::Horizontal, true) => {
                crate::render::open_close::SlideDirection::Left
            }
            (margo_config::TagAnimDirection::Horizontal, false) => {
                crate::render::open_close::SlideDirection::Right
            }
            (margo_config::TagAnimDirection::Vertical, true) => {
                crate::render::open_close::SlideDirection::Up
            }
            (margo_config::TagAnimDirection::Vertical, false) => {
                crate::render::open_close::SlideDirection::Down
            }
        };
        let now = crate::utils::now_ms();

        // Snapshot outgoing clients into the close-animation pipeline.
        // We DON'T touch the live `clients` vec — those entries stay
        // around but become invisible per the new tagset; the render
        // path skips them naturally. The snapshot we push here uses
        // the same OpenCloseRenderElement as the toplevel close path,
        // just with a slide kind instead of zoom.
        if do_anim {
            for c in self.clients.iter() {
                if c.monitor != mon_idx {
                    continue;
                }
                let was_vis = c.is_visible_on(mon_idx, current);
                let is_vis = c.is_visible_on(mon_idx, new_tagmask);
                if was_vis && !is_vis {
                    let surface = c.window.wl_surface().map(|s| (*s).clone());
                    self.closing_clients.push(ClosingClient {
                        id: smithay::backend::renderer::element::Id::new(),
                        texture: None,
                        capture_pending: surface.is_some(),
                        geom: c.geom,
                        monitor: mon_idx,
                        // Outgoing snapshot needs to render on *this*
                        // tagset until the slide completes — pin its
                        // visibility tag bitmap to all-bits-set so
                        // `push_closing_clients` always draws it.
                        // The list-removal in `tick_animations` is
                        // what bounds its lifetime.
                        tags: !0u32,
                        time_started: now,
                        duration: self.config.animation_duration_tag,
                        progress: 0.0,
                        kind: crate::render::open_close::OpenCloseKind::Slide(slide_dir),
                        extreme_scale: 1.0, // pure slide, no scale
                        border_radius: self.config.border_radius as f32,
                        source_surface: surface,
                    });
                }
            }
        }

        // Stage incoming clients at an off-screen *but full-size*
        // staging rect so arrange_monitor's Move animation slides
        // them in as a pure translate. We deliberately preserve the
        // client's previous c.geom dimensions: the layout for a
        // returning tag almost always recomputes the same slot size,
        // so initial.size == target.size, `slot_size_changed` is
        // false, and the renderer skips the resize-snapshot path
        // entirely. Border tracks the interpolated `c.geom` and the
        // surface buffer (which is already at the target size,
        // committed during the *previous* visit to this tag) follows
        // it via map_element on each tick. Result: border and surface
        // travel as a unit, with no settle / pop / scale-in.
        if do_anim {
            for c in self.clients.iter_mut() {
                if c.monitor != mon_idx {
                    continue;
                }
                let was_vis = c.is_visible_on(mon_idx, current);
                let is_vis = c.is_visible_on(mon_idx, new_tagmask);
                if !was_vis && is_vis {
                    // First-show case: the client was never properly
                    // arranged on this monitor (typically because it
                    // mapped while a different tag was active — e.g.
                    // a startup script launching apps onto their home
                    // tags before the user has visited those tags).
                    // c.geom is still default `(0, 0, 0, 0)` and no
                    // configure has been sent for the actual slot
                    // size yet. Skip the tag-in animation entirely:
                    // staging an offscreen rect with a fabricated
                    // size would force arrange to run a size-changing
                    // move animation (`mon/2 → slot`), the renderer
                    // would try to capture a resize-snapshot from a
                    // surface tree without a usable buffer, that
                    // capture would fail or render at the wrong size,
                    // and the user would see the live surface stuck
                    // at its default size while the border tracked
                    // the slot — exactly the "first-launch via
                    // semsumo doesn't fit, pkill+relaunch fixes it"
                    // symptom on Spotify and Helium. By falling
                    // through, `arrange_monitor` runs its
                    // direct-snap branch (because `old.width == 0`
                    // makes `should_animate` false), pushes the
                    // window to its slot in one go, and sends the
                    // first valid configure. The next tag-switch
                    // visit will animate normally with a populated
                    // c.geom.
                    if c.geom.width <= 0 || c.geom.height <= 0 {
                        continue;
                    }
                    c.geom = crate::layout::Rect {
                        x: off_in_xy.0,
                        y: off_in_xy.1,
                        width: c.geom.width,
                        height: c.geom.height,
                    };
                    // Force arrange to start a fresh animation (the
                    // already_animating_to_target guard would skip if
                    // a previous animation's target happens to match).
                    c.animation.running = false;
                }
            }
        }

        self.update_pertag_for_tagset(mon_idx, new_tagmask);
        self.arrange_monitor(mon_idx);
        self.focus_first_visible_or_clear(mon_idx);
        if do_anim {
            self.request_repaint();
        }
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
                if self
                    .lock_surfaces
                    .iter()
                    .any(|(_, s)| s.wl_surface() == &root)
                {
                    tracing::info!(
                        "session_lock: commit on lock surface {:?}, surfaces total={}",
                        root.id(),
                        self.lock_surfaces.len()
                    );
                    // First commit on a lock surface = it's now mapped (has
                    // a buffer attached). Run focus refresh AT THIS POINT
                    // so `wl_keyboard.enter` lands on a fully-formed
                    // surface — Qt's QtWayland plugin doesn't always wire
                    // forceActiveFocus on the QML TextInput until the
                    // QQuickWindow has received both surface activation
                    // AND a paint event, so re-issuing focus once the
                    // first buffer commits is what flips the password
                    // field from "renders but is dead" to "accepts the
                    // first keystroke." `refresh_keyboard_focus` also
                    // makes sure the surface that gets focus is the one
                    // on the cursor's output (not always the first
                    // surface in `lock_surfaces`).
                    self.refresh_keyboard_focus();
                    self.request_repaint();
                    return;
                }
            }

            // First check if this commit belongs to a client we've
            // deferred (created in `new_toplevel`, not yet mapped
            // because we wanted to wait for app_id before applying
            // window rules). If so, finalise the initial map now.
            let deferred_idx = self
                .clients
                .iter()
                .position(|c| {
                    c.is_initial_map_pending
                        && c.window.wl_surface().as_deref() == Some(&root)
                });
            if let Some(idx) = deferred_idx {
                self.finalize_initial_map(idx);
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

                // A layer commit can flip `keyboard_interactivity` —
                // noctalia's bar / launcher / settings / control-center
                // all live on a single per-screen MainScreen layer and
                // mutate `WlrLayershell.keyboardFocus` between
                // `Exclusive` and `None` instead of destroying the
                // surface. Without recomputing focus here, closing one
                // of those panels with Esc leaves keyboard focus
                // pinned to the (still-alive) layer surface in `None`
                // mode — keys go nowhere until the user nudges the
                // mouse, which is exactly what made "rofi works but
                // the noctalia launcher does not" reproducible.
                self.refresh_keyboard_focus();
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

// ── Smithay delegate: linux-drm-syncobj-v1 (explicit sync) ───────────────────
//
// The protocol global is only exposed when the udev backend has had a
// chance to test the primary DRM node for `syncobj_eventfd` support and
// flipped `drm_syncobj_state` to `Some`. Until that happens
// `drm_syncobj_state()` returns `None` and smithay's dispatch refuses to
// bind, so kernels / drivers without timeline syncobj support don't see
// a global advertised at all (which is exactly the contract niri /
// sway / mutter follow). Once the global is up, the per-surface
// `wp_linux_drm_syncobj_surface_v1` plumbs acquire + release fences
// through smithay's compositor pre-commit hooks automatically — clients
// (Chromium 100+, Firefox with `widget.dmabuf-textures.enabled`) can
// stop relying on implicit-sync hacks and tile their frame pacing on a
// real GPU timeline.
impl DrmSyncobjHandler for MargoState {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.drm_syncobj_state.as_mut()
    }
}
smithay::delegate_drm_syncobj!(MargoState);

// ── Deferred initial map (out-of-trait helper) ───────────────────────────────

impl MargoState {
    /// Finalize the deferred initial map of a client created in
    /// `new_toplevel` but held back from `space.map_element` until its
    /// app_id had a chance to arrive. Called from the commit handler
    /// the first time a buffer is attached to the toplevel's surface.
    /// At this point Qt clients have invariably set `app_id`, so
    /// window rules can be applied with their full intended effect
    /// (`isfloating`, custom geom, tag pinning, …) BEFORE the window
    /// is ever placed in the smithay space — no rule-jump flicker.
    pub(crate) fn finalize_initial_map(&mut self, idx: usize) {
        // Sync the latest app_id / title from the surface before
        // running window rules — by this point Qt has had its chance.
        if idx >= self.clients.len() {
            return;
        }
        if let WindowSurface::Wayland(toplevel) = self.clients[idx].window.underlying_surface() {
            let (app_id, title) = read_toplevel_identity(&toplevel);
            self.clients[idx].app_id = app_id;
            self.clients[idx].title = title;
        }

        // Now run rules with the live app_id/title.
        let _changed = self.apply_window_rules_to_client(idx);

        // Tag-home redirect: if rules picked tag N but didn't pin a
        // monitor, route to the tag's home output.
        let no_explicit_monitor = !self
            .matching_window_rules(
                &self.clients[idx].app_id,
                &self.clients[idx].title,
            )
            .iter()
            .any(|r| r.monitor.is_some());
        if no_explicit_monitor {
            if let Some(home) = self.tag_home_monitor(self.clients[idx].tags) {
                self.clients[idx].monitor = home;
            }
        }

        let target_mon = self.clients[idx].monitor;
        let focus_new =
            !self.clients[idx].no_focus && !self.clients[idx].open_silent;
        let window = self.clients[idx].window.clone();

        let map_loc = self
            .monitors
            .get(target_mon)
            .map(|m| (m.monitor_area.x, m.monitor_area.y))
            .unwrap_or((0, 0));
        self.space.map_element(window.clone(), map_loc, true);

        if focus_new {
            if target_mon < self.monitors.len() {
                self.monitors[target_mon].prev_selected =
                    self.monitors[target_mon].selected;
                self.monitors[target_mon].selected = Some(idx);
            }
            self.focus_surface(Some(FocusTarget::Window(window)));
        }

        // Mark the client mapped BEFORE arrange so the layout pass
        // sees it as a real participant.
        self.clients[idx].is_initial_map_pending = false;

        if !self.monitors.is_empty() {
            self.arrange_monitor(target_mon);
        }

        // Inactive-tag bootstrap. If the client mapped onto a tag
        // that's not currently active on `target_mon` (typical at
        // session start: `semsumo-daily` launches Spotify with
        // tag-rule `tags:8` while the user is still on tag 1), the
        // arrange_monitor call above ran with the *current* tagset
        // and skipped this client — it didn't get a slot, didn't
        // receive a configure, and its `c.geom` stays at the
        // `Rect::default()` zero rect. The client picks its own
        // default size and commits a buffer at that size; later,
        // when the user finally tag-switches in, arrange has to
        // run a `default → slot` transition that fights the
        // pre-existing buffer. That's the long tail of the
        // "Spotify only fits the border after pkill+relaunch"
        // symptom.
        //
        // Kick a configure with the monitor's working area as a
        // sane default so the client can at least commit at a
        // reasonable size during launch. The eventual tag-switch
        // arrange will send a *real* configure (the actual slot
        // computed by the layout) and the resize transition there
        // will work as designed because the client now has a real
        // buffer to snapshot.
        if self.clients[idx].geom.width <= 0 || self.clients[idx].geom.height <= 0 {
            if let Some(mon) = self.monitors.get(target_mon) {
                let area = mon.monitor_area;
                self.clients[idx].geom = Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: area.height,
                };
                if let WindowSurface::Wayland(toplevel) =
                    self.clients[idx].window.underlying_surface()
                {
                    toplevel.with_pending_state(|state| {
                        state.size = Some(smithay::utils::Size::from((
                            area.width,
                            area.height,
                        )));
                    });
                    let initial_sent = with_states(toplevel.wl_surface(), |states| {
                        states
                            .data_map
                            .get::<XdgToplevelSurfaceData>()
                            .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                            .unwrap_or(false)
                    });
                    if !initial_sent {
                        toplevel.send_configure();
                    }
                }
                tracing::info!(
                    "inactive-tag bootstrap: app_id={} tags={:#x} \
                     active_tagset={:#x} sent default {}x{}",
                    self.clients[idx].app_id,
                    self.clients[idx].tags,
                    mon.current_tagset(),
                    area.width,
                    area.height,
                );
            }
        }

        // Kick off the open animation if globally enabled, this client
        // didn't opt out (window-rule `no_animation` / `open_silent`),
        // and the user configured a non-zero open duration. The
        // renderer captures the surface into a `GlesTexture` on the
        // very next frame (driven by `opening_capture_pending`) and
        // from then on the live `wl_surface` is hidden — we only draw
        // the snapshot through `OpenCloseRenderElement` until the
        // curve settles. This eliminates the "instant pop at the new
        // geom for one frame, then the animation kicks in" flash that
        // pure wrap-the-live-surface approaches produce.
        if self.config.animations
            && self.config.animation_duration_open > 0
            && !self.clients[idx].no_animation
            && !self.clients[idx].open_silent
        {
            // Per-client override (set by window-rule
            // `animation_type_open=…`) wins over the global config.
            let kind_str = self.clients[idx]
                .animation_type_open
                .clone()
                .unwrap_or_else(|| self.config.animation_type_open.clone());
            let kind = crate::render::open_close::OpenCloseKind::parse(&kind_str);
            let now = crate::utils::now_ms();
            self.clients[idx].opening_animation =
                Some(crate::animation::OpenCloseClientAnim {
                    kind,
                    time_started: now,
                    duration: self.config.animation_duration_open,
                    progress: 0.0,
                    extreme_scale: self.config.zoom_initial_ratio.clamp(0.05, 1.0),
                });
            self.clients[idx].opening_capture_pending = true;
            self.request_repaint();
        }

        tracing::info!(
            "finalize_initial_map: app_id={} idx={idx} monitor={target_mon} \
             floating={} tags={:#x} open_anim={}",
            self.clients[idx].app_id,
            self.clients[idx].is_floating,
            self.clients[idx].tags,
            self.clients[idx].opening_animation.is_some(),
        );
    }
}

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

        // Defer the actual map / rule-application / arrange / focus
        // until the first commit. Qt clients (CopyQ, KeePassXC, the
        // GTK file picker via `pcmanfm-qt`, …) almost always create
        // the xdg_toplevel role *before* sending `set_app_id`, so at
        // this point `app_id` is empty and any windowrule keyed on
        // `appid:^copyq$` doesn't fire. If we mapped the window now
        // the user would see the toplevel briefly at the layout's
        // default position (top-left of the focused monitor) and
        // then snap to its rule-driven floating geometry one frame
        // later — the visible "super+v ile copyq açtığımda pencere
        // çok hızlı bir şekilde bir kaybolup tekrar gözüküyor"
        // flicker. Holding the map until the first commit (when Qt
        // has had its chance to set app_id and we can look up the
        // right rules) eliminates that flicker entirely.
        client.is_initial_map_pending = true;

        let ft_handle = self.foreign_toplevel_list.new_toplevel::<Self>(&title, &app_id);
        ft_handle.send_done();
        client.foreign_toplevel_handle = Some(ft_handle);

        // Smart-insert (niri pattern): in scroller layout, place the new
        // client right after the focused one so closing it returns you near
        // your previous position. Other layouts are order-agnostic.
        let target_mon = client.monitor;
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

        tracing::info!(
            "new toplevel: app_id={:?} monitor={target_mon} idx={new_idx} \
             (map deferred until first commit)",
            if app_id.is_empty() { "<unset>" } else { &app_id },
        );
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
            // Enqueue a close animation entry BEFORE removing the
            // client. The renderer captures the wl_surface to a
            // texture on its very next frame (the surface is still
            // alive — Wayland clients destroy their xdg_toplevel role
            // first, then their wl_surface), and from then on draws
            // the texture scaled+faded out around the slot's centre.
            // Without this, the user sees windows wink out instantly
            // when closed; with it, they pull in like the rest of a
            // modern compositor's behaviour.
            //
            // We still unmap and remove from `clients` immediately so
            // every other state machine (focus stack, layout, scene
            // ordering) treats the close as having happened. The
            // closing entry lives in `closing_clients` purely as a
            // render-side concern.
            if self.config.animations
                && self.config.animation_duration_close > 0
                && !self.clients[idx].no_animation
            {
                let kind_str = self.clients[idx]
                    .animation_type_close
                    .clone()
                    .unwrap_or_else(|| self.config.animation_type_close.clone());
                let kind = crate::render::open_close::OpenCloseKind::parse(&kind_str);
                let now = crate::utils::now_ms();
                let c = &self.clients[idx];
                self.closing_clients.push(ClosingClient {
                    id: smithay::backend::renderer::element::Id::new(),
                    texture: None,
                    capture_pending: true,
                    geom: c.geom,
                    monitor: c.monitor,
                    tags: c.tags,
                    time_started: now,
                    duration: self.config.animation_duration_close,
                    progress: 0.0,
                    kind,
                    extreme_scale: self.config.zoom_end_ratio.clamp(0.05, 1.0),
                    border_radius: self.config.border_radius as f32,
                    source_surface: Some(wl_surf.clone()),
                });
                self.request_repaint();
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
        let wl_surface_clone = desktop_layer.wl_surface().clone();
        {
            let mut map = layer_map_for_output(&smithay_output);
            map.map_layer(&desktop_layer).unwrap();
            map.arrange();
        }
        self.refresh_output_work_area(&smithay_output);

        // Open animation: fade in from `layer_animation_type_open`
        // direction. We use the live render path during the
        // transition (no snapshot needed — surface is alive),
        // applying a per-layer alpha + offset based on this
        // animation's progress in `push_layer_elements`.
        if self.config.animations
            && self.config.layer_animations
            && self.config.animation_duration_open > 0
        {
            let kind = crate::render::open_close::OpenCloseKind::parse(
                &self.config.layer_animation_type_open,
            );
            let now = crate::utils::now_ms();
            self.layer_animations.insert(
                wl_surface_clone.id(),
                LayerSurfaceAnim {
                    time_started: now,
                    duration: self.config.animation_duration_open,
                    progress: 0.0,
                    is_close: false,
                    texture: None,
                    capture_pending: false,
                    geom: Rect::default(),
                    kind,
                    source_surface: None,
                },
            );
            self.request_repaint();
        }

        tracing::info!(
            "new layer surface: namespace={namespace} output={} anim={}",
            smithay_output.name(),
            self.layer_animations.contains_key(&wl_surface_clone.id()),
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

        // Close animation: capture the layer's wl_surface tree to a
        // texture and push a `LayerSurfaceAnim` entry so the renderer
        // keeps painting it sliding/fading away after smithay's
        // `LayerMap::unmap_layer` removes it. Cancel any pending open
        // animation for the same surface — if a layer was destroyed
        // mid-open we just play the close from wherever the open was.
        let wl_surf = surface.wl_surface().clone();
        if self.config.animations
            && self.config.layer_animations
            && self.config.animation_duration_close > 0
        {
            // Read geometry off the layer map BEFORE we unmap it.
            let geom = layer.as_ref().and_then(|l| {
                let map = layer_map_for_output(&output);
                map.layer_geometry(l).map(|g| Rect {
                    x: g.loc.x,
                    y: g.loc.y,
                    width: g.size.w,
                    height: g.size.h,
                })
            });
            if let Some(geom) = geom {
                let kind = crate::render::open_close::OpenCloseKind::parse(
                    &self.config.layer_animation_type_close,
                );
                let now = crate::utils::now_ms();
                self.layer_animations.insert(
                    wl_surf.id(),
                    LayerSurfaceAnim {
                        time_started: now,
                        duration: self.config.animation_duration_close,
                        progress: 0.0,
                        is_close: true,
                        texture: None,
                        capture_pending: true,
                        geom,
                        kind,
                        source_surface: Some(wl_surf.clone()),
                    },
                );
                self.request_repaint();
            }
        }

        if let Some(layer) = layer {
            let mut map = layer_map_for_output(&output);
            map.unmap_layer(&layer);
            map.arrange();
        }

        self.refresh_output_work_area(&output);

        // Hand keyboard focus back to a real window when the layer that
        // had grabbed it (typically noctalia's launcher / settings panel
        // / control-center, all of them `keyboard-interactivity:
        // exclusive`) goes away. Without this, keyboard.current_focus
        // is left pointing at the just-destroyed surface, every key
        // press is delivered to nothing, and the user has to nudge the
        // mouse before the toplevel underneath wakes up — exactly the
        // "esc the launcher and the window stays dead" symptom.
        //
        // Only intervene if the destroyed layer was actually holding
        // focus: a non-exclusive layer (notification toasts, the bar)
        // disappearing should not yank focus around. We pick the
        // monitor's currently `selected` client as the fallback because
        // that's the last-focused-window-on-this-output that
        // focus_surface tracked, falling back to the topmost visible
        // client if nothing was previously selected (fresh session,
        // or the prior focus belonged to a different layer).
        let current_focus_was_layer = self
            .seat
            .get_keyboard()
            .and_then(|kb| kb.current_focus())
            .map(|f| match f {
                FocusTarget::LayerSurface(s) => s == surface,
                _ => false,
            })
            .unwrap_or(false);

        if current_focus_was_layer {
            let restore = self.monitors[mon_idx]
                .selected
                .filter(|&idx| {
                    idx < self.clients.len()
                        && self.clients[idx].is_visible_on(
                            mon_idx,
                            self.monitors[mon_idx].current_tagset(),
                        )
                })
                .or_else(|| {
                    let tagset = self.monitors[mon_idx].current_tagset();
                    self.clients
                        .iter()
                        .position(|c| c.monitor == mon_idx && c.is_visible_on(mon_idx, tagset))
                });

            match restore {
                Some(idx) => {
                    let window = self.clients[idx].window.clone();
                    self.monitors[mon_idx].selected = Some(idx);
                    self.focus_surface(Some(FocusTarget::Window(window)));
                }
                None => self.focus_surface(None),
            }
        }

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

        // Tag-home redirect: if a windowrule set `tags:N` but DIDN'T pin
        // a `monitor:`, route to the tag's home monitor as defined by
        // `tagrule = id:N, monitor_name:X`. Lets the user write
        //   tagrule = id:7, monitor_name:eDP-1
        //   windowrule = tags:7, appid:^transmission$
        // and the windowrule doesn't have to repeat `monitor:eDP-1`.
        let no_explicit_monitor = !self
            .matching_window_rules(&client.app_id, &client.title)
            .iter()
            .any(|r| r.monitor.is_some());
        if no_explicit_monitor {
            if let Some(home) = self.tag_home_monitor(client.tags) {
                client.monitor = home;
            }
        }

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
        tracing::info!(
            "session_lock: lock() called (was locked={}, lock_surfaces={})",
            self.session_locked,
            self.lock_surfaces.len()
        );
        confirmation.lock();
        self.session_locked = true;
        self.arrange_all();
    }

    fn unlock(&mut self) {
        tracing::info!("session_lock: unlock() called");
        self.session_locked = false;
        self.lock_surfaces.clear();
        self.arrange_all();
        // After unlock, push focus back to a real window — by default
        // current_focus is still pointing at the (now-dead) lock surface
        // and the user has to nudge the mouse before any keys reach the
        // toplevel underneath.
        self.refresh_keyboard_focus();
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

        self.lock_surfaces.push((output, surface));
        // Don't try to set focus here: the wl_surface exists but has no
        // buffer yet, so `wl_keyboard.enter` arrives before Qt's
        // QQuickWindow is paint-ready and the password TextInput's
        // `forceActiveFocus()` no-ops. The commit handler runs the
        // refresh once the surface attaches its first buffer, which
        // both fixes that timing AND picks the lock surface on the
        // user's monitor instead of the first one in `lock_surfaces`.
        self.refresh_keyboard_focus();
        self.request_repaint();
    }
}
delegate_session_lock!(MargoState);

// ── Smithay delegate: text-input-v3 + input-method-v2 ────────────────────────
//
// Qt's `text-input-v3` plugin is what backs every `QML.TextInput` field on
// Wayland. It probes for both `wp_text_input_v3` and `zwp_input_method_v2`
// globals at activate-time; if either one is missing, Qt falls back to a
// degraded path where keystrokes are NOT routed to the focused TextInput
// even though `wl_keyboard.key` is being delivered to the surface. The
// most visible symptom: noctalia's lock screen receives wl_keyboard.enter
// just fine, the cursor blinks, MouseArea forces focus — and yet the
// password field stays empty no matter what you type.
//
// Smithay handles all the protocol plumbing as long as the globals are
// registered. We do NOT drive an IME ourselves (no fcitx/ibus integration
// here), so the handler is intentionally minimal: input-method popups
// just get tracked through the regular xdg popup manager so they render
// at the right location, and dismissal hooks back into PopupManager.

impl InputMethodHandler for MargoState {
    fn new_popup(&mut self, surface: InputMethodPopupSurface) {
        if let Err(err) = self
            .popups
            .track_popup(smithay::desktop::PopupKind::from(surface))
        {
            tracing::warn!("input_method: failed to track popup: {err}");
        }
    }

    fn popup_repositioned(&mut self, _surface: InputMethodPopupSurface) {}

    fn dismiss_popup(&mut self, surface: InputMethodPopupSurface) {
        if let Some(parent) = surface.get_parent().map(|p| p.surface.clone()) {
            let _ = smithay::desktop::PopupManager::dismiss_popup(
                &parent,
                &smithay::desktop::PopupKind::from(surface),
            );
        }
    }

    fn parent_geometry(
        &self,
        parent: &WlSurface,
    ) -> Rectangle<i32, smithay::utils::Logical> {
        // Look up the parent toplevel and report its window-geometry so
        // input-method popups (e.g. fcitx candidate window) can position
        // relative to the cursor inside the focused window.
        self.space
            .elements()
            .find_map(|w| {
                (w.wl_surface().as_deref() == Some(parent)).then(|| w.geometry())
            })
            .unwrap_or_default()
    }
}
delegate_text_input_manager!(MargoState);
delegate_input_method_manager!(MargoState);

// ── Smithay delegate: pointer constraints + relative pointer ─────────────────
//
// `wp_pointer_constraints_v1` lets clients lock or confine the cursor to
// their surface. Two flavours:
//   * Lock: the pointer's *position* on screen freezes at request time;
//     the client still receives relative_motion events, but nothing else.
//     This is the FPS / Blender / DCC-app pattern — the user moves the
//     mouse, the camera turns, the cursor itself doesn't visibly drift.
//   * Confine: the pointer is allowed to move freely, but only inside
//     the surface (and an optional sub-region). Used by Krita to keep
//     the brush from leaving the canvas during a drag, and by remote-
//     desktop clients to keep the host pointer trapped inside the
//     remote view.
//
// `wp_relative_pointer_manager_v1` is the natural complement: it lets
// clients listen for pure delta-only motion events, so a locked pointer
// still reports "the user moved the mouse by Δ" without leaking an
// absolute position. Our `handle_pointer_motion` already calls
// `pointer.relative_motion(...)` on every libinput delta, so once the
// global is registered all clients can bind a `wp_relative_pointer_v1`
// per pointer and get the full event stream.
//
// Constraint *enforcement* (lock the cursor, clamp to region) lives in
// `input_handler::handle_pointer_motion`; this module only wires the
// protocol surface.
impl PointerConstraintsHandler for MargoState {
    fn new_constraint(
        &mut self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        pointer: &smithay::input::pointer::PointerHandle<Self>,
    ) {
        // Activate the constraint immediately if the pointer is
        // already over the requesting surface. The client typically
        // requests a constraint while it has pointer focus (a
        // fullscreen game, a Blender viewport drag, …), so this is
        // the common path. If the pointer is somewhere else when
        // the request arrives, smithay defers activation until
        // pointer focus moves into the surface.
        let Some(current_focus) = pointer.current_focus() else {
            return;
        };
        if current_focus.wl_surface().as_deref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |constraint| {
                if let Some(constraint) = constraint {
                    constraint.activate();
                }
            });
        }
    }

    fn cursor_position_hint(
        &mut self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        pointer: &smithay::input::pointer::PointerHandle<Self>,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        // While a lock is active, the client may suggest a
        // post-unlock cursor position via this hint (e.g. "the
        // crosshair was at (320, 200) when I locked, please put the
        // cursor there when unlocking"). Honour it only if the
        // constraint is currently active and the surface still owns
        // the pointer.
        let active = with_pointer_constraint(surface, pointer, |constraint| {
            constraint.is_some_and(|c| c.is_active())
        });
        if !active {
            return;
        }
        // Resolve the surface's screen origin so we can convert the
        // surface-relative `location` hint to compositor-global
        // coordinates.
        let origin = self
            .space
            .elements()
            .find_map(|window| {
                (window.wl_surface().as_deref() == Some(surface)).then(|| {
                    self.space
                        .element_location(window)
                        .unwrap_or_default()
                })
            })
            .unwrap_or_default()
            .to_f64();
        let target = origin + location;
        // Update the pointer's tracked location AND our own
        // `input_pointer` shadow so the next motion event runs from
        // the correct anchor.
        pointer.set_location(target);
        self.input_pointer.x = target.x;
        self.input_pointer.y = target.y;
    }
}
delegate_pointer_constraints!(MargoState);
delegate_relative_pointer!(MargoState);

// ── Smithay delegate: xdg-activation-v1 ──────────────────────────────────────
//
// xdg-activation is the polite focus-stealing channel. Use cases:
//   * Notification daemon "Reply" / "Open" action buttons asking the
//     compositor to activate the conversation thread in the messenger
//     app the notification came from.
//   * `notify-send -A` style scripts that want the user to come back
//     to a long-running task after the OK click.
//   * `xdg-desktop-portal-wlr`'s `Activate` request, used by Discord
//     screen-share, Telegram desktop, etc., to bring themselves to
//     the front when the user clicks a system-tray icon.
//   * Browser → mailto: → Thunderbird already running → activate.
//
// Anti-focus-steal: spec recommends rejecting any token whose creating
// client wasn't the most recently keyboard-focused one. We follow
// anvil's reading: the token is valid only if its bundled serial is
// no older than our seat keyboard's last `enter` event, AND the seat
// in the token matches our seat. Without this, anything that knows
// the protocol could steal focus by spinning up a token at any time.
//
// On accept we route through the same focus path the user's bindings
// use: switch to the target window's tag, restore that monitor, focus
// the window. That keeps activation-driven jumps consistent with
// alt+tab / explicit `mctl dispatch view N`.

impl XdgActivationHandler for MargoState {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn token_created(
        &mut self,
        _token: XdgActivationToken,
        data: XdgActivationTokenData,
    ) -> bool {
        // A token without a (serial, seat) bundle is suspicious —
        // someone scripted activation without going through a real
        // user interaction. Reject.
        let Some((serial, seat)) = data.serial else {
            return false;
        };
        // Different seat? Don't trust.
        if Seat::<MargoState>::from_resource(&seat).as_ref() != Some(&self.seat) {
            return false;
        }
        // Serial must be no older than the seat keyboard's last enter
        // — i.e. the requesting client was the keyboard-focused one
        // when it generated the token.
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        let Some(last_enter) = keyboard.last_enter() else {
            return false;
        };
        serial.is_no_older_than(&last_enter)
    }

    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        // Token expires after 10s — older requests are stale (the
        // user has moved on). Anvil's value, matches GNOME mutter's.
        if token_data.timestamp.elapsed().as_secs() >= 10 {
            return;
        }

        // Find which client owns the surface.
        let Some(idx) = self
            .clients
            .iter()
            .position(|c| c.window.wl_surface().as_deref() == Some(&surface))
        else {
            return;
        };

        // Switch to the client's tag (view its mask). Multi-bit
        // masks pick the lowest set bit so we land on a single
        // canonical tag rather than enabling several at once. The
        // existing view_tag handles the per-tag home-monitor warp,
        // so multi-monitor users come back to the right output too.
        let mask = self.clients[idx].tags;
        let one_bit = mask & mask.wrapping_neg();
        let target = if one_bit != 0 { one_bit } else { mask };
        self.view_tag(target);

        // Focus + raise. focus_surface tracks selected/prev-selected
        // history per monitor, and the layer-mapped Space takes care
        // of the actual stack ordering when we follow with
        // enforce_z_order so the activated window comes to the top
        // of its z-band.
        let window = self.clients[idx].window.clone();
        self.focus_surface(Some(FocusTarget::Window(window.clone())));
        self.space.raise_element(&window, true);
        self.enforce_z_order();
        self.request_repaint();

        tracing::info!(
            "xdg_activation: activated app_id={} idx={} tag={:#x}",
            self.clients[idx].app_id,
            idx,
            target
        );
    }
}
delegate_xdg_activation!(MargoState);

// ── Smithay delegate: wlr-output-management-v1 ───────────────────────────────

impl crate::protocols::output_management::OutputManagementHandler for MargoState {
    fn output_management_state(
        &mut self,
    ) -> &mut crate::protocols::output_management::OutputManagementManagerState {
        &mut self.output_management_state
    }

    fn apply_output_pending(
        &mut self,
        pending: std::collections::HashMap<
            String,
            crate::protocols::output_management::PendingHeadConfig,
        >,
    ) -> bool {
        // Hard-reject any change we can't safely apply. Mode
        // changes need DRM-level re-modeset which is risky and out
        // of scope for v1; disable requests need teardown of an
        // entire OutputDevice. Both come back next iteration.
        if pending
            .values()
            .any(|p| p.requests_mode_change() || !p.enabled())
        {
            tracing::warn!("output_management: rejecting config (mode/disable not yet supported)");
            return false;
        }

        // Apply the subset we DO support: scale, transform,
        // position. Each goes through smithay's
        // `Output::change_current_state` which both updates the
        // output's recorded values AND broadcasts wl_output events
        // to all clients (so their fractional-scale-aware widgets
        // re-layout). Layout reflow happens via arrange_all.
        let mut changed = false;
        for (name, p) in &pending {
            let Some(mon_idx) = self.monitors.iter().position(|m| m.name == *name) else {
                tracing::warn!(
                    "output_management: ignoring pending head for unknown output {name}"
                );
                continue;
            };
            let mon = &self.monitors[mon_idx];
            let output = mon.output.clone();
            let mut local_change = false;

            if let Some(scale) = p.scale() {
                output.change_current_state(
                    None,
                    None,
                    Some(smithay::output::Scale::Fractional(scale)),
                    None,
                );
                local_change = true;
            }
            if let Some(t) = p.transform() {
                let smithay_t: smithay::utils::Transform = t.into();
                output.change_current_state(None, Some(smithay_t), None, None);
                local_change = true;
            }
            if let Some((x, y)) = p.position() {
                output.change_current_state(
                    None,
                    None,
                    None,
                    Some(smithay::utils::Point::from((x, y))),
                );
                local_change = true;
            }
            if local_change {
                self.refresh_output_work_area(&output);
                changed = true;
            }
        }
        if changed {
            self.arrange_all();
            self.request_repaint();
            // Re-publish topology so other wlr-output-management
            // clients (kanshi watchers, secondary wlr-randr) see
            // the new state.
            self.publish_output_topology();
        }
        changed
    }
}
crate::delegate_output_management!(MargoState);
delegate_presentation!(MargoState);

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
