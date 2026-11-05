//! Seat, SHM and output handlers: keyboard/pointer/touch focus types,
//! clipboard focus routing, cursor image updates.

use smithay::{
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    wayland::{
        output::OutputHandler,
        seat::WaylandFocus,
        selection::{data_device::set_data_device_focus, primary_selection::set_primary_focus},
        shm::{ShmHandler, ShmState},
    },
};

use crate::state::{FocusTarget, MargoState};

// ── Smithay delegate: SHM ────────────────────────────────────────────────────

impl ShmHandler for MargoState {
    fn shm_state(&self) -> &ShmState {
        &self.protocols.shm
    }
}

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
    /// Touch focus is the drilled `WlSurface` under each finger, for the
    /// same reason as the pointer's.
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<MargoState> {
        &mut self.seat_state
    }
    fn led_state_changed(
        &mut self,
        _seat: &Seat<Self>,
        led_state: smithay::input::keyboard::LedState,
    ) {
        self.sync_keyboard_leds(led_state);
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
        // Browsers re-send the same shape on every hover; an identical named
        // or hidden cursor changes no pixel. A surface cursor always counts:
        // `set_cursor` with the same surface may carry a new hotspot.
        if image == self.cursor_status && !matches!(image, CursorImageStatus::Surface(_)) {
            return;
        }
        // A named request (wp_cursor_shape_v1 / set_cursor → e.g. the `pointer`
        // hand over a link) only changes anything if we point the cursor
        // manager at that icon — the `Named` render path draws whatever it
        // currently holds.
        if let CursorImageStatus::Named(icon) = &image {
            self.cursor_manager.set_named(icon.name(), icon.alt_names());
        }
        if let CursorImageStatus::Surface(surface) = &image {
            self.send_pointer_output_scale_transform(surface);
        }
        self.cursor_status = image;
        self.request_cursor_image_repaint();
    }
}

// `wp_cursor_shape_v1`: clients send a shape name instead of their
// own cursor surface; we draw via `cursor_manager` at our own size.
// Required for GTK4 layer-shell surfaces to avoid oversized cursor.
// (TabletSeatHandler lives in state/handlers/tablet_manager.rs.)

// ── Smithay delegate: Output ──────────────────────────────────────────────────

impl OutputHandler for MargoState {}
