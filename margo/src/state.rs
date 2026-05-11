#![allow(dead_code)]

// W4.2: per-protocol handler impls extracted into sibling files
// under `state/handlers/` for incremental-compile wins. Each
// submodule reaches into `MargoState` via `crate::state::MargoState`.
mod handlers;

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

use anyhow::{Context, Result};
use smithay::{
    backend::allocator::dmabuf::Dmabuf,
    delegate_output,
    delegate_seat, delegate_shm,
    delegate_presentation,
    desktop::{layer_map_for_output, PopupManager, Space, Window, WindowSurface},
    input::{
        Seat, SeatHandler, SeatState,
        dnd::{DndFocus, Source},
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
            Display,
        },
    },
    utils::{Clock, Logical, Monotonic, Point, Serial, Size, SERIAL_COUNTER},
    wayland::{
        compositor::{with_states, CompositorClientState, CompositorState},
        output::{OutputHandler, OutputManagerState},
        seat::WaylandFocus,
        selection::{
            data_device::{set_data_device_focus, DataDeviceState, WlOfferData},
            primary_selection::{set_primary_focus, PrimarySelectionState},
            wlr_data_control::DataControlState,
        },
        shell::{
            wlr_layer::{
                LayerSurface as WlrLayerSurface, WlrLayerShellState,
            },
            xdg::{
                decoration::XdgDecorationState,
                ToplevelSurface, XdgShellState, XdgToplevelSurfaceData,
            },
        },
        shm::{ShmHandler, ShmState},
        session_lock::LockSurface,
        input_method::InputMethodManagerState,
        pointer_constraints::PointerConstraintsState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        text_input::TextInputManagerState,
        xdg_activation::XdgActivationState,
        viewporter::ViewporterState,
        dmabuf::{DmabufGlobal, DmabufState},
        drm_syncobj::DrmSyncobjState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Surface, X11Wm},
};

use margo_config::{parse_config, Config, WindowRule};

/// Filesystem path of the runtime state file consumed by mctl's
/// rich subcommands (`clients`, `outputs`, the prettier
/// `status`). Default location: `$XDG_RUNTIME_DIR/margo/state.json`,
/// fallback `/run/user/$UID/margo/state.json` if XDG isn't set.
pub fn state_file_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/run/user/{uid}"))
        });
    dir.join("margo").join("state.json")
}

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
        FocusTarget::Popup(s) => format!("Popup({:?})", s.id()),
    }
}

// ── Hot corner ───────────────────────────────────────────────────────────────

/// Which screen corner the pointer is currently dwelling in. niri's
/// pattern: a 1×1-logical-pixel rectangle at each corner; pointer
/// entry arms a dwell timer; if the pointer stays past
/// `Config::hot_corner_dwell_ms` the configured action fires
/// (`config.hot_corner_top_left` etc., e.g. `"toggle_overview"`).
///
/// Only one corner can be active at a time — pointer-leave clears.
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
///
/// Mode-by-mode:
///
/// * `WorkArea` — the standard "fill the available space" feeling.
///   Pencere `MargoMonitor::work_area` boyutuna büyür (yani
///   layer-shell exclusion zone'undan sonra kalan alan); bar /
///   notification overlay görünür kalır. Çoğu compositor'un default
///   `F11` davranışı.
///
/// * `Exclusive` — gerçek "tam ekran". Pencere `monitor_area`'yı
///   tam kaplar; render path o output için tüm layer-shell katmanlarını
///   (Overlay / Top / Bottom / Background) suppress eder, böylece bar
///   pencerenin üstüne çıkamaz. mpv / browser fullscreen movie / oyun
///   için doğru davranış.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FullscreenMode {
    #[default]
    Off,
    WorkArea,
    Exclusive,
}

// ── Theme presets ────────────────────────────────────────────────────────────

/// Snapshot of the theme-relevant `Config` fields. Captured the first
/// time `apply_theme_preset` runs so `mctl theme default` can revert
/// to "what the config file said". Reset to `None` on `mctl reload`
/// so the baseline always tracks the latest parse.
#[derive(Debug, Clone)]
pub(crate) struct ThemeBaseline {
    pub(crate) borderpx: u32,
    pub(crate) border_radius: i32,
    pub(crate) shadows: bool,
    pub(crate) layer_shadows: bool,
    pub(crate) shadow_only_floating: bool,
    pub(crate) shadows_size: u32,
    pub(crate) shadows_blur: f32,
    pub(crate) blur: bool,
    pub(crate) blur_layer: bool,
}

impl ThemeBaseline {
    pub(crate) fn capture(c: &Config) -> Self {
        Self {
            borderpx: c.borderpx,
            border_radius: c.border_radius,
            shadows: c.shadows,
            layer_shadows: c.layer_shadows,
            shadow_only_floating: c.shadow_only_floating,
            shadows_size: c.shadows_size,
            shadows_blur: c.shadows_blur,
            blur: c.blur,
            blur_layer: c.blur_layer,
        }
    }

    pub(crate) fn apply_to(&self, c: &mut Config) {
        c.borderpx = self.borderpx;
        c.border_radius = self.border_radius;
        c.shadows = self.shadows;
        c.layer_shadows = self.layer_shadows;
        c.shadow_only_floating = self.shadow_only_floating;
        c.shadows_size = self.shadows_size;
        c.shadows_blur = self.shadows_blur;
        c.blur = self.blur;
        c.blur_layer = self.blur_layer;
    }
}

#[cfg(test)]
mod theme_baseline_tests {
    use super::*;

    #[test]
    fn round_trip_preserves_every_captured_field() {
        let mut c = Config::default();
        c.borderpx = 3;
        c.border_radius = 8;
        c.shadows = true;
        c.layer_shadows = true;
        c.shadow_only_floating = true;
        c.shadows_size = 22;
        c.shadows_blur = 14.0;
        c.blur = true;
        c.blur_layer = false;

        let baseline = ThemeBaseline::capture(&c);

        // Stomp every field with a different value.
        c.borderpx = 1;
        c.border_radius = 0;
        c.shadows = false;
        c.layer_shadows = false;
        c.shadow_only_floating = false;
        c.shadows_size = 0;
        c.shadows_blur = 0.0;
        c.blur = false;
        c.blur_layer = true;

        baseline.apply_to(&mut c);

        assert_eq!(c.borderpx, 3);
        assert_eq!(c.border_radius, 8);
        assert!(c.shadows);
        assert!(c.layer_shadows);
        assert!(c.shadow_only_floating);
        assert_eq!(c.shadows_size, 22);
        assert!((c.shadows_blur - 14.0).abs() < f32::EPSILON);
        assert!(c.blur);
        assert!(!c.blur_layer);
    }
}

// ── Window-rule reapply trigger ──────────────────────────────────────────────

/// Tags the three sites that drive a post-mount window-rule reapply.
/// Lets [`MargoState::reapply_rules`] log the trigger reason and gives
/// future per-trigger policy (e.g. "don't move clients on `Reload`,
/// only on `InitialMap`") a single place to live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowRuleReason {
    /// Initial XDG toplevel map — `finalize_initial_map` runs rules
    /// once after `app_id` and `title` settle.
    InitialMap,
    /// Late `app_id` (or title) change after the initial commit.
    /// Browsers + Electron apps frequently set their own app_id
    /// asynchronously after the first frame; rules keyed on app_id
    /// have to re-run when the value finally lands.
    AppIdSettled,
    /// Config reload — rules pulled from the new `Config` get
    /// reapplied to every existing client. Conservative trigger:
    /// runs even when the rule set didn't change, so `mctl reload`
    /// is always idempotent.
    Reload,
}

// ── Focus target ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum FocusTarget {
    Window(Window),
    LayerSurface(WlrLayerSurface),
    SessionLock(LockSurface),
    /// XDG popup that grabbed input. We don't track the
    /// `desktop::PopupKind` itself because that wrapper would force a
    /// dependency cycle with the popup manager; the bare wl_surface is
    /// enough — it's what `KeyboardTarget`'s default impl forwards
    /// keys to.
    Popup(WlSurface),
}

impl smithay::utils::IsAlive for FocusTarget {
    fn alive(&self) -> bool {
        match self {
            FocusTarget::Window(w) => w.alive(),
            FocusTarget::LayerSurface(l) => l.alive(),
            FocusTarget::SessionLock(s) => s.alive(),
            FocusTarget::Popup(s) => s.alive(),
        }
    }
}

impl WaylandFocus for FocusTarget {
    fn wl_surface(&self) -> Option<std::borrow::Cow<'_, WlSurface>> {
        match self {
            FocusTarget::Window(w) => w.wl_surface(),
            FocusTarget::LayerSurface(l) => Some(std::borrow::Cow::Borrowed(l.wl_surface())),
            FocusTarget::SessionLock(s) => Some(std::borrow::Cow::Borrowed(s.wl_surface())),
            FocusTarget::Popup(s) => Some(std::borrow::Cow::Borrowed(s)),
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
            Self::Popup(s) => Some(s),
        }
    }
}

// `PopupManager::grab_popup` requires the seat's `KeyboardFocus` to
// be `From<PopupKind>` so margo can hand a popup wl_surface in as the
// grab target. Wrap the popup's wl_surface in a `FocusTarget::Popup`.
impl From<smithay::desktop::PopupKind> for FocusTarget {
    fn from(kind: smithay::desktop::PopupKind) -> Self {
        FocusTarget::Popup(kind.wl_surface().clone())
    }
}

// `PopupManager::grab_popup` also requires the seat's `PointerFocus`
// (`WlSurface` for margo) to be `From<KeyboardFocus>`. The standard
// extraction works for every variant — every FocusTarget kind maps
// to exactly one wl_surface (toplevels carry one in their underlying
// WindowSurface; layer / lock / popup expose one directly). The
// only edge case is a Window with an X11 surface, which has NO
// wl_surface — in practice grab_popup is never called against an
// X11 window root (only XDG toplevels open xdg_popups), so the
// expect is a never-fire diagnostic rather than an assertion that
// could panic in a real session.
impl From<FocusTarget> for WlSurface {
    fn from(target: FocusTarget) -> Self {
        match target {
            FocusTarget::Window(w) => w
                .wl_surface()
                .map(|cow| cow.into_owned())
                .expect("FocusTarget::Window passed to grab_popup must have a wl_surface (i.e. be a Wayland toplevel, not X11)"),
            FocusTarget::Popup(s) => s,
            FocusTarget::LayerSurface(l) => l.wl_surface().clone(),
            FocusTarget::SessionLock(s) => s.wl_surface().clone(),
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
    /// Whether the client is in any fullscreen mode. Kept as a bool
    /// for backward-compat with existing callsites; the *kind* of
    /// fullscreen lives in [`fullscreen_mode`]. Setters keep the two
    /// fields in lock-step (`is_fullscreen == fullscreen_mode != Off`).
    pub is_fullscreen: bool,
    /// Which fullscreen mode the client is currently in:
    ///
    /// * `Off`       — not fullscreen.
    /// * `WorkArea`  — pencere bar exclusion zone'una kadar büyür;
    ///                 bar (top / overlay layer-shell) görünür kalır.
    ///                 Default `togglefullscreen` action.
    /// * `Exclusive` — pencere `monitor_area`'nın tamamını kaplar;
    ///                 render path o output'taki layer-shell yüzeylerini
    ///                 suppress eder, bar gizlenir. Triggered by
    ///                 `togglefullscreen_exclusive`.
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
    /// of `bordercolor` for the duration so the user gets a clear
    /// visual cue about which thumbnail a click would activate.
    /// Toggled by `handle_pointer_motion` while `is_overview_open()`,
    /// cleared on overview exit.
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
    /// True if any of this client's surfaces was selected for direct
    /// scan-out on the most recent successful render frame (zero-copy
    /// to a primary or overlay plane). Refreshed by
    /// `update_client_scanout_flags` after each `render_frame`. Surfaces
    /// in `RenderElementPresentationState::ZeroCopy` set this to true;
    /// composited or skipped paths leave it false. Exposed via
    /// `mctl status --json` so users can verify "yes, this fullscreen
    /// mpv is on the primary plane" without reading frame logs.
    pub last_scanout: bool,
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
        // unmapped from the scene by `hide_scratchpad_client` but
        // remain in `clients` (so the next toggle press picks the
        // same instance up). Without this exclusion every
        // subsequent `arrange_monitor` would walk visible_in_pass,
        // see the scratchpad's tag still matches the active tagset,
        // and `map_element` it right back onto the screen — that's
        // exactly the user-visible "tekrar basıyorum kaybolmuyor"
        // bug. is_visible_on is the single chokepoint every layout
        // / focus / IPC path goes through, so guarding it here
        // keeps the rest of the codebase from each having to learn
        // about the scratchpad show flag.
        if self.is_in_scratchpad && !self.is_scratchpad_show {
            return false;
        }
        self.monitor == mon && (self.tags & tagset) != 0
    }
}

/// Whether a `LayerRule` applies to a given layer-shell namespace.
/// Empty `layer_name` patterns match every namespace (so a rule with
/// no `layer_name:` filter applies globally — matches mango's
/// behaviour). Anything else is treated as a regex pattern via
/// [`matches_rule_text`] so the user's existing
/// `layer_name:^(rofi|fuzzel|launcher).*` style rules carry over.
fn matches_layer_name(rule: &margo_config::LayerRule, namespace: &str) -> bool {
    rule.layer_name
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| matches_rule_text(p, namespace))
        .unwrap_or(true)
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
    /// Last N focused-client indices for this monitor (MRU order,
    /// most recent first). Populated by `focus_surface` whenever
    /// focus shifts to a client living on this output. Capped at
    /// `FOCUS_HISTORY_DEPTH` so it stays bounded across long
    /// sessions. Exposed in state.json as `focus_history` (a list
    /// of app_id strings) for MRU widgets / dock icons.
    pub focus_history: std::collections::VecDeque<usize>,
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
    let _span = tracy_client::span!("tick_animations");
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
    /// at runtime. Disable still rejected; mode changes are now
    /// queued via `pending_output_mode_changes` for the udev
    /// backend to drain at the next repaint.
    pub output_management_state:
        crate::protocols::output_management::OutputManagementManagerState,
    /// Mode changes accepted by `apply_output_pending` but not yet
    /// applied at the DRM layer. The udev repaint handler drains
    /// this and feeds each entry through `DrmCompositor::use_mode`,
    /// then updates the smithay `Output` state so wl_output mode
    /// events fire for any client (kanshi watcher, status bar).
    /// Held outside the apply path because the handler runs on
    /// MargoState and doesn't have a borrow on the udev BackendData.
    pub pending_output_mode_changes: Vec<crate::PendingOutputModeChange>,
    /// `wp_color_management_v1` (staging) — Phase 1 scaffolding.
    /// The global is registered so Chromium / mpv probe-detection
    /// finds a colour-managed compositor and lights up their HDR
    /// decode paths. Composite stays sRGB; per-surface descriptions
    /// are stored on the surface tracker, not yet read by render.
    /// Phase 2 (linear-light fp16 composite) consumes these.
    pub color_management_state: crate::protocols::color_management::ColorManagementState,
    /// User-script engine + compiled AST + registered event hooks.
    /// `None` if no `~/.config/margo/init.rhai` is present. Boxed so
    /// the field is small + we can `Option::take()` it during hook
    /// invocation (the recursion guard + borrow-checker dance lives
    /// in `scripting::fire_hook`).
    pub scripting: Option<Box<crate::scripting::ScriptingState>>,
    /// Active screencasting state — PipeWire core, list of running
    /// casts, dynamic-cast queue. `None` until xdp-gnome opens its
    /// first ScreenCast session and the lazy PipeWire init runs;
    /// margo's compositor process otherwise pays no PipeWire cost.
    /// Gated on the `xdp-gnome-screencast` feature so distro builds
    /// without screencast support drop the entire PipeWire dep tree.
    #[cfg(feature = "xdp-gnome-screencast")]
    pub screencasting: Option<Box<crate::screencasting::Screencasting>>,
    /// D-Bus shim connections so xdp-gnome can serve the
    /// ScreenCast / Screenshot / Mutter portals on margo without
    /// gnome-shell. See `crate::dbus`. Set once at startup;
    /// connections close when the field drops (compositor exit).
    /// Gated on the `dbus` feature.
    #[cfg(feature = "dbus")]
    pub dbus_servers: crate::dbus::DBusServers,
    /// Shared snapshot of monitors used by the Mutter D-Bus shims
    /// (`DisplayConfig` + `ScreenCast`). The same Arc handed to
    /// both services so a hotplug-driven `refresh_ipc_outputs()`
    /// updates both views at once. Previously each service got
    /// its own `ipc_output::snapshot(&margo)` at startup and
    /// neither refreshed — a monitor unplugged mid-cast left
    /// xdp-gnome's chooser dialog still listing the gone output.
    /// Lazy in the sense that we only re-snapshot when the
    /// monitor list actually changes, not every frame.
    #[cfg(feature = "dbus")]
    pub ipc_outputs: std::sync::Arc<std::sync::Mutex<crate::dbus::ipc_output::IpcOutputMap>>,
    /// GBM device the udev backend opened for buffer allocation.
    /// Populated at backend init; D-Bus / screencast threads pull
    /// it for `Cast::new` to allocate dmabuf-backed PipeWire
    /// buffers without re-opening the DRM node. `None` outside
    /// the udev backend (winit nested mode).
    pub cast_gbm: Option<smithay::backend::allocator::gbm::GbmDevice<smithay::backend::drm::DrmDeviceFd>>,
    /// Renderer-side dmabuf format constraints, snapshotted at
    /// backend init so the screencast cast lifecycle has them
    /// without crossing the borrow boundary into the udev
    /// renderer mid-D-Bus-call.
    pub cast_render_formats: smithay::backend::allocator::format::FormatSet,
    /// `ext-image-capture-source-v1` core state. Mints opaque
    /// source handles that clients pass to ext-image-copy-capture
    /// to identify what they want to capture. xdp-wlr 0.8+ uses
    /// these for the per-window screencast path.
    pub image_capture_source_state:
        smithay::wayland::image_capture_source::ImageCaptureSourceState,
    /// `ext-output-image-capture-source-manager-v1` global —
    /// "give me a capture source for this wl_output". Backs the
    /// monitor-share path in xdp-wlr.
    pub output_capture_source_state:
        smithay::wayland::image_capture_source::OutputCaptureSourceState,
    /// `ext-foreign-toplevel-image-capture-source-manager-v1`
    /// global — "give me a capture source for this toplevel".
    /// Margo's `ForeignToplevelListState` already implements the
    /// matching `ext-foreign-toplevel-list-v1`, so xdp-wlr can
    /// enumerate windows + ask for per-window capture; this is
    /// the protocol that lights up the **Window tab** in
    /// browser-based meeting clients (Google Meet, Zoom Web,
    /// Discord, Jitsi).
    pub toplevel_capture_source_state:
        smithay::wayland::image_capture_source::ToplevelCaptureSourceState,
    /// `ext-image-copy-capture-v1` — the actual capture transport.
    /// Clients open a session against an `ImageCaptureSource`,
    /// receive buffer constraints, allocate a matching buffer,
    /// then request a frame which margo renders into the buffer.
    pub image_copy_capture_state:
        smithay::wayland::image_copy_capture::ImageCopyCaptureState,
    /// Active capture sessions, keyed by something we can match
    /// against an `ImageCaptureSource` later — for now we hold
    /// the `Session` handles so they don't get dropped (which
    /// would auto-stop the session). Real frame routing wires
    /// up in the rendering follow-up commit.
    pub image_copy_capture_sessions: Vec<smithay::wayland::image_copy_capture::Session>,
    /// Frames awaiting their backing source's content. The udev
    /// repaint handler drains this list after every render and
    /// fills each frame's buffer from the matching output (or
    /// fails the frame if the source has gone stale). Stored as
    /// `(session_ref, frame, source_kind)` so we can route
    /// without re-querying user_data on each iteration.
    pub pending_image_copy_frames: Vec<crate::PendingImageCopyFrame>,
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
    /// Snapshot of theme-relevant `Config` fields captured the first
    /// time `apply_theme_preset` runs. `Theme::Default` resets to
    /// this snapshot; the snapshot is also reset on `mctl reload` so
    /// "default" always means "what config.conf says today".
    pub(crate) theme_baseline: Option<ThemeBaseline>,
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
    /// Discovered Rhai plugins (W3.3). Empty when no
    /// `~/.config/margo/plugins/` exists; populated by
    /// `init_plugins` after init_user_scripting. Stored on state
    /// so `mctl plugin list` (future) can enumerate without
    /// re-walking the FS.
    pub plugins: Vec<crate::plugin::Plugin>,
    /// AccessKit accessibility-tree adapter (W2.4). `start()` is
    /// called once at compositor init; subsequent
    /// `publish_window_list` calls flush a fresh tree on every
    /// arrange + focus change. Off by default — only built when
    /// the `a11y` feature is on.
    #[cfg(feature = "a11y")]
    pub a11y: crate::a11y::A11yState,
    /// Active region-selection UI for the in-compositor screenshot
    /// flow (W2.1). `Some(...)` while the user is dragging /
    /// pondering a rect; cleared on Escape, on confirm (after
    /// spawning mscreenshot with `MARGO_REGION_GEOM`), and on
    /// session-lock (so the selector doesn't leak across login
    /// boundaries). Render path overlays the rect; input path
    /// intercepts pointer + keyboard while this is `Some`.
    pub region_selector: Option<crate::screenshot_region::ActiveRegionSelector>,
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
    /// Per-arrange override for the move-animation duration (in ms). Set
    /// by `open_overview` / `close_overview` so the overview transition
    /// uses a snappy ~180 ms slide instead of the full
    /// `animation_duration_move`. `arrange_monitor` reads this and
    /// `open_overview` / `close_overview` clear it after their batched
    /// arrange is done. None ⇒ fall back to the configured duration.
    pub overview_transition_animation_ms: Option<u32>,

    /// Diagnostics from the most recent `reload_config` validation pass.
    /// Empty when the last reload was clean (or no reload has happened
    /// yet). Populated by `reload_config` after running
    /// `margo_config::validator::validate_config`. Queryable from
    /// userspace via `mctl config-errors`. The compositor keeps its
    /// previous config when `has_errors()` is true, so this field
    /// doubles as "why did the last reload not apply?".
    pub last_reload_diagnostics: Vec<margo_config::diagnostics::ConfigDiagnostic>,
    /// `Instant` the config-error overlay first appeared on screen.
    /// Cleared on a clean reload or after the banner's display
    /// window expires (driven by `tick_animations`). Drives the
    /// niri-style red-bordered banner pinned to the active output's
    /// top-right corner.
    pub config_error_overlay_until: Option<std::time::Instant>,
    /// Persistent SolidColorBuffers backing the config-error banner.
    /// Kept on `MargoState` (rather than allocated per-frame) so the
    /// buffers' Ids stay stable across frames and damage tracking
    /// stays tight.
    pub config_error_overlay: crate::render::config_error_overlay::ConfigErrorOverlay,

    /// Alt+Tab muscle-memory: when an `overview_focus_next/prev` keybind
    /// fires, the input handler snapshots which modifier(s) the user is
    /// holding and sets `overview_cycle_pending = true`. On the next key
    /// release event whose modifier state no longer overlaps that snapshot
    /// (i.e. the user let go of Alt/Super/whatever they were holding),
    /// the input handler calls `overview_activate` to commit the cycle's
    /// pick — closing overview onto the highlighted thumbnail. This is
    /// the standard Win/GNOME/Hypr "hold modifier, tap Tab to cycle,
    /// release modifier to confirm" behaviour. Cleared by
    /// `overview_activate`, `close_overview`, and `open_overview`.
    pub overview_cycle_pending: bool,
    pub overview_cycle_modifier_mask: margo_config::Modifiers,

    /// Which hot corner the pointer is currently dwelling in (if any).
    /// `None` while pointer is anywhere else; set on entry, cleared on
    /// exit. Together with [`hot_corner_armed_at`] drives the dwell
    /// threshold before the corner's action fires.
    pub hot_corner_dwelling: Option<HotCorner>,
    /// `Instant` the pointer entered the current dwell corner. The
    /// dwell threshold (`Config::hot_corner_dwell_ms`) is checked in
    /// the same `pointer_motion` handler that sets / clears
    /// `hot_corner_dwelling`. Cleared together with `hot_corner_dwelling`.
    pub hot_corner_armed_at: Option<std::time::Instant>,
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
        // wp_color_management_v1 (staging) — Phase 1 scaffolding.
        // Standing the global up early lets HDR-aware clients
        // (Chromium, mpv) detect "this compositor speaks colour
        // management" and enable their decode paths even though
        // composite is still SDR. See `protocols/color_management.rs`
        // and `docs/hdr-design.md` for the four-phase rollout.
        let color_management_state =
            crate::protocols::color_management::ColorManagementState::new::<Self, _>(
                &dh,
                |_client| true,
            );
        // ext-image-capture-source-v1 + ext-image-copy-capture-v1
        // — the modern Wayland screencast stack. Without these
        // globals, xdp-wlr 0.8+ can't expose per-window share
        // (Window tab in meeting clients). Smithay ships full
        // helpers; output and toplevel source globals are
        // independent so we can advertise both.
        let image_capture_source_state =
            smithay::wayland::image_capture_source::ImageCaptureSourceState::new();
        let output_capture_source_state =
            smithay::wayland::image_capture_source::OutputCaptureSourceState::new::<Self>(&dh);
        let toplevel_capture_source_state =
            smithay::wayland::image_capture_source::ToplevelCaptureSourceState::new::<Self>(&dh);
        let image_copy_capture_state =
            smithay::wayland::image_copy_capture::ImageCopyCaptureState::new::<Self>(&dh);
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
            pending_output_mode_changes: Vec::new(),
            color_management_state,
            scripting: None,
            #[cfg(feature = "xdp-gnome-screencast")]
            screencasting: None,
            #[cfg(feature = "dbus")]
            dbus_servers: crate::dbus::DBusServers::default(),
            #[cfg(feature = "dbus")]
            ipc_outputs: std::sync::Arc::new(std::sync::Mutex::new(
                crate::dbus::ipc_output::IpcOutputMap::new(),
            )),
            cast_gbm: None,
            cast_render_formats: Default::default(),
            image_capture_source_state,
            output_capture_source_state,
            toplevel_capture_source_state,
            image_copy_capture_state,
            image_copy_capture_sessions: Vec::new(),
            pending_image_copy_frames: Vec::new(),
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
            plugins: Vec::new(),
            #[cfg(feature = "a11y")]
            a11y: crate::a11y::A11yState::new(),
            region_selector: None,
            layer_animations: std::collections::HashMap::new(),
            overview_transition_animation_ms: None,
            last_reload_diagnostics: Vec::new(),
            config_error_overlay_until: None,
            config_error_overlay:
                crate::render::config_error_overlay::ConfigErrorOverlay::new(),
            overview_cycle_pending: false,
            overview_cycle_modifier_mask: margo_config::Modifiers::empty(),
            hot_corner_dwelling: None,
            hot_corner_armed_at: None,
            config,
            theme_baseline: None,
        }
    }

    /// Rebuild the wlr-output-management snapshot from the current
    /// monitor list and publish it to all bound clients (kanshi,
    /// wlr-randr, way-displays, …). Cheap when nothing's changed:
    /// `snapshot_changed` early-returns on equal snapshots.
    /// Path of the runtime state-file used by `mctl clients` /
    /// `mctl outputs`. Public so dispatch handlers can also
    /// trigger a write after non-arrange state changes.
    pub fn refresh_state_file(&self) {
        self.write_state_file();
    }

    /// Open the in-compositor region selector at the current cursor
    /// position. Replaces the previous "spawn slurp via mscreenshot"
    /// flow — the selector lives entirely inside margo's render +
    /// input loops so there's no second window fighting focus, no
    /// IPC round-trip, no stale-frame artifacts. Subsequent pointer
    /// button + motion + key events route through the selector
    /// until [`Self::confirm_region_selection`] or
    /// [`Self::cancel_region_selection`] runs.
    pub fn open_region_selector(
        &mut self,
        mode: crate::screenshot_region::SelectorMode,
    ) {
        let cursor = (self.input_pointer.x, self.input_pointer.y);
        self.region_selector = Some(
            crate::screenshot_region::ActiveRegionSelector::at(cursor, mode),
        );
        self.request_repaint();
        tracing::info!(
            "region selector opened at ({:.0}, {:.0}) mode={:?}",
            cursor.0,
            cursor.1,
            mode
        );
    }

    /// User pressed Enter / released the drag button — finalize
    /// the selection: spawn `mscreenshot <mode>` with
    /// `MARGO_REGION_GEOM` set, then close the selector. Re-arms
    /// (keeps the selector open) if the selection is degenerate
    /// — user clicked but didn't drag. Caller decides whether to
    /// route Enter through this immediately or wait for a real
    /// drag.
    pub fn confirm_region_selection(&mut self) {
        let Some(sel) = self.region_selector.take() else {
            return;
        };
        let Some(geom) = sel.geom_string() else {
            // Degenerate rect — re-arm so user can try again.
            self.region_selector = Some(sel);
            return;
        };
        let mode = sel.mode.subcommand();
        let cmd = format!("MARGO_REGION_GEOM='{}' mscreenshot {}", geom, mode);
        tracing::info!("region selector confirm: {cmd}");
        if let Err(e) = crate::utils::spawn_shell(&cmd) {
            tracing::error!("spawn mscreenshot: {e}");
        }
        self.request_repaint();
    }

    /// User pressed Escape — drop the selector without spawning
    /// mscreenshot.
    pub fn cancel_region_selection(&mut self) {
        if self.region_selector.take().is_some() {
            tracing::info!("region selector cancelled");
            self.request_repaint();
        }
    }

    /// Soft-disable a monitor: mark it inactive, migrate every client
    /// to the first remaining enabled monitor, and clear focus from it.
    /// Render and arrange paths skip disabled monitors so the panel
    /// stops getting dirty repaints; the underlying smithay `Output`
    /// stays alive so a later `enable_monitor` call can restore it
    /// without a full hotplug round-trip. Pertag state survives across
    /// the cycle.
    ///
    /// Note: the DRM connector is NOT powered off here — that needs
    /// the udev backend's DrmCompositor handle, plumbed separately.
    /// What this fixes: the wlr-output-management protocol-level
    /// "disable" request now succeeds, kanshi profiles that toggle
    /// outputs flip cleanly, and the bar / state file see the right
    /// active-output set. Power-off of the panel is a follow-up.
    pub fn disable_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        if !self.monitors[mon_idx].enabled {
            return;
        }
        // Pick a migration target — first OTHER enabled monitor.
        let target = (0..self.monitors.len())
            .find(|&i| i != mon_idx && self.monitors[i].enabled);
        let Some(target) = target else {
            tracing::warn!(
                "disable_monitor: refusing to disable {} — no other enabled monitor",
                self.monitors[mon_idx].name
            );
            return;
        };
        let target_tagset = self.monitors[target].current_tagset();
        let target_name = self.monitors[target].name.clone();
        let src_name = self.monitors[mon_idx].name.clone();

        // Migrate every client living on the doomed monitor.
        for c in self.clients.iter_mut() {
            if c.monitor == mon_idx {
                c.monitor = target;
                // Pull onto an active tag of the new home so the
                // client doesn't vanish into a hidden tagset.
                if c.tags & target_tagset == 0 {
                    c.tags = target_tagset;
                }
            }
        }
        // Clear focus history that points at the disabled monitor.
        if self.focused_monitor() == mon_idx {
            for mon in &mut self.monitors {
                mon.selected = None;
            }
        }
        self.monitors[mon_idx].enabled = false;
        self.arrange_monitor(target);
        self.focus_first_visible_or_clear(target);
        self.publish_output_topology();
        self.write_state_file();
        tracing::info!("disabled output {src_name} → migrated clients to {target_name}");
    }

    /// Re-enable a previously soft-disabled monitor. New windows can
    /// land on it again; arrange picks it up; render starts drawing
    /// it on the next frame.
    pub fn enable_monitor(&mut self, mon_idx: usize) {
        if mon_idx >= self.monitors.len() {
            return;
        }
        if self.monitors[mon_idx].enabled {
            return;
        }
        self.monitors[mon_idx].enabled = true;
        self.arrange_monitor(mon_idx);
        self.publish_output_topology();
        self.write_state_file();
        tracing::info!("re-enabled output {}", self.monitors[mon_idx].name);
    }

    /// Notify xdp-gnome's window picker that the toplevel set changed
    /// so a live screencast share dialog refreshes its list. Fires
    /// the `org.gnome.Shell.Introspect.WindowsChanged` D-Bus signal
    /// against the registered `Introspect` interface. Cheap no-op if
    /// the D-Bus shim isn't running (no screencast portal use).
    /// On builds without the `dbus` feature this is a literal no-op
    /// — the call sites stay regardless so the rest of the
    /// codebase doesn't have to learn about the feature flag.
    pub fn emit_windows_changed(&self) {
        #[cfg(feature = "dbus")]
        if let Some(conn) = &self.dbus_servers.conn_introspect {
            crate::dbus::gnome_shell_introspect::emit_windows_changed_sync(conn);
        }
    }

    /// Re-build the shared `ipc_outputs` snapshot from the live
    /// `monitors` list. No-op without the `dbus` feature. Called
    /// from `remove_output` (hotplug-out) and from the udev
    /// backend's `setup_connector` (hotplug-in) so xdp-gnome's
    /// chooser dialog always reflects the actual output set —
    /// without this, a monitor unplugged mid-cast would still
    /// appear in the Entire Screen tab.
    #[cfg(feature = "dbus")]
    pub fn refresh_ipc_outputs(&self) {
        let snap = crate::dbus::ipc_output::snapshot(self);
        if let Ok(mut guard) = self.ipc_outputs.lock() {
            *guard = snap;
        }
    }

    /// No-op stub when dbus is off. Lets call sites in udev /
    /// state stay un-cfg-gated.
    #[cfg(not(feature = "dbus"))]
    pub fn refresh_ipc_outputs(&self) {}

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
        // Hotplug-out: refresh the shared D-Bus snapshot so
        // xdp-gnome's chooser dialog drops the now-gone output.
        self.refresh_ipc_outputs();
        self.request_repaint();
    }

    pub fn arrange_all(&mut self) {
        for mon_idx in 0..self.monitors.len() {
            self.arrange_monitor(mon_idx);
        }
        self.request_repaint();
        self.write_state_file();
        self.publish_a11y_window_list();
    }

    /// Arrange just the listed monitors. Used by `open_overview` and
    /// `close_overview` so a multi-monitor setup doesn't pay the cost
    /// of re-laying out outputs that didn't flip overview state. Skips
    /// out-of-range indices defensively — the caller is the same
    /// process that built the list, but `monitors` can shrink under us
    /// during multi-output hot-unplug and we don't want to panic mid-
    /// arrange.
    pub fn arrange_monitors(&mut self, indices: &[usize]) {
        for &idx in indices {
            if idx < self.monitors.len() {
                self.arrange_monitor(idx);
            }
        }
        self.request_repaint();
        self.write_state_file();
        self.publish_a11y_window_list();
    }

    /// Snapshot the client list and ship it to the AccessKit
    /// adapter so screen readers see the current toplevels +
    /// focus state. No-op without the `a11y` feature.
    #[cfg(feature = "a11y")]
    pub fn publish_a11y_window_list(&mut self) {
        let focused_idx = self.focused_client_idx();
        let snapshot: Vec<crate::a11y::WindowSnapshot> = self
            .clients
            .iter()
            .enumerate()
            .map(|(i, c)| crate::a11y::WindowSnapshot {
                app_id: c.app_id.clone(),
                title: c.title.clone(),
                is_focused: Some(i) == focused_idx,
            })
            .collect();
        self.a11y.publish_window_list(snapshot.iter());
    }

    /// Stub on builds without the `a11y` feature so call sites
    /// don't have to learn the feature flag.
    #[cfg(not(feature = "a11y"))]
    pub fn publish_a11y_window_list(&mut self) {}

    /// Serialise the current state — outputs, clients, layouts —
    /// to `$XDG_RUNTIME_DIR/margo/state.json` (atomic rename).
    /// Read by `mctl clients` / `mctl outputs` / the
    /// improved `mctl status` so they can list richer info than
    /// what fits in the wire-level dwl-ipc-v2 events.
    ///
    /// Best-effort: failures are logged at debug level, never
    /// surfaced to the user — the file is a side-channel for
    /// tooling, not a hard correctness requirement.
    pub fn write_state_file(&self) {
        let path = state_file_path();
        if let Err(err) = self.write_state_file_inner(&path) {
            tracing::debug!("write_state_file({}): {err}", path.display());
        }
    }

    fn write_state_file_inner(&self, path: &std::path::Path) -> anyhow::Result<()> {
        use std::io::Write as _;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let payload = self.build_state_snapshot();
        let json = serde_json::to_string(&payload)?;

        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    fn build_state_snapshot(&self) -> serde_json::Value {
        use serde_json::json;

        let focused_idx = self.focused_client_idx();
        let outputs: Vec<_> = self
            .monitors
            .iter()
            .enumerate()
            .map(|(i, mon)| {
                let mode = mon.output.current_mode();
                let phys_w = mode.map(|m| m.size.w).unwrap_or(0);
                let phys_h = mode.map(|m| m.size.h).unwrap_or(0);
                let refresh = mode.map(|m| m.refresh).unwrap_or(0);
                let active_tag = mon.tagset[mon.seltags];
                let prev_tag = mon.tagset[mon.seltags ^ 1];
                let active_output = focused_idx
                    .and_then(|fc| self.clients.get(fc))
                    .map(|c| c.monitor == i)
                    .unwrap_or(false);
                json!({
                    "name": mon.name,
                    "active": active_output,
                    "x": mon.monitor_area.x,
                    "y": mon.monitor_area.y,
                    "width": mon.monitor_area.width,
                    "height": mon.monitor_area.height,
                    "scale": mon.scale,
                    "transform": mon.transform,
                    "mode": {
                        "physical_width": phys_w,
                        "physical_height": phys_h,
                        "refresh_mhz": refresh,
                    },
                    "layout_idx": mon.pertag.ltidxs[mon.pertag.curtag] as u32,
                    "active_tag_mask": active_tag,
                    "prev_tag_mask": prev_tag,
                    "occupied_tag_mask": self.clients.iter()
                        .filter(|c| c.monitor == i)
                        .fold(0u32, |a, c| a | c.tags),
                    "is_overview": mon.is_overview,
                    // W3.6: per-tag wallpaper hint of the *active*
                    // tag. Wallpaper daemons watching state.json
                    // can swap on tag change. Empty string = "use
                    // session default". Per-tag map is in
                    // `wallpapers_by_tag` below for daemons that
                    // want to pre-cache.
                    "wallpaper": mon.pertag.wallpapers
                        .get(mon.pertag.curtag).cloned().unwrap_or_default(),
                    "wallpapers_by_tag": (1..=crate::MAX_TAGS)
                        .map(|t| mon.pertag.wallpapers
                            .get(t).cloned().unwrap_or_default())
                        .collect::<Vec<_>>(),
                    // W3.4: scratchpad summary (counts of visible /
                    // hidden) and per-monitor focus history (MRU
                    // app_ids, most recent first). MRU widgets and
                    // dock indicators read these to render counts +
                    // recently-used app rings.
                    "scratchpad_visible": self.clients.iter()
                        .filter(|c| c.monitor == i
                            && c.is_in_scratchpad
                            && c.is_scratchpad_show)
                        .count(),
                    "scratchpad_hidden": self.clients.iter()
                        .filter(|c| c.monitor == i
                            && c.is_in_scratchpad
                            && !c.is_scratchpad_show)
                        .count(),
                    "focus_history": mon.focus_history.iter()
                        .filter_map(|&idx| self.clients.get(idx))
                        .map(|c| c.app_id.clone())
                        .collect::<Vec<_>>(),
                })
            })
            .collect();

        let clients: Vec<_> = self
            .clients
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                let mon_name = self.monitors.get(c.monitor)
                    .map(|m| m.name.clone()).unwrap_or_default();
                json!({
                    "idx": idx,
                    "monitor": mon_name,
                    "monitor_idx": c.monitor,
                    "tags": c.tags,
                    "app_id": c.app_id,
                    "title": c.title,
                    "x": c.geom.x,
                    "y": c.geom.y,
                    "width": c.geom.width,
                    "height": c.geom.height,
                    "floating": c.is_floating,
                    "fullscreen": c.is_fullscreen,
                    "minimized": c.is_minimized,
                    "urgent": c.is_urgent,
                    "scratchpad": c.is_in_scratchpad,
                    "global": c.is_global,
                    "focused": Some(idx) == focused_idx,
                    "pid": c.pid,
                    "scanout": c.last_scanout,
                })
            })
            .collect();

        // Mirror dwl-ipc's layouts list — same set the live status
        // bar shows.
        let all_layouts = [
            crate::layout::LayoutId::Tile,
            crate::layout::LayoutId::Scroller,
            crate::layout::LayoutId::Grid,
            crate::layout::LayoutId::Monocle,
            crate::layout::LayoutId::Deck,
            crate::layout::LayoutId::CenterTile,
            crate::layout::LayoutId::RightTile,
            crate::layout::LayoutId::VerticalScroller,
            crate::layout::LayoutId::VerticalTile,
            crate::layout::LayoutId::VerticalGrid,
            crate::layout::LayoutId::VerticalDeck,
            crate::layout::LayoutId::TgMix,
            crate::layout::LayoutId::Canvas,
            crate::layout::LayoutId::Dwindle,
        ];
        let layout_names: Vec<_> = all_layouts
            .iter()
            .map(|l| serde_json::Value::String(l.name().to_string()))
            .collect();

        // Active output: the one the focused client is on, else the
        // first monitor.
        let active_output = focused_idx
            .and_then(|idx| self.clients.get(idx))
            .and_then(|c| self.monitors.get(c.monitor))
            .map(|m| m.name.clone())
            .or_else(|| self.monitors.first().map(|m| m.name.clone()))
            .unwrap_or_default();

        // Diagnostics from the most recent reload (or initial parse).
        // Exposed in state.json so `mctl config-errors` can fetch
        // them without a dedicated IPC roundtrip.
        let config_errors: Vec<_> = self
            .last_reload_diagnostics
            .iter()
            .map(|d| {
                json!({
                    "path": d.path.display().to_string(),
                    "line": d.line,
                    "col": d.col,
                    "end_col": d.end_col,
                    "severity": match d.severity {
                        margo_config::diagnostics::Severity::Error => "error",
                        margo_config::diagnostics::Severity::Warning => "warning",
                    },
                    "code": d.code,
                    "message": d.message,
                    "line_text": d.line_text,
                })
            })
            .collect();

        json!({
            "version": 1,
            "tag_count": MAX_TAGS,
            "active_output": active_output,
            "focused_idx": focused_idx,
            "outputs": outputs,
            "clients": clients,
            "layouts": layout_names,
            "config_errors": config_errors,
        })
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
        let _ = crate::utils::spawn([
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
    /// Drain a PipeWire-side message into the compositor side.
    /// Mirrors niri's `State::on_pw_msg`. Three message types:
    ///
    ///   * `StopCast { session_id }` — tear down the cast plus
    ///     any matching streams.
    ///   * `Redraw { stream_id }` — kick the render path so this
    ///     stream's next frame renders.
    ///   * `FatalError` — PipeWire failed catastrophically; tear
    ///     down everything and let the next session start cleanly.
    #[cfg(feature = "xdp-gnome-screencast")]
    pub fn on_pw_msg(&mut self, msg: crate::screencasting::pw_utils::PwToNiri) {
        use crate::screencasting::pw_utils::PwToNiri;
        match msg {
            PwToNiri::StopCast { session_id } => self.stop_cast(session_id),
            PwToNiri::Redraw { stream_id: _ } => {
                // PipeWire only fires Redraw twice per stream
                // (initial Streaming + first dmabuf); the steady-
                // state cast cadence comes from the udev repaint
                // loop iterating every active cast on every tick.
                // We just wake the loop so the first cast frame
                // lands on the next VBlank instead of waiting on
                // unrelated input.
                self.request_repaint();
            }
            PwToNiri::FatalError => {
                tracing::warn!("stopping screencasting due to PipeWire fatal error");
                if let Some(mut casting) = self.screencasting.take() {
                    let session_ids: Vec<_> =
                        casting.casts.iter().map(|c| c.session_id).collect();
                    casting.casts.clear();
                    casting.pipewire = None;
                    self.screencasting = Some(casting);
                    for id in session_ids {
                        self.stop_cast(id);
                    }
                    self.screencasting = None;
                }
            }
        }
    }

    /// Tear down every cast belonging to the given session. Called
    /// from xdp-gnome's `Session.Stop` D-Bus method (via the
    /// `ScreenCastToCompositor` channel) and from `on_pw_msg` when
    /// PipeWire errors out.
    #[cfg(feature = "xdp-gnome-screencast")]
    pub fn stop_cast(&mut self, session_id: crate::dbus::cast_ids::CastSessionId) {
        let Some(casting) = self.screencasting.as_mut() else {
            return;
        };
        casting.casts.retain(|cast| cast.session_id != session_id);
    }

    /// Start a cast in response to xdp-gnome's `Session.Start`
    /// D-Bus call. Margo equivalent of niri's
    /// `on_screen_cast_msg::StartCast` arm in
    /// `screencasting/mod.rs`.
    ///
    /// Steps:
    ///   1. Resolve the `StreamTargetId` against margo's monitor
    ///      list (output) or client list (toplevel) → produce a
    ///      `CastTarget` + `(size, refresh, alpha)` triple.
    ///   2. Lazy-init `Screencasting` + the PipeWire core if this
    ///      is the first cast of the session.
    ///   3. Call `pw.start_cast(...)` to mint a `Cast`. The cast
    ///      drives PipeWire negotiation; once the format is
    ///      agreed it emits `pipe_wire_stream_added(node_id)` over
    ///      the supplied `signal_ctx` so xdp-gnome / browser can
    ///      open the PipeWire node.
    ///   4. Push the cast onto `casting.casts`. Subsequent frame
    ///      production goes through the udev backend's repaint
    ///      hook (Phase E2 — render integration).
    #[cfg(feature = "xdp-gnome-screencast")]
    pub fn start_cast(
        &mut self,
        session_id: crate::dbus::cast_ids::CastSessionId,
        stream_id: crate::dbus::cast_ids::CastStreamId,
        target: crate::dbus::mutter_screen_cast::StreamTargetId,
        cursor_mode: crate::dbus::mutter_screen_cast::CursorMode,
        signal_ctx: zbus::object_server::SignalEmitter<'static>,
    ) {
        use crate::dbus::mutter_screen_cast::StreamTargetId;
        use crate::screencasting::CastTarget;

        let (target, size, refresh, alpha) = match target {
            StreamTargetId::Output { name } => {
                let Some(mon) = self.monitors.iter().find(|m| m.name == name) else {
                    tracing::warn!("StartCast: requested output {name} is missing");
                    self.stop_cast(session_id);
                    return;
                };
                let Some(mode) = mon.output.current_mode() else {
                    tracing::warn!("StartCast: output {name} has no current mode");
                    self.stop_cast(session_id);
                    return;
                };
                let size = smithay::utils::Size::<i32, smithay::utils::Physical>::from(
                    (mode.size.w, mode.size.h),
                );
                let refresh = mode.refresh as u32;
                let weak = mon.output.downgrade();
                (
                    CastTarget::Output {
                        output: weak,
                        name,
                    },
                    size,
                    refresh,
                    false,
                )
            }
            StreamTargetId::Window { id } => {
                // Match the window-id (we hand out per-client
                // memory addresses cast to u64 from
                // `gnome_shell_introspect`). Look up by re-scanning
                // clients; the address is stable for the duration
                // of the client's life.
                let Some(client) = self
                    .clients
                    .iter()
                    .find(|c| std::ptr::addr_of!(**c) as u64 == id)
                else {
                    tracing::warn!("StartCast: requested window {id} is missing");
                    self.stop_cast(session_id);
                    return;
                };
                let geom = client.geom;
                if geom.width <= 0 || geom.height <= 0 {
                    tracing::warn!("StartCast: window {id} has degenerate geometry");
                    self.stop_cast(session_id);
                    return;
                }
                let size = smithay::utils::Size::<i32, smithay::utils::Physical>::from(
                    (geom.width, geom.height),
                );
                // Use the focused monitor's refresh as a stand-in;
                // PipeWire negotiates an actual pacing later.
                let refresh = self
                    .monitors
                    .get(client.monitor)
                    .and_then(|m| m.output.current_mode())
                    .map(|m| m.refresh as u32)
                    .unwrap_or(60_000);
                (CastTarget::Window { id }, size, refresh, true)
            }
        };

        let Some(gbm) = self.cast_gbm.clone() else {
            tracing::warn!("StartCast: udev GBM device unavailable (winit?)");
            self.stop_cast(session_id);
            return;
        };
        let render_formats = self.cast_render_formats.clone();

        // Lazy-init Screencasting + PipeWire on first cast.
        if self.screencasting.is_none() {
            let casting =
                crate::screencasting::Screencasting::new(&self.loop_handle);
            self.screencasting = Some(Box::new(casting));
        }
        let casting = self.screencasting.as_mut().unwrap();

        if casting.pipewire.is_none() {
            let pw_to_compositor = casting.pw_to_compositor.clone();
            match crate::screencasting::pw_utils::PipeWire::new(
                self.loop_handle.clone(),
                pw_to_compositor,
            ) {
                Ok(pw) => casting.pipewire = Some(pw),
                Err(err) => {
                    tracing::warn!("StartCast: PipeWire init failed: {err:?}");
                    self.stop_cast(session_id);
                    return;
                }
            }
        }
        let pw = casting.pipewire.as_ref().unwrap();

        match pw.start_cast(
            gbm,
            render_formats,
            session_id,
            stream_id,
            target,
            size,
            refresh,
            alpha,
            cursor_mode,
            signal_ctx,
        ) {
            Ok(cast) => {
                casting.casts.push(cast);
                tracing::info!(
                    "StartCast: session={session_id} stream={stream_id} cast pushed"
                );
            }
            Err(err) => {
                tracing::warn!("StartCast: pw.start_cast failed: {err:?}");
                self.stop_cast(session_id);
            }
        }
    }

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
        // Validate first. The parser is permissive (silent defaults
        // on malformed values), so a "successful" parse can still mean
        // the user's intent was misread. Run the structured validator
        // and bail before swapping config if it found errors —
        // compositor stays on the previous good config.
        match margo_config::validator::validate_config(self.config_path.as_deref()) {
            Ok(report) => {
                self.last_reload_diagnostics = report.diagnostics.clone();
                if report.has_errors() {
                    // Trigger the C2 on-screen banner. 10 s ought to be
                    // long enough to read "your config is broken, run
                    // mctl check-config" without being a pest.
                    self.config_error_overlay_until = Some(
                        std::time::Instant::now()
                            + std::time::Duration::from_secs(10),
                    );
                    self.request_repaint();
                    let err_count = report.errors().count();
                    return Err(anyhow::anyhow!(
                        "config has {err_count} error(s) — run `mctl check-config` for details"
                    ));
                }
            }
            Err(e) => {
                tracing::warn!("config validator could not read file: {e}");
                // Fall through: let parse_config produce the canonical
                // error so the caller's message says "I/O failure"
                // rather than "validator missing".
            }
        }

        let new_config = parse_config(self.config_path.as_deref())
            .with_context(|| "reload margo config")?;

        // Successful reload — clear any stale diagnostics + overlay
        // (warnings from the validation pass above are still in
        // last_reload_diagnostics, intentionally; the user can still
        // query them via mctl config-errors).
        self.config_error_overlay_until = None;

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
        // Reload re-establishes "what the file says" — invalidate the
        // theme baseline so a subsequent `mctl theme default` resets
        // to the freshly-parsed values.
        self.theme_baseline = None;
        for idx in 0..self.clients.len() {
            self.reapply_rules(idx, WindowRuleReason::Reload);
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

    pub(crate) fn refresh_output_work_area(&mut self, output: &Output) {
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
        let _span = tracy_client::span!("arrange_monitor");
        if mon_idx >= self.monitors.len() {
            return;
        }
        // Soft-disabled monitor: don't lay out — clients have already
        // been migrated off, and laying out against a panel that isn't
        // being rendered just produces stale geometry.
        if !self.monitors[mon_idx].enabled {
            return;
        }

        // Adaptive layout: when `Config::auto_layout` is on AND the
        // user hasn't explicitly picked a layout for the current tag
        // (`pertag.user_picked_layout[curtag]` sticky bit), pick a
        // layout based on the visible-client count and the monitor's
        // aspect ratio. Sets `pertag.ltidxs[curtag]` *before* we read
        // it for `layout` below, so a single arrange pass picks up
        // the new value naturally.
        if self.config.auto_layout && !self.monitors[mon_idx].is_overview {
            self.maybe_apply_adaptive_layout(mon_idx);
        }

        let mon = &self.monitors[mon_idx];
        let is_overview = mon.is_overview;
        // Overview path: a single Grid arrangement over the
        // (already-zoomed) work area, holding every tag's clients
        // simultaneously. Mango/Hypr-style geometric continuity —
        // each window keeps a deterministic spot in the thumbnail,
        // and the keyboard-first MRU navigation
        // (`overview_focus_next/prev`) cycles through them with
        // focus + border tracking the selection.
        let layout = if is_overview { crate::layout::LayoutId::Grid } else { mon.current_layout() };
        let tagset = if is_overview { !0 } else { mon.current_tagset() };
        let nmaster = mon.current_nmaster();
        let mfact = mon.current_mfact();
        let monitor_area = mon.monitor_area;
        // Apply `overview_zoom` to the work area so the overview Grid
        // arranges every visible window inside a *centered* sub-rect
        // smaller than the full work area — niri's "zoom 0.5" feeling
        // without a true scene-tree transform. Centering keeps the
        // overview rect inside the layer-shell exclusion zone, so the
        // bar and other top/overlay layers stay anchored to the panel
        // edges (niri pattern: top + overlay layers stay at 1.0,
        // background + bottom would zoom in lock-step — margo doesn't
        // depend on the latter today, so we only zoom the workspace
        // surface).
        let work_area = if is_overview {
            let zoom = self.config.overview_zoom.clamp(0.1, 1.0) as f64;
            let wa = mon.work_area;
            let new_w = ((wa.width as f64) * zoom).round() as i32;
            let new_h = ((wa.height as f64) * zoom).round() as i32;
            let dx = (wa.width - new_w) / 2;
            let dy = (wa.height - new_h) / 2;
            crate::layout::Rect {
                x: wa.x + dx,
                y: wa.y + dy,
                width: new_w.max(1),
                height: new_h.max(1),
            }
        } else {
            mon.work_area
        };
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

        let curtag = self.monitors[mon_idx].pertag.curtag;
        let canvas_pan = (
            self.monitors[mon_idx]
                .pertag
                .canvas_pan_x
                .get(curtag)
                .copied()
                .unwrap_or(0.0),
            self.monitors[mon_idx]
                .pertag
                .canvas_pan_y
                .get(curtag)
                .copied()
                .unwrap_or(0.0),
        );
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
            canvas_pan,
        };

        // Overview path — mango-ext pattern (`overview(m) { grid(m); }`).
        // Above we forced `layout = Grid` and `tagset = !0` when
        // `is_overview`, and the `tiled` filter at line ~2977 admits
        // floating clients in overview too. So a single Grid arrange
        // over every visible window produces the right shape: 1 window
        // ≈ 90%×90% centred, 2 → side-by-side halves, 4 → 2×2 quarters,
        // 9 → 3×3 evenly. Cells shrink as window count grows, which is
        // the natural Mango/Hypr feel — no fixed 3×3 per-tag thumbnails.
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
                    // Overview transitions override the configured
                    // move duration with a snappier value (set by
                    // open_overview/close_overview); falls through to
                    // the user's animation_duration_move otherwise.
                    self.overview_transition_animation_ms
                        .unwrap_or(self.config.animation_duration_move)
                        .max(1)
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
                // Fullscreen geometry per mode:
                //   * Exclusive — full panel, bar will be suppressed
                //     by the render path so the window literally
                //     covers everything.
                //   * WorkArea  — `monitors[mon_idx].work_area`, i.e.
                //     the rect after layer-shell exclusion zones
                //     are subtracted; bar stays drawn on top.
                //   * Off       — fall through to the normal layout /
                //     floating geometry.
                match c.fullscreen_mode {
                    FullscreenMode::Exclusive => {
                        self.clients[i].geom = monitor_area;
                    }
                    FullscreenMode::WorkArea => {
                        self.clients[i].geom = work_area;
                    }
                    FullscreenMode::Off => {
                        if c.is_floating && c.float_geom.width > 0 {
                            self.clients[i].geom = self.clients[i].float_geom;
                        }
                    }
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
        // Refresh the IPC channels so `mctl clients`/`focused`/`status`
        // and any dwl-ipc-v2 bar (waybar-dwl, noctalia, fnott) see new
        // windows the moment they're laid out. arrange_all already
        // covered both, but arrange_monitor (the path most map/unmap/
        // tag-move events take) didn't — leaving state.json + the bar
        // tag-counts stuck on the boot snapshot of zero.
        self.write_state_file();
        crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
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
        let _span = tracy_client::span!("focus_surface");
        // W3.4: push to per-monitor focus_history when a new client
        // takes focus. Walks `target` to a client index, drops dups
        // (same client re-focused = front of queue, no churn), caps
        // at FOCUS_HISTORY_DEPTH.
        const FOCUS_HISTORY_DEPTH: usize = 5;
        if let Some(FocusTarget::Window(w)) = &target {
            let new_idx = self.clients.iter().position(|c| &c.window == w);
            if let Some(idx) = new_idx {
                let mon = self.clients[idx].monitor;
                if mon < self.monitors.len() {
                    let hist = &mut self.monitors[mon].focus_history;
                    hist.retain(|&i| i != idx);
                    hist.push_front(idx);
                    while hist.len() > FOCUS_HISTORY_DEPTH {
                        hist.pop_back();
                    }
                }
            }
        }
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

        // Broadcast the new focus to dwl-ipc-v2 clients (noctalia,
        // waybar-dwl, …). The struct gets its title / appid /
        // fullscreen / floating fields from `focused_client_idx`,
        // which we just changed; without this the bar would keep
        // showing the previously-focused window's title until the
        // next tag-switch / arrange caused some other broadcast to
        // fire. mango broadcasts on every focus change too — this
        // is straight parity.
        if prev_focus_idx != new_focus_idx {
            crate::protocols::dwl_ipc::broadcast_all(self);
            // Phase 3 scripting: invoke any `on_focus_change`
            // handlers the user registered in init.rhai. Hooks
            // see the new focused state via `focused_appid()` /
            // `focused_title()`. Wrapped in a `prev != new` gate
            // because focus_surface is called speculatively from
            // `refresh_keyboard_focus` and we don't want to fire
            // hooks for no-op refreshes.
            crate::scripting::fire_focus_change(self);
        }
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
            if let Some(wp) = &rule.wallpaper {
                mon.pertag.wallpapers[tag] = wp.clone();
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

    /// Geometric rect of the tag-thumbnail cell for `tag` (1..=9) on
    /// `mon_idx`. Returns `None` if the tag is out of range or the
    /// monitor doesn't exist. Same math as
    pub fn is_overview_open(&self) -> bool {
        self.monitors.iter().any(|mon| mon.is_overview)
    }

    /// Snappy overview transition duration (ms). Hard-coded for now —
    /// `animation_duration_move` defaults to 250 ms and the per-window
    /// move animation across N tiles is what made the previous
    /// overview feel laggy. 180 ms with the user's configured easing
    /// curve gives a smooth grid-zoom that still reads as animated.
    /// Fallback overview transition duration. `Config::overview_transition_ms`
    /// overrides this when non-zero; the default config value also
    /// happens to be 180 so behaviour is unchanged unless the user
    /// tunes it. See [`overview_transition_ms`] for the live read.
    const OVERVIEW_TRANSITION_MS: u32 = 180;

    /// Live overview-transition duration: config knob if set, else the
    /// hard-coded fallback. Used by `open_overview` / `close_overview`
    /// to seed `overview_transition_animation_ms`.
    fn overview_transition_ms(&self) -> u32 {
        let cfg = self.config.overview_transition_ms;
        if cfg > 0 { cfg } else { Self::OVERVIEW_TRANSITION_MS }
    }

    pub fn open_overview(&mut self) {
        // Collect the indices of monitors that actually flip into
        // overview on this call. Any monitor already in overview is
        // skipped — re-flipping would clobber `overview_backup_tagset`
        // with the all-tags overview tagset and the close path would
        // restore to `!0` (every tag) on every monitor. That was the
        // root cause of the "tüm pencereler aynı tag'da kalıyor"
        // regression in 8c58b20: the previous attempt mutated state
        // before deciding whether the monitor actually changed.
        let mut flipped: Vec<usize> = Vec::new();
        for (i, mon) in self.monitors.iter_mut().enumerate() {
            if !mon.is_overview {
                mon.overview_backup_tagset = mon.current_tagset().max(1);
                mon.is_overview = true;
                flipped.push(i);
            }
        }

        if flipped.is_empty() {
            return;
        }

        // NOTE: we deliberately don't reset `overview_cycle_pending`
        // here. `open_overview` is reachable from inside
        // `overview_focus_step` (alt+Tab while overview is closed → we
        // open + cycle in one call), and the input handler has ALREADY
        // set `overview_cycle_pending` + `overview_cycle_modifier_mask`
        // by the time we get here. Resetting them would clobber the
        // alt+Tab muscle memory — Alt release wouldn't auto-commit
        // because the flag the release branch reads is false.
        // `close_overview` and `overview_activate` handle the lifetime
        // of the flag on the way out.

        // Snappy 180 ms slide into the grid (vs the user's possibly
        // 250+ ms `animation_duration_move`). The per-client move
        // animation in arrange_monitor reads this override and falls
        // back to the configured value when None.
        self.overview_transition_animation_ms = Some(self.overview_transition_ms());
        self.arrange_monitors(&flipped);
        self.overview_transition_animation_ms = None;
        crate::protocols::dwl_ipc::broadcast_all(self);
    }

    pub fn close_overview(&mut self, activate_window: Option<Window>) {
        let was_open = self.is_overview_open();
        if !was_open {
            return;
        }

        // Drop any pending alt+Tab commit — overview is closing now,
        // a stray modifier-release after this point shouldn't trigger
        // a second `overview_activate` (which would reopen overview).
        self.overview_cycle_pending = false;
        self.overview_cycle_modifier_mask = margo_config::Modifiers::empty();

        let previous_focus = self.focused_client_idx();
        // Fallback chain for "which client should be focused after
        // close":
        //   1. The explicit `activate_window` arg (mouse click on a
        //      thumbnail, `overview_activate` action).
        //   2. The currently-hovered thumbnail — covers keyboard
        //      navigation followed by `Esc` / `alt+Tab` /
        //      `toggleoverview`. Without this, `alt+ctrl+Tab` would
        //      shift the visible highlight but `previous_focus`
        //      would yank focus back to whatever was active before
        //      overview opened, defeating the entire navigation.
        //   3. `previous_focus` below — pre-overview focused client,
        //      used when no thumbnail was ever hovered.
        let activate_idx = activate_window
            .as_ref()
            .and_then(|window| self.clients.iter().position(|client| &client.window == window))
            .or_else(|| self.clients.iter().position(|c| c.is_overview_hovered));

        // Same targeting as open_overview: only arrange the monitors
        // that actually leave overview state. Track them up-front so
        // the tagset restore + arrange operate on the same set even
        // if some side effect somewhere later mutates `is_overview`.
        let mut flipped: Vec<usize> = Vec::new();
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
            flipped.push(mon_idx);
        }

        // Clear hover state on every client — overview is gone, the
        // border layer should drop back to its non-overview palette
        // immediately. Doing this before arrange means the very next
        // border::refresh sees a coherent post-overview world.
        for client in self.clients.iter_mut() {
            client.is_overview_hovered = false;
        }

        self.overview_transition_animation_ms = Some(self.overview_transition_ms());
        self.arrange_monitors(&flipped);
        self.overview_transition_animation_ms = None;

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

    /// All clients shown as overview thumbnails, in the order
    /// `alt+Tab` should walk them. Driven by
    /// `Config::overview_cycle_order`:
    ///
    /// * `Mru` — `focus_history` first (most-recent first), then
    ///   any remaining visible clients in clients-vec order.
    ///   Matches i3/sway/Hypr/niri/GNOME muscle memory.
    /// * `Tag` — tag 1 → 9 in order, clients-vec order inside each
    ///   tag. Spatial-memory: tag 1's windows always first, tag 9's
    ///   always last, independent of focus history.
    /// * `Mixed` — current tag's clients in MRU order, then the
    ///   remaining tags in strict tag order. "MRU where you live,
    ///   tag elsewhere."
    fn overview_visible_clients(&self) -> Vec<usize> {
        use margo_config::OverviewCycleOrder;
        let mut out = Vec::new();
        for mon_idx in 0..self.monitors.len() {
            if !self.monitors[mon_idx].is_overview {
                continue;
            }
            let visible_here = |i: usize, c: &MargoClient| -> bool {
                c.monitor == mon_idx
                    && !c.is_initial_map_pending
                    && !c.is_minimized
                    && !c.is_killing
                    && !c.is_in_scratchpad
                    && i < self.clients.len()
            };
            let mut seen: std::collections::HashSet<usize> =
                std::collections::HashSet::new();

            let push_mru = |out: &mut Vec<usize>,
                            seen: &mut std::collections::HashSet<usize>,
                            tag_filter: u32| {
                for &i in &self.monitors[mon_idx].focus_history {
                    if i >= self.clients.len() {
                        continue;
                    }
                    let c = &self.clients[i];
                    if tag_filter != 0 && (c.tags & tag_filter) == 0 {
                        continue;
                    }
                    if visible_here(i, c) && seen.insert(i) {
                        out.push(i);
                    }
                }
            };
            let push_tag_order = |out: &mut Vec<usize>,
                                  seen: &mut std::collections::HashSet<usize>,
                                  skip_tags: u32| {
                for tag_idx in 0..crate::layout::MAX_TAGS as u32 {
                    let tag_bit = 1u32 << tag_idx;
                    if (skip_tags & tag_bit) != 0 {
                        continue;
                    }
                    for (i, c) in self.clients.iter().enumerate() {
                        if (c.tags & tag_bit) == 0 {
                            continue;
                        }
                        if visible_here(i, c) && seen.insert(i) {
                            out.push(i);
                        }
                    }
                }
            };

            match self.config.overview_cycle_order {
                OverviewCycleOrder::Mru => {
                    push_mru(&mut out, &mut seen, 0);
                    // Trailing tail: anything `focus_history` never
                    // touched (newly-mapped, never-focused) goes at
                    // the end in clients-vec order. Without this a
                    // brand-new window would be unreachable via
                    // alt+Tab until it gained focus once.
                    for (i, c) in self.clients.iter().enumerate() {
                        if visible_here(i, c) && seen.insert(i) {
                            out.push(i);
                        }
                    }
                }
                OverviewCycleOrder::Tag => {
                    push_tag_order(&mut out, &mut seen, 0);
                }
                OverviewCycleOrder::Mixed => {
                    // Current tag(set) in MRU order: covers the
                    // common case where the user is rapidly
                    // alternating between two windows on the active
                    // tag. Remaining tags fall back to strict tag
                    // order for predictability.
                    let cur_tagset = self.monitors[mon_idx].current_tagset();
                    push_mru(&mut out, &mut seen, cur_tagset);
                    push_tag_order(&mut out, &mut seen, cur_tagset);
                }
            }
        }
        out
    }

    pub fn overview_focus_next(&mut self) {
        self.overview_focus_step(1);
    }

    pub fn overview_focus_prev(&mut self) {
        self.overview_focus_step(-1);
    }

    /// Cycle the overview thumbnail one step in `dir` (+1 = next,
    /// −1 = prev). niri-style keyboard-first MRU navigator:
    ///
    /// * **Overview closed?** Open it first. The first cycle press
    ///   then lands on the natural starting thumbnail (first for +1,
    ///   last for −1) — single-keystroke "open + select first".
    /// * **Cycle wrap-around** matches alt+Tab on every other DE.
    /// * **Focus follows the cycle** — every step calls
    ///   `focus_surface(Some(FocusTarget::Window(...)))` so the
    ///   border immediately repaints with `focuscolor`, smithay's
    ///   keyboard focus is on the new thumbnail's window, and
    ///   activating it later (Enter / `overview_activate`) is just
    ///   `close_overview(focus)`. Overview stays open between
    ///   cycles — user keeps tapping Tab to walk the MRU.
    /// * **Pointer warp** to thumbnail centre keeps the next mouse
    ///   motion from yanking hover off the keyboard-selected
    ///   thumbnail.
    fn overview_focus_step(&mut self, dir: i32) {
        // First press while closed = open + select natural start.
        if !self.is_overview_open() {
            self.open_overview();
        }
        let list = self.overview_visible_clients();
        if list.is_empty() {
            return;
        }
        // Anchor the cycle. Priority:
        //   1. An already-hovered thumbnail (keyboard cycle in progress).
        //   2. The currently-focused client's position in the list.
        //      With MRU order that's index 0 (the focused window is
        //      the freshest entry in `focus_history`), so the first
        //      `dir = +1` press lands on index 1 = the previously-used
        //      window. This is the standard alt+Tab behaviour every
        //      other DE ships: one tap moves you to the *other* MRU
        //      window, not back to yourself. With `tag` / `mixed`
        //      modes the focused window's index can be anywhere in
        //      the list, but the same "step away from focused" rule
        //      gives the user a meaningful first move.
        //   3. Fall through to position 0 / n-1 only if no client is
        //      focused (empty workspace, lock screen edge cases).
        let cur = list
            .iter()
            .position(|&i| self.clients[i].is_overview_hovered)
            .or_else(|| {
                self.focused_client_idx()
                    .and_then(|f| list.iter().position(|&i| i == f))
            });
        let n = list.len() as i32;
        let next_pos = match cur {
            Some(p) => (((p as i32 + dir).rem_euclid(n)) + n).rem_euclid(n),
            None => {
                if dir > 0 {
                    0
                } else {
                    n - 1
                }
            }
        } as usize;
        let new_idx = list[next_pos];

        for &i in &list {
            self.clients[i].is_overview_hovered = false;
        }
        self.clients[new_idx].is_overview_hovered = true;

        // Note: deliberately NO `arrange_monitor` here. Mango-ext
        // overview is a Grid layout — every cell stays put across
        // a cycle, only the *selected* state changes. Skipping the
        // arrange means the only state that flips this tick is
        // `is_overview_hovered`, which `border::refresh` reads on
        // the very next frame. Result: the focuscolor border lights
        // up the new selection in a single render, no animation
        // gate, no per-client move recompute, no opacity
        // crossfade — what the user calls "instant."

        // Pointer warp to thumbnail centre so a subsequent mouse
        // motion doesn't yank `is_overview_hovered` off our
        // keyboard pick. Geometry is steady (no in-flight arrange)
        // so the centre we compute is the cell the user is about
        // to click.
        let g = self.clients[new_idx].geom;
        if g.width > 0 && g.height > 0 {
            self.input_pointer.x = (g.x + g.width / 2) as f64;
            self.input_pointer.y = (g.y + g.height / 2) as f64;
            self.clamp_pointer_to_outputs();
        }

        // Don't call `focus_surface` here. While overview is open,
        // `border::refresh` already paints `is_overview_hovered`
        // with `focuscolor` (margo/src/border.rs:64), so the border
        // colour tracks the selection without going through the
        // smithay focus path. Calling `focus_surface` on every Tab
        // press also kicks off an opacity-crossfade animation per
        // step (state.rs:200-208) and shuffles dwl-ipc focus_history
        // — both visible side-effects that made the cycle feel
        // sluggish ("border yerine sadece imleç dolaşıyor"). The
        // user commits the cycle's choice via `overview_activate`
        // (Enter), which closes the overview onto the hovered
        // thumbnail and runs the focus path once.
        crate::border::refresh(self);
        self.request_repaint();
        tracing::debug!(
            target: "overview",
            dir = dir,
            new_idx = new_idx,
            list_len = list.len(),
            "cycle",
        );
    }

    /// Close overview activating whichever thumbnail keyboard
    /// navigation last highlighted (or the cursor-hovered one).
    /// No-op outside overview. With no hover set, falls through to
    /// `close_overview(None)` which restores the pre-overview tag
    /// without changing focus.
    pub fn overview_activate(&mut self) {
        if !self.is_overview_open() {
            return;
        }
        let window = self
            .clients
            .iter()
            .find(|c| c.is_overview_hovered)
            .map(|c| c.window.clone());
        self.close_overview(window);
    }

    /// Why a window-rule reapply is happening. Lets the single
    /// reapply path log meaningfully and (in future) skip rule subsets
    /// that don't make sense for a given trigger (e.g. `tags:`
    /// shouldn't move a client on `Reload`).
    fn apply_window_rules(&self, client: &mut MargoClient) {
        // Pre-mount path (X11 + initial XDG before the client is in
        // `self.clients`). The post-mount equivalent is
        // [`reapply_rules`].
        let rules = self.matching_window_rules(&client.app_id, &client.title);
        Self::apply_matched_window_rules(&self.monitors, client, &rules);
    }

    /// Single post-mount window-rule reapply path. All three trigger
    /// sites — initial XDG mount, late `app_id` settle, config reload —
    /// route through this with a [`WindowRuleReason`] tag so the debug
    /// log says *why* a rule fired.
    pub(crate) fn reapply_rules(
        &mut self,
        idx: usize,
        reason: WindowRuleReason,
    ) -> bool {
        if idx >= self.clients.len() {
            return false;
        }
        let (app_id, title) = {
            let client = &self.clients[idx];
            (client.app_id.clone(), client.title.clone())
        };
        let rules = self.matching_window_rules(&app_id, &title);
        if rules.is_empty() {
            tracing::trace!(
                target: "windowrule",
                reason = ?reason,
                app_id = %app_id,
                title = %title,
                "reapply: no rules match",
            );
            return false;
        }
        Self::apply_matched_window_rules(&self.monitors, &mut self.clients[idx], &rules);
        tracing::debug!(
            target: "windowrule",
            reason = ?reason,
            count = rules.len(),
            app_id = %app_id,
            title = %title,
            "reapply: applied {} rules",
            rules.len(),
        );
        true
    }

    /// Live-swap the visual theme without touching `~/.config/margo/config.conf`.
    ///
    /// Three built-in presets:
    ///   * `default` — restore the values parsed from the config file at
    ///     startup (or the most recent `mctl reload`).
    ///   * `minimal` — borders thin, shadows off, blur off, square corners.
    ///     Good for low-end GPUs or anyone who likes a flat look.
    ///   * `gaudy`   — chunky borders, deep drop shadows, rounded corners,
    ///     blur on. Demo / screenshot mode.
    ///
    /// The first call captures the current config values into
    /// `theme_baseline` so `default` always means "what was on disk
    /// before the user started swapping". `mctl reload` re-invalidates
    /// the baseline so reload + `default` gives the freshly-parsed
    /// values.
    ///
    /// Returns `Err(reason)` for an unknown preset name; the dispatch
    /// handler turns this into a user-visible warning.
    pub fn apply_theme_preset(&mut self, name: &str) -> Result<(), String> {
        // Lazy capture — first preset switch establishes the
        // "what the config file said" baseline.
        if self.theme_baseline.is_none() {
            self.theme_baseline = Some(ThemeBaseline::capture(&self.config));
        }
        let baseline = self.theme_baseline.as_ref().unwrap().clone();

        match name {
            "default" => baseline.apply_to(&mut self.config),
            "minimal" => {
                self.config.shadows = false;
                self.config.layer_shadows = false;
                self.config.shadow_only_floating = false;
                self.config.blur = false;
                self.config.blur_layer = false;
                self.config.border_radius = 0;
                self.config.borderpx = 1;
            }
            "gaudy" => {
                self.config.shadows = true;
                self.config.layer_shadows = true;
                self.config.shadows_size = 32;
                self.config.shadows_blur = 18.0;
                self.config.border_radius = 14;
                self.config.borderpx = 4;
            }
            other => {
                return Err(format!(
                    "unknown theme preset `{other}` — try `default`, `minimal`, or `gaudy`"
                ));
            }
        }

        // Border / shadow / blur all read straight off `self.config`
        // every frame, so an arrange + repaint is enough — no
        // per-client mutation, no animation re-bake.
        self.arrange_all();
        self.request_repaint();
        tracing::info!(target: "theme", "applied preset `{name}`");
        Ok(())
    }

    pub(crate) fn matching_window_rules(&self, app_id: &str, title: &str) -> Vec<WindowRule> {
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
            // Per-window animation-type overrides. The rule's
            // `animation_type_open` / `animation_type_close` win over
            // the global config when the window opens or closes —
            // `finalize_initial_map` and `toplevel_destroyed` already
            // read these per-client fields and only fall back to the
            // global `Config::animation_type_*` when they're `None`.
            if let Some(value) = rule.animation_type_open.as_ref() {
                client.animation_type_open = Some(value.clone());
            }
            if let Some(value) = rule.animation_type_close.as_ref() {
                client.animation_type_close = Some(value.clone());
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

    pub(crate) fn window_rule_matches(
        &self,
        rule: &WindowRule,
        app_id: &str,
        title: &str,
    ) -> bool {
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

        if should_reapply_rules && self.reapply_rules(idx, WindowRuleReason::AppIdSettled) {
            let new_monitor = self.clients[idx].monitor;
            if old_monitor != new_monitor {
                self.arrange_monitor(old_monitor);
            }
            self.arrange_monitor(new_monitor);
            crate::protocols::dwl_ipc::broadcast_all(self);
        } else if title_changed || app_id_changed {
            // Even when no rule reapply was needed (the client just
            // changed its title — e.g. browser tab switch — and no
            // title-keyed rules exist), noctalia / waybar-dwl still
            // care about the new title / app_id for their focused-
            // window indicator. Mango broadcasts on every title
            // commit; without this the bar would freeze on the
            // previous title until something else triggered a
            // broadcast.
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
        // Phase 3 scripting: fire `on_tag_switch` handlers. Runs
        // after focus + broadcast so a handler reading
        // `current_tag()` / `focused_appid()` sees the post-switch
        // state, not the pre-switch.
        crate::scripting::fire_tag_switch(self);
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
            // User explicitly picked a layout — adaptive auto-layout
            // must back off on this tag so its choice survives every
            // subsequent arrange pass. Reset by `view_tag` switching
            // to a tag that's never been touched by `setlayout` and
            // letting auto-layout pick again.
            self.monitors[mon_idx].pertag.user_picked_layout[curtag] = true;
            self.arrange_monitor(mon_idx);
        }
    }

    /// Adaptive layout heuristic: pick the most ergonomic layout for
    /// the current tag based on visible-client count + monitor aspect
    /// ratio. Called from `arrange_monitor` when `Config::auto_layout`
    /// is on. Skipped on tags where the user has explicitly called
    /// `setlayout` (sticky `pertag.user_picked_layout` flag) so a
    /// deliberate user choice is never overridden.
    fn maybe_apply_adaptive_layout(&mut self, mon_idx: usize) {
        let curtag = self.monitors[mon_idx].pertag.curtag;
        if self
            .monitors[mon_idx]
            .pertag
            .user_picked_layout
            .get(curtag)
            .copied()
            .unwrap_or(false)
        {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();
        let mon_area = self.monitors[mon_idx].monitor_area;

        // Count tile-eligible visible clients (skip floating /
        // fullscreen / scratchpad / minimised — they don't take up a
        // tile slot).
        let count = self
            .clients
            .iter()
            .filter(|c| c.is_visible_on(mon_idx, tagset) && c.is_tiled())
            .count();
        if count == 0 {
            // Empty tag — keep whatever's set so the user sees the
            // *next* arrival land in a sensible layout for one.
            return;
        }

        let aspect = if mon_area.height > 0 {
            mon_area.width as f32 / mon_area.height as f32
        } else {
            16.0 / 9.0
        };
        let very_wide = aspect >= 2.4; // ultrawide / 32:9
        let wide = aspect >= 1.5; // 16:9 / 16:10
        let portrait = aspect <= 0.9; // rotated panels

        // Heuristic. Tuned for the user's two-monitor setup
        // (DP-3 2560x1440 → wide; eDP-1 1920x1200 → wide-ish):
        //
        //   1 client  → monocle    (no point splitting space for one)
        //   2 clients → tile        (master/stack ratio classic)
        //   3-5 wide  → scroller    (niri-style horizontal tracks)
        //   3-5 portrait → vertical_scroller
        //   6+  wide  → grid
        //   6+  ultrawide → vertical_scroller (long horizontal track)
        //   6+  portrait → vertical_grid
        //
        // The thresholds and choices are deliberately conservative —
        // adaptive should "feel right" 90% of the time, never wrong.
        // A user who wants a different mapping toggles it off and
        // bumps `setlayout` directly per tag.
        let chosen = match (count, very_wide, wide, portrait) {
            (1, _, _, _) => crate::layout::LayoutId::Monocle,
            (2, _, _, _) => crate::layout::LayoutId::Tile,
            (3..=5, _, _, true) => crate::layout::LayoutId::VerticalScroller,
            (3..=5, _, true, _) => crate::layout::LayoutId::Scroller,
            (3..=5, _, _, _) => crate::layout::LayoutId::Tile,
            (_, _, _, true) => crate::layout::LayoutId::VerticalGrid,
            (_, true, _, _) => crate::layout::LayoutId::VerticalScroller,
            (_, _, true, _) => crate::layout::LayoutId::Grid,
            _ => crate::layout::LayoutId::Tile,
        };

        if self.monitors[mon_idx].pertag.ltidxs[curtag] != chosen {
            tracing::info!(
                "auto_layout: tag={} clients={} aspect={:.2} → {:?}",
                curtag,
                count,
                aspect,
                chosen,
            );
            self.monitors[mon_idx].pertag.ltidxs[curtag] = chosen;
        }
    }

    /// Spatial-canvas pan: shift the *viewport* on the active tag by
    /// (dx, dy) logical pixels. Stored per-tag so each tag remembers
    /// where the user had been "looking" in the canvas. The
    /// `Canvas` layout reads the offset on every arrange and
    /// translates each client's `canvas_geom` by it — clients stay
    /// anchored on the canvas, the viewport moves.
    pub fn canvas_pan(&mut self, dx: i32, dy: i32) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let curtag = self.monitors[mon_idx].pertag.curtag;
        if let Some(slot) = self.monitors[mon_idx].pertag.canvas_pan_x.get_mut(curtag) {
            *slot += dx as f64;
        }
        if let Some(slot) = self.monitors[mon_idx].pertag.canvas_pan_y.get_mut(curtag) {
            *slot += dy as f64;
        }
        self.arrange_monitor(mon_idx);
    }

    /// Reset the active tag's canvas viewport to the origin (0, 0).
    pub fn canvas_reset(&mut self) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let curtag = self.monitors[mon_idx].pertag.curtag;
        if let Some(slot) = self.monitors[mon_idx].pertag.canvas_pan_x.get_mut(curtag) {
            *slot = 0.0;
        }
        if let Some(slot) = self.monitors[mon_idx].pertag.canvas_pan_y.get_mut(curtag) {
            *slot = 0.0;
        }
        self.arrange_monitor(mon_idx);
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
        self.notify_layout(&next);
    }

    /// Toggle the focused client's "sticky" / global state — visible
    /// on every tag of its current monitor instead of only the tag
    /// it was tagged with. Equivalent to niri-float-sticky's
    /// per-window sticky toggle, but built into the compositor so
    /// no external daemon is needed.
    ///
    /// Implementation: when sticking, save the current tag mask onto
    /// `old_tags` and overwrite `tags = u32::MAX`. Every
    /// `is_visible_on(mon, tagset)` check walks `(tags & tagset)
    /// != 0`, and `u32::MAX & anything` is `anything` (non-zero
    /// for any active tagset), so the window shows up wherever the
    /// monitor goes.
    ///
    /// When unsticking, restore from `old_tags`. If `old_tags` is
    /// 0 (rule never saved one — a freshly-created sticky-by-rule
    /// client) fall back to whichever tag is currently visible on
    /// the monitor so the window doesn't vanish.
    ///
    /// Cross-monitor sticky (window visible on multiple monitors at
    /// once) is a separate, much-bigger change — would need scene-
    /// graph mapping per output. Skipped for now; this covers the
    /// niri-float-sticky single-monitor "appears on every tag of
    /// this output" case which is the 95% use.
    pub fn toggle_sticky(&mut self) {
        let Some(idx) = self.focused_client_idx() else { return };
        // Don't sticky scratchpads — they have their own
        // visibility model (`is_scratchpad_show` flag); flipping
        // tags out from under the scratchpad path would confuse it.
        if self.clients[idx].is_in_scratchpad {
            tracing::info!("toggle_sticky: skipped (client is in scratchpad)");
            return;
        }
        let was_sticky = self.clients[idx].is_global;
        let mon_idx = self.clients[idx].monitor;
        let appid = self.clients[idx].app_id.clone();

        if was_sticky {
            // Restore previous tag mask. Fall back to the monitor's
            // currently-visible tag if old_tags wasn't populated
            // (rule-driven sticky-from-spawn never went through
            // toggle).
            let restored = if self.clients[idx].old_tags != 0 {
                self.clients[idx].old_tags
            } else {
                self.monitors
                    .get(mon_idx)
                    .map(|m| m.current_tagset())
                    .filter(|m| *m != 0)
                    .unwrap_or(1)
            };
            self.clients[idx].tags = restored;
            self.clients[idx].is_global = false;
        } else {
            self.clients[idx].old_tags = self.clients[idx].tags;
            self.clients[idx].tags = u32::MAX;
            self.clients[idx].is_global = true;
        }

        self.arrange_monitor(mon_idx);
        crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
        self.request_repaint();
        crate::scripting::fire_focus_change(self);

        // OSD-style notification — short timeout so it doesn't
        // pile up if the user toggles a few windows in a row.
        let title = if was_sticky { "Sticky off" } else { "Sticky on" };
        let body = if appid.is_empty() {
            String::from("Focused window")
        } else {
            appid
        };
        let _ = crate::utils::spawn([
            "notify-send", "-a", "margo",
            "-i", "view-pin-symbolic",
            "-t", "1200",
            title, &body,
        ]);
    }

    /// Fire an OSD-style notification telling the user the active
    /// layout just changed. Called from `switch_layout` (cycle) and
    /// from the `setlayout` dispatch handler (explicit pick) — not
    /// from `set_layout` itself, because that's also called
    /// internally for window-rule application and we don't want to
    /// notify on every rule-driven re-arrangement.
    pub fn notify_layout(&self, name: &str) {
        // W3.5: enrich the toast with position-in-cycle context so
        // users navigating the 14-layout catalogue see where they
        // are. Format: `<name> (<pos>/<total>) → <next>`. Falls
        // back to bare name when not in `circle_layout`.
        let cycle: Vec<String> = if self.config.circle_layouts.is_empty() {
            vec![]
        } else {
            self.config.circle_layouts.clone()
        };
        let body = if let Some(pos) = cycle.iter().position(|n| n == name) {
            let total = cycle.len();
            let next = &cycle[(pos + 1) % total];
            format!("{name}  ({}/{total}) → next: {next}", pos + 1)
        } else {
            name.to_string()
        };
        let _ = crate::utils::spawn([
            "notify-send", "-a", "margo",
            "-i", "view-grid-symbolic",
            "-t", "1200",
            "Margo Layout", &body,
        ]);
    }

    /// Toast for layout-adjacent actions (proportion preset,
    /// gap toggle). Same look-and-feel as `notify_layout` so the
    /// user can rely on the in-corner toast giving consistent
    /// state feedback for layout-cycle keybinds.
    pub fn notify_layout_state(&self, action: &str, value: &str) {
        let body = format!("{action}: {value}");
        let _ = crate::utils::spawn([
            "notify-send", "-a", "margo",
            "-i", "view-grid-symbolic",
            "-t", "1000",
            "Margo Layout", &body,
        ]);
    }

    pub fn toggle_floating(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            self.clients[idx].is_floating = !self.clients[idx].is_floating;
            if self.clients[idx].is_floating && self.clients[idx].float_geom.width == 0 {
                self.clients[idx].float_geom = self.clients[idx].geom;
            }
            let mon_idx = self.clients[idx].monitor;
            self.arrange_monitor(mon_idx);
            // dwl-ipc-v2 reports `floating` per output's focused
            // client; the bar status indicator (noctalia "tile/float"
            // glyph) needs an explicit broadcast or it stays stale.
            crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
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
        let next_proportion = presets[(current_pos + 1) % presets.len()];
        self.clients[idx].scroller_proportion = next_proportion;
        let mon_idx = self.clients[idx].monitor;
        self.arrange_monitor(mon_idx);
        // W3.5: toast feedback for the cycling action.
        self.notify_layout_state(
            "scroller proportion",
            &format!("{:.2}", next_proportion),
        );
    }

    /// Toggle the focused client between [`FullscreenMode::Exclusive`] and
    /// [`FullscreenMode::Off`]. Bound to `togglefullscreen_exclusive`. The
    /// difference vs `togglefullscreen` is that the render path will
    /// suppress every layer-shell surface on this output while
    /// `Exclusive` is active — the bar disappears, the focused window
    /// covers the panel pixels too.
    pub fn toggle_fullscreen_exclusive(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            let target = if self.clients[idx].fullscreen_mode == FullscreenMode::Exclusive {
                FullscreenMode::Off
            } else {
                FullscreenMode::Exclusive
            };
            self.set_client_fullscreen_mode(idx, target);
        }
    }

    pub fn toggle_fullscreen(&mut self) {
        if let Some(idx) = self.focused_client_idx() {
            let target = !self.clients[idx].is_fullscreen;
            self.set_client_fullscreen(idx, target);
        }
    }

    /// Set a client's fullscreen state and inform the client via
    /// the xdg_toplevel protocol so it actually re-renders for
    /// fullscreen (drops decorations, fills the new geom).
    ///
    /// Three things happen in lockstep here:
    ///
    /// 1. `client.is_fullscreen` flips — drives margo's layout
    ///    pass (arrange_monitor gives a fullscreen client the
    ///    full monitor rect).
    /// 2. `xdg_toplevel.with_pending_state` adds / removes the
    ///    `Fullscreen` state and pins the size to the monitor
    ///    rect. Without this, browsers + native fullscreen apps
    ///    keep rendering the windowed UI even when their geom
    ///    has changed — they trust the protocol state, not the
    ///    geom. This was the bug behind "F11 / video player
    ///    fullscreen does nothing" until W4.5.
    /// 3. `arrange_monitor` runs the layout pass which queues
    ///    the actual configure send + rerenders the scene.
    /// 4. `broadcast_monitor` updates dwl-ipc bars (which carry
    ///    the focused-client `fullscreen` flag for the icon
    ///    indicator).
    ///
    /// X11 clients (XWayland) follow a different protocol path
    /// (NetWMState); we just flip the flag for them and let
    /// arrange handle geometry — that path was already correct
    /// before this fix because XWayland clients trust geom
    /// without a state-change packet.
    pub fn set_client_fullscreen(&mut self, idx: usize, fullscreen: bool) {
        // Backward-compat shim: `bool` API maps to `WorkArea` mode.
        // XDG `set_fullscreen()` requests + the existing keybind path
        // both still go through here. Real two-way distinction lives
        // in [`set_client_fullscreen_mode`].
        let mode = if fullscreen {
            FullscreenMode::WorkArea
        } else {
            FullscreenMode::Off
        };
        self.set_client_fullscreen_mode(idx, mode);
    }

    /// Apply a [`FullscreenMode`] to a client. The single source of truth
    /// for fullscreen — `set_client_fullscreen` is a shim, the dispatch
    /// actions `togglefullscreen` / `togglefullscreen_exclusive` route
    /// through here.
    ///
    /// Three rotating concerns:
    ///
    /// 1. `MargoClient` state — `fullscreen_mode` + the
    ///    backward-compat `is_fullscreen` bool stay in lock-step.
    /// 2. xdg_toplevel pending state — Wayland clients get the
    ///    `Fullscreen` state bit + a size hint matching the mode
    ///    (`work_area` for WorkArea, `monitor_area` for Exclusive).
    ///    X11 surfaces are skipped; NetWMState round-trip isn't
    ///    wired today (known limitation, see `state/handlers/x11.rs`).
    /// 3. Layout pass + IPC broadcast — `arrange_monitor` reads the
    ///    new mode to size the geometry, then dwl-ipc clients
    ///    (noctalia / waybar-dwl) see the updated state.json.
    pub fn set_client_fullscreen_mode(&mut self, idx: usize, mode: FullscreenMode) {
        if idx >= self.clients.len() {
            return;
        }
        let mon_idx = self.clients[idx].monitor;
        if mon_idx >= self.monitors.len() {
            return;
        }
        self.clients[idx].fullscreen_mode = mode;
        self.clients[idx].is_fullscreen = mode != FullscreenMode::Off;

        if let WindowSurface::Wayland(toplevel) =
            self.clients[idx].window.underlying_surface()
        {
            use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
            // The size hint matches the mode: WorkArea respects the
            // bar's exclusion zone, Exclusive covers the entire
            // panel. Clients honour this for their initial buffer
            // allocation; the actual rect lands via `arrange_monitor`.
            let target_size = match mode {
                FullscreenMode::Off => None,
                FullscreenMode::WorkArea => {
                    let wa = self.monitors[mon_idx].work_area;
                    Some(smithay::utils::Size::from((wa.width, wa.height)))
                }
                FullscreenMode::Exclusive => {
                    let ma = self.monitors[mon_idx].monitor_area;
                    Some(smithay::utils::Size::from((ma.width, ma.height)))
                }
            };
            toplevel.with_pending_state(|state| {
                if mode == FullscreenMode::Off {
                    state.states.unset(xdg_toplevel::State::Fullscreen);
                    state.size = None;
                } else {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                    state.size = target_size;
                }
            });
            toplevel.send_pending_configure();
        }

        tracing::info!(
            target: "fullscreen",
            client = idx,
            mode = ?mode,
            "applied",
        );

        self.arrange_monitor(mon_idx);
        crate::protocols::dwl_ipc::broadcast_monitor(self, mon_idx);
    }

    /// Does any client on `mon_idx` currently hold an exclusive
    /// fullscreen lease? Used by the render path to decide whether
    /// to suppress layer-shell surfaces (bar / notification overlay)
    /// for that output.
    pub fn monitor_has_exclusive_fullscreen(&self, mon_idx: usize) -> bool {
        if mon_idx >= self.monitors.len() {
            return false;
        }
        let active_tagset = self.monitors[mon_idx].current_tagset();
        self.clients.iter().any(|c| {
            c.monitor == mon_idx
                && c.fullscreen_mode == FullscreenMode::Exclusive
                && c.is_visible_on(mon_idx, active_tagset)
        })
    }

    // ── Scratchpad ────────────────────────────────────────────────────────────
    //
    // Mango-style named scratchpads. A scratchpad client is a regular
    // toplevel that the user keeps "in their pocket": invisible by
    // default, summoned onto the current tag with a single keybind,
    // dismissed back into hiding with the same keybind. Margo's window
    // rules already let the user mark a client `isnamedscratchpad:1`
    // and pin its float geometry (width / height / offsetx / offsety)
    // via the existing windowrule plumbing — what was missing is the
    // toggle / spawn-on-miss action that ties it together.
    //
    // The implementation mirrors mango-ext's `toggle_named_scratchpad`
    // + `apply_named_scratchpad` + `switch_scratchpad_client_state` +
    // `show_scratchpad` chain, simplified by skipping cross-monitor
    // migration and canvas-layout per-tag offsets (we don't carry
    // those on `MargoClient` yet).

    /// Find the index of the first client whose `app_id` matches `name`
    /// (substring, case-insensitive) and, if `title` is supplied, whose
    /// title also contains it. Used by [`toggle_named_scratchpad`] to
    /// locate an already-running instance before deciding whether to
    /// spawn a new one.
    fn find_client_by_id_or_title(
        &self,
        name: Option<&str>,
        title: Option<&str>,
    ) -> Option<usize> {
        // Use the same regex matcher the windowrule machinery uses so
        // bind authors can write `clipse`, `^clipse$`, or `clip(se|board)`
        // and get consistent semantics. The earlier `.contains()`
        // substring match was a footgun: a user-typed bare `clipse`
        // matched any client whose app_id contained the substring
        // "clipse", which is fine right up until a different toolkit
        // happens to namespace itself with one of the scratchpad
        // names — at which point a regular window silently got
        // promoted to a scratchpad on the next toggle press, with no
        // way to escape short of restarting margo. Anchored or
        // word-boundary-aware patterns (`^clipse$`, `\bwiremix\b`)
        // protect against that.
        let name_pat = name.unwrap_or("");
        let title_pat = title.unwrap_or("");
        for (idx, c) in self.clients.iter().enumerate() {
            let app_match = if name_pat.is_empty() {
                true
            } else {
                matches_rule_text(name_pat, &c.app_id)
            };
            let title_match = if title_pat.is_empty() {
                true
            } else {
                matches_rule_text(title_pat, &c.title)
            };
            if app_match && title_match {
                return Some(idx);
            }
        }
        None
    }

    /// Bring a scratchpad client onto the active tagset and centre it
    /// at its `float_geom` (already populated by the windowrule's
    /// width/height/offsetx/offsety, or falls back to the
    /// `scratchpad_*_ratio` config defaults).
    fn show_scratchpad_client(&mut self, idx: usize) {
        if idx >= self.clients.len() {
            return;
        }

        // Migrate the scratchpad to the *cursor's* monitor when
        // `scratchpad_cross_monitor` is on (the user's config has
        // it enabled). Without this, a scratchpad first opened on
        // eDP-1 (because tag-home routing parked it there at map
        // time, or because the focused client lived there when the
        // spawn fired) would always re-show on eDP-1 even after
        // the user moved their cursor to DP-3 — exactly the
        // "imlecin olduğu ekranda değil eDP-1'de açılıyor" symptom.
        //
        // We deliberately use `pointer_monitor()` rather than
        // `focused_monitor()` for the migration target. A
        // scratchpad summon is a "bring it *here*" gesture; if the
        // user is reading docs on DP-3 with cursor there but their
        // last keyboard focus happened to land on eDP-1
        // (Spotify-on-tag-8 stays focused after a brief click
        // through), they'd still want clipse / wiremix to drop
        // down where the cursor is. Falls back to focused-monitor
        // → client's stored monitor if the pointer hasn't entered
        // any output yet (rare, mostly during session bring-up).
        let target_mon_idx = if self.config.scratchpad_cross_monitor {
            self.pointer_monitor()
                .or_else(|| {
                    let f = self.focused_monitor();
                    (f < self.monitors.len()).then_some(f)
                })
                .filter(|i| *i < self.monitors.len())
                .unwrap_or(self.clients[idx].monitor)
        } else {
            self.clients[idx].monitor
        };
        if target_mon_idx >= self.monitors.len() {
            return;
        }
        // Apply the migration before reading work_area / tagset so
        // the scratchpad rect is centred on its new home.
        self.clients[idx].monitor = target_mon_idx;
        let work_area = self.monitors[target_mon_idx].work_area;
        let active_tagset = self.monitors[target_mon_idx].current_tagset();

        // Re-centre the float_geom on the target monitor's work
        // area while preserving the rule-supplied size+offset. The
        // windowrule's offsetx/offsety is a hint about *where on
        // the active monitor* the scratchpad should sit, not an
        // absolute screen position — using the absolute coords
        // baked at first-map time would put the panel on whichever
        // monitor it was originally arranged for.
        let c = &mut self.clients[idx];
        c.is_in_scratchpad = true;
        c.is_scratchpad_show = true;
        c.is_minimized = false;
        c.is_floating = true;
        c.is_fullscreen = false;
        c.is_maximized_screen = false;

        // Decide width/height: prefer windowrule values if they
        // were set, fall back to `scratchpad_*_ratio * work_area`.
        let (w, h) = if c.float_geom.width > 0 && c.float_geom.height > 0 {
            (
                c.float_geom.width.min(work_area.width.max(1)),
                c.float_geom.height.min(work_area.height.max(1)),
            )
        } else {
            (
                (work_area.width as f32 * self.config.scratchpad_width_ratio).round() as i32,
                (work_area.height as f32 * self.config.scratchpad_height_ratio).round() as i32,
            )
        };
        // Recentre on the active monitor's work area, then layer
        // the rule's offset on top so user-tuned positioning still
        // applies (the user has e.g. `offsety:-100` on
        // dropdown-terminal so it docks near the top of whatever
        // monitor it lands on).
        let center_x = work_area.x + (work_area.width - w) / 2;
        let center_y = work_area.y + (work_area.height - h) / 2;
        c.float_geom = Rect {
            x: center_x,
            y: center_y,
            width: w.max(100),
            height: h.max(100),
        };
        c.geom = c.float_geom;
        c.tags = active_tagset; // join the current tagset

        let window = c.window.clone();
        // Re-map at the float position. `map_element(_, _, true)`
        // raises to the top of the scene, which is what we want for a
        // toggled-up scratchpad.
        self.space.map_element(window.clone(), (c.float_geom.x, c.float_geom.y), true);
        self.enforce_z_order();
        self.arrange_monitor(target_mon_idx);
        self.focus_surface(Some(FocusTarget::Window(window)));
    }

    /// Tuck a scratchpad client away. We unmap from the scene so it
    /// doesn't render anywhere, and clear `is_scratchpad_show` so the
    /// next `toggle_named_scratchpad` flips it back on.
    fn hide_scratchpad_client(&mut self, idx: usize) {
        if idx >= self.clients.len() {
            return;
        }
        let window = self.clients[idx].window.clone();
        self.clients[idx].is_scratchpad_show = false;
        self.clients[idx].is_minimized = true;
        self.space.unmap_elem(&window);
        // If this was the focused window, drop focus to the next
        // visible client on the same monitor so the keyboard isn't
        // stranded on a hidden surface.
        let mon_idx = self.clients[idx].monitor;
        if mon_idx < self.monitors.len() {
            self.focus_first_visible_or_clear(mon_idx);
        }
        self.request_repaint();
    }

    /// Toggle the show/hide state of a single scratchpad client.
    fn switch_scratchpad_state(&mut self, idx: usize) {
        if idx >= self.clients.len() {
            return;
        }
        if self.clients[idx].is_scratchpad_show {
            self.hide_scratchpad_client(idx);
        } else {
            self.show_scratchpad_client(idx);
        }
    }

    /// Public action: toggle the visibility of a named scratchpad.
    ///
    /// `name`  — appid pattern (substring match, case-insensitive).
    /// `title` — optional title pattern. Both must match if supplied.
    /// `spawn` — shell command to launch if no running client matches;
    ///           the next call after the spawn picks up the new client
    ///           (its windowrule should set `isnamedscratchpad:1` so
    ///           it lands hidden, ready to be toggled).
    pub fn toggle_named_scratchpad(
        &mut self,
        name: Option<&str>,
        title: Option<&str>,
        spawn: Option<&str>,
    ) {
        let target = self.find_client_by_id_or_title(name, title);
        let Some(idx) = target else {
            // No matching client — spawn the launcher command if the
            // user supplied one. The just-launched client will land
            // tagged `isnamedscratchpad:1` per its windowrule and the
            // next bind press will toggle it visible.
            if let Some(cmd) = spawn.filter(|s| !s.trim().is_empty()) {
                if let Err(e) = crate::utils::spawn_shell(cmd) {
                    tracing::error!("toggle_named_scratchpad spawn '{cmd}': {e}");
                }
            }
            return;
        };

        // Mark as named scratchpad (the windowrule may already have
        // set it, but this is idempotent and lets bare keybindings
        // turn arbitrary running clients into scratchpads).
        self.clients[idx].is_named_scratchpad = true;

        // Single-scratchpad enforcement: when this config is on, only
        // ONE named scratchpad may be visible at a time. Hide every
        // other shown scratchpad on the same monitor before switching
        // the target's state.
        if self.config.single_scratchpad {
            let mon_idx = self.clients[idx].monitor;
            let to_hide: Vec<usize> = self
                .clients
                .iter()
                .enumerate()
                .filter(|(i, c)| {
                    *i != idx
                        && c.is_in_scratchpad
                        && c.is_scratchpad_show
                        && (self.config.scratchpad_cross_monitor || c.monitor == mon_idx)
                })
                .map(|(i, _)| i)
                .collect();
            for i in to_hide {
                self.hide_scratchpad_client(i);
            }
        }

        // First-time toggle: mark the client as `is_in_scratchpad` so
        // future toggles see it as a scratchpad. Then flip the
        // visibility.
        if !self.clients[idx].is_in_scratchpad {
            self.clients[idx].is_in_scratchpad = true;
            // Start the client hidden so the very first toggle reveals
            // it (mirrors mango's "set_minimized then switch_state"
            // dance).
            self.clients[idx].is_scratchpad_show = false;
            self.clients[idx].is_minimized = true;
            let window = self.clients[idx].window.clone();
            self.space.unmap_elem(&window);
        }
        self.switch_scratchpad_state(idx);
    }

    /// Public action: bring a window matching <name>/<title> to the
    /// currently-focused monitor's active tag, launching it via
    /// <spawn> if no instance is open. The mango-here.sh script
    /// implements the same flow for the C compositor; this is the
    /// in-process Rust port — no `mmsg` round-trips, no view
    /// snapshot/restore dance.
    ///
    /// Three args (mapped from the bind line):
    ///   v  → app_id pattern (regex; same matcher as windowrule appid)
    ///   v2 → optional title pattern (use `none` to skip)
    ///   v3 → spawn command run when no matching client exists
    /// Together: `bind = alt,1,summon,^Kenp$,none,start-kkenp`
    ///
    /// Hidden scratchpads are skipped — they have their own
    /// `toggle_named_scratchpad` dispatch and summoning them here
    /// would bypass the single-scratchpad enforcement.
    pub fn summon(
        &mut self,
        name: Option<&str>,
        title: Option<&str>,
        spawn: Option<&str>,
    ) {
        let target = self.find_summonable_client(name, title);
        let Some(idx) = target else {
            if let Some(cmd) = spawn.filter(|s| !s.trim().is_empty()) {
                if let Err(e) = crate::utils::spawn_shell(cmd) {
                    tracing::error!("summon spawn '{cmd}': {e}");
                }
            }
            return;
        };

        let target_mon = self.focused_monitor();
        if target_mon >= self.monitors.len() {
            return;
        }
        let target_tagset = self.monitors[target_mon].current_tagset();
        if target_tagset == 0 {
            return;
        }

        // Fast path: window is already visible on the focused monitor's
        // active tag — just refocus it. Saves a needless tag-switch
        // animation and a re-arrange when the user presses summon while
        // the target is already in front of them.
        let already_here = self.clients[idx].monitor == target_mon
            && (self.clients[idx].tags & target_tagset) != 0
            && !self.clients[idx].is_minimized;
        if already_here {
            let window = self.clients[idx].window.clone();
            self.focus_surface(Some(FocusTarget::Window(window)));
            return;
        }

        let source_mon = self.clients[idx].monitor;
        let was_minimized = self.clients[idx].is_minimized;

        self.clients[idx].old_tags = self.clients[idx].tags;
        self.clients[idx].is_tag_switching = true;
        self.clients[idx].animation.running = false;
        self.clients[idx].tags = target_tagset;
        self.clients[idx].monitor = target_mon;
        if was_minimized {
            self.clients[idx].is_minimized = false;
        }

        if source_mon != target_mon && source_mon < self.monitors.len() {
            self.arrange_monitor(source_mon);
        }
        self.arrange_monitor(target_mon);

        let window = self.clients[idx].window.clone();
        self.focus_surface(Some(FocusTarget::Window(window)));

        crate::protocols::dwl_ipc::broadcast_monitor(self, target_mon);
        if source_mon != target_mon && source_mon < self.monitors.len() {
            crate::protocols::dwl_ipc::broadcast_monitor(self, source_mon);
        }
    }

    /// Like `find_client_by_id_or_title` but skips hidden scratchpads —
    /// summoning them would conflict with the named-scratchpad toggle
    /// dispatch and bypass single_scratchpad enforcement.
    fn find_summonable_client(
        &self,
        name: Option<&str>,
        title: Option<&str>,
    ) -> Option<usize> {
        let name_pat = name.unwrap_or("");
        let title_pat = title.unwrap_or("");
        for (idx, c) in self.clients.iter().enumerate() {
            if c.is_in_scratchpad && !c.is_scratchpad_show {
                continue;
            }
            let app_match = if name_pat.is_empty() {
                true
            } else {
                matches_rule_text(name_pat, &c.app_id)
            };
            let title_match = if title_pat.is_empty() {
                true
            } else {
                matches_rule_text(title_pat, &c.title)
            };
            if app_match && title_match {
                return Some(idx);
            }
        }
        None
    }

    /// Public action: full reset of the focused client back to
    /// a normal tile. Bind this to an emergency-recovery key
    /// (the user has it on `super+ctrl,Escape`) for any time a
    /// window ends up in a state the standard binds can't get it
    /// out of — accidental scratchpad promotion, sticky floating
    /// because some popup left it that way, fullscreen
    /// stuck-on, the list goes on. Mirrors mango-ext's "exit
    /// scratchpad" but also drops the floating / fullscreen /
    /// minimised flags so the next arrange treats the window as a
    /// vanilla tiled toplevel. Cheaper and more reliable than
    /// chasing the specific flag that's misbehaving.
    pub fn unscratchpad_focused(&mut self) {
        let Some(idx) = self.focused_client_idx() else { return };
        let already_normal = !self.clients[idx].is_in_scratchpad
            && !self.clients[idx].is_named_scratchpad
            && !self.clients[idx].is_scratchpad_show
            && !self.clients[idx].is_floating
            && !self.clients[idx].is_fullscreen
            && !self.clients[idx].is_maximized_screen
            && !self.clients[idx].is_minimized;
        if already_normal {
            return;
        }
        let c = &mut self.clients[idx];
        let app_id = c.app_id.clone();
        let snapshot = (
            c.is_in_scratchpad,
            c.is_scratchpad_show,
            c.is_named_scratchpad,
            c.is_minimized,
            c.is_floating,
            c.is_fullscreen,
            c.is_maximized_screen,
        );
        c.is_in_scratchpad = false;
        c.is_scratchpad_show = false;
        c.is_named_scratchpad = false;
        c.is_minimized = false;
        c.is_floating = false;
        c.is_fullscreen = false;
        c.is_maximized_screen = false;
        let mon_idx = c.monitor;
        let window = c.window.clone();
        let geom = c.geom;

        // Re-map the surface (scratchpad hide had unmapped it from
        // the scene). Active tagset already covers the recovered
        // window since `is_visible_on`'s scratchpad-guard no longer
        // suppresses it.
        self.space.map_element(window.clone(), (geom.x, geom.y), true);
        self.arrange_monitor(mon_idx);
        self.focus_surface(Some(FocusTarget::Window(window)));
        tracing::info!(
            "unscratchpad: recovered app_id={} from \
             (in_scratch={}, scratch_show={}, named_scratch={}, \
              minimized={}, floating={}, fullscreen={}, max_screen={})",
            app_id,
            snapshot.0,
            snapshot.1,
            snapshot.2,
            snapshot.3,
            snapshot.4,
            snapshot.5,
            snapshot.6,
        );
    }

    /// Public action: toggle the *anonymous* scratchpad set — every
    /// client previously promoted to a scratchpad via the legacy
    /// `toggle_scratchpad` command (no name, no title, just "stash
    /// the current focused window"). Mirrors mango's implementation
    /// faithfully enough that the pattern carries over.
    pub fn toggle_scratchpad(&mut self) {
        if let Some(mon_idx) = self
            .focused_client_idx()
            .map(|i| self.clients[i].monitor)
        {
            // First pass: if any anonymous scratchpad is currently
            // shown, hide them all. (single_scratchpad makes this
            // mostly the same as toggle_named, just keyed off the
            // anonymous flag.)
            let mut hit = false;
            let to_toggle: Vec<usize> = (0..self.clients.len())
                .filter(|&i| {
                    let c = &self.clients[i];
                    !c.is_named_scratchpad
                        && (self.config.scratchpad_cross_monitor || c.monitor == mon_idx)
                })
                .collect();

            for i in to_toggle {
                let c = &self.clients[i];
                if self.config.single_scratchpad
                    && c.is_named_scratchpad
                    && !c.is_minimized
                {
                    self.clients[i].is_minimized = true;
                    let window = self.clients[i].window.clone();
                    self.space.unmap_elem(&window);
                    continue;
                }
                if c.is_named_scratchpad {
                    continue;
                }
                if hit {
                    continue;
                }
                if c.is_in_scratchpad {
                    self.switch_scratchpad_state(i);
                    hit = true;
                }
            }
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
        self.notify_layout_state(
            "gaps",
            if self.enable_gaps { "on" } else { "off" },
        );
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
            let (app_id, title) = read_toplevel_identity(toplevel);
            self.clients[idx].app_id = app_id;
            self.clients[idx].title = title;
        }

        // Now run rules with the live app_id/title.
        let _changed = self.reapply_rules(idx, WindowRuleReason::InitialMap);

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

        // Named scratchpad bootstrap. If the windowrule flagged this
        // client as a named scratchpad (mango's `isnamedscratchpad:1`
        // pattern), promote it to a *visible* scratchpad on first
        // map: `is_in_scratchpad = true`, `is_scratchpad_show = true`,
        // float_geom from the rule, focus retained. The user-side
        // mental model is "press the bind → my scratchpad appears
        // here", so the very first press of the toggle key (which
        // spawned the app in the first place because nothing was
        // running) MUST land a visible window. Subsequent presses
        // toggle hide / show via the regular `switch_scratchpad_state`
        // path.
        //
        // Earlier this branch tucked the freshly-spawned client away
        // (unmap + is_minimized) on the theory that the spawn-cmd
        // and the visibility toggle were two separate steps. They
        // aren't on the user side — pressing the bind once should
        // result in a visible scratchpad; pressing again should
        // hide it. Only the second-and-later cycles go through
        // toggle_named_scratchpad's switch_scratchpad_state branch.
        if self.clients[idx].is_named_scratchpad
            && !self.clients[idx].is_in_scratchpad
        {
            self.clients[idx].is_in_scratchpad = true;
            self.clients[idx].is_scratchpad_show = true;
            self.clients[idx].is_floating = true;
            // Don't unmap — leave the window where finalize_initial_map's
            // own map_element / arrange_monitor placed it. The
            // windowrule's float_geom (offsetx/offsety/width/height)
            // already drove that placement, so the visible result
            // matches the show_scratchpad_client positioning we'd
            // otherwise apply on a subsequent toggle.
            tracing::info!(
                "named_scratchpad bootstrap: app_id={} visible from first map",
                self.clients[idx].app_id,
            );
        }

        // Note: clients that mapped onto a non-active tag intentionally
        // get NO bootstrap configure here. An earlier version of this
        // code seeded `c.geom` with the monitor's work area and sent a
        // matching configure so the client could commit at "some size"
        // during launch. That actively hurt: XWayland clients (Spotify
        // is the canonical case) commit at the bootstrap size, cache
        // it as their natural extent, and then resist the smaller
        // configure the eventual tag-switch arrange tries to send —
        // the surface stays stuck at the larger bootstrap size and the
        // `clipped_surface` shader ends up cropping the right / bottom
        // of the visible content. Leaving `c.geom` at the default zero
        // rect lets `view_tag`'s "skip tag-in staging when c.geom is
        // degenerate" branch fall through to `arrange_monitor`'s
        // direct-snap path, which sends a *first* configure at the
        // real slot size. Native Wayland clients always honour that
        // first configure; XWayland clients honour it far more
        // reliably than a subsequent shrink.


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

        // Phase 3 scripting: invoke `on_window_open` handlers now
        // that app_id / title / window-rules have all settled. A
        // handler that calls `focused_appid()` sees the just-mapped
        // window's identity, and dispatches like `tagview` /
        // `togglefloating` apply to it because focus has already
        // been pushed to it earlier in this function.
        crate::scripting::fire_window_open(self);

        // Notify xdp-gnome's window picker so a live screencast
        // share dialog refreshes its list while open.
        self.emit_windows_changed();
    }
}


// ── Smithay delegate: XDG decoration ─────────────────────────────────────────

impl MargoState {
    /// What decoration mode should we send to a freshly-bound or
    /// reset toplevel? Defaults to `ServerSide`; flips to
    /// `ClientSide` only when the client is in our `clients` vec
    /// and matches a window-rule that whitelists CSD. At the time
    /// `new_decoration` fires the toplevel may not even be in
    /// `clients` yet (xdg-decoration arrives before the first
    /// commit), in which case we ALSO check the raw window-rule
    /// list against the toplevel's current app_id / title — the
    /// rule machinery would otherwise only kick in at
    /// `finalize_initial_map`, too late to influence the very first
    /// configure.
    fn decoration_mode_for(&self, toplevel: &ToplevelSurface) -> XdgDecorationMode {
        if self.client_allows_csd(toplevel) {
            XdgDecorationMode::ClientSide
        } else {
            XdgDecorationMode::ServerSide
        }
    }

    fn client_allows_csd(&self, toplevel: &ToplevelSurface) -> bool {
        let wl_surface = toplevel.wl_surface();
        // Path A: client already mapped — read the resolved
        // `allow_csd` flag right off the `MargoClient`.
        if let Some(client) = self.clients.iter().find(|c| {
            c.window.wl_surface().as_deref() == Some(wl_surface)
        }) {
            return client.allow_csd;
        }
        // Path B: client is between role bind and first commit —
        // best we can do is look up the window-rule by the
        // toplevel's currently-set app_id / title. This is the
        // path that fires for the *first* `xdg_decoration.configure`
        // many compositors get wrong (Chromium / Firefox often
        // bind decoration before any role-data commit, so the
        // initial mode the user sees depends entirely on what we
        // send right now).
        let (app_id, title) = read_toplevel_identity(toplevel);
        self.config
            .window_rules
            .iter()
            .filter(|rule| self.window_rule_matches(rule, &app_id, &title))
            .any(|rule| rule.allow_csd == Some(true))
    }
}

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
        // Refresh xdp-gnome's window picker — same path the
        // Wayland finalize_initial_map handler uses.
        self.emit_windows_changed();
    }

    fn remove_x11_window(&mut self, x11surface: &X11Surface) {
        if let Some(idx) = self.find_x11_client(x11surface) {
            let app_id = self.clients[idx].app_id.clone();
            let title = self.clients[idx].title.clone();
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
            // Refresh xdp-gnome's window picker — same path the
            // Wayland toplevel_destroyed handler uses.
            self.emit_windows_changed();
            crate::scripting::fire_window_close(self, &app_id, &title);
        }
    }
}


// ── Smithay delegate: Viewporter ───────────────────────────────────────────────

smithay::delegate_viewporter!(MargoState);

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


delegate_presentation!(MargoState);

