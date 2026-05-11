//! Keyboard / pointer / touch / DnD focus target — the
//! `MargoState::seat` focus channel's payload.
//!
//! Extracted from `state.rs` (roadmap Q1). One enum + a handful of
//! smithay trait delegations, all in the "forward to the inner
//! wl_surface" shape. Lifts cleanly because the bodies don't touch
//! `MargoState` itself; they delegate to the surface's own impl.
//!
//! Re-exported as `crate::state::FocusTarget` so existing callers
//! don't move.

use std::sync::Arc;

use smithay::{
    desktop::{Window, WindowSurface},
    input::{
        dnd::{DndFocus, Source},
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
            MotionEvent, PointerTarget, RelativeMotionEvent,
        },
        touch::TouchTarget,
        Seat,
    },
    reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    utils::{Logical, Point, Serial},
    wayland::{
        seat::WaylandFocus,
        selection::data_device::WlOfferData,
        session_lock::LockSurface,
        shell::wlr_layer::LayerSurface as WlrLayerSurface,
    },
};

use super::MargoState;

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
    pub(crate) fn inner_wl_surface(&self) -> Option<&WlSurface> {
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
        // Debug level: this fires on every sloppy-focus crossing and
        // every overview hover sweep. INFO floods the journal in
        // normal use; the structured `target` field lets
        // `journalctl --output=json | jq` slice cleanly when the
        // user actually wants to debug focus routing.
        tracing::debug!(target = ?self, "FocusTarget::enter");
        if let Some(s) = self.inner_wl_surface() {
            tracing::debug!("FocusTarget::enter forwarding to WlSurface");
            KeyboardTarget::enter(s, seat, data, keys, serial);
        }
    }
    fn leave(&self, seat: &Seat<MargoState>, data: &mut MargoState, serial: Serial) {
        tracing::debug!(target = ?self, "FocusTarget::leave");
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
