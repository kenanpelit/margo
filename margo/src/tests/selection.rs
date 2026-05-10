//! Integration tests for the selection-family handlers (W4.2
//! Phase 3 extracted impls at `state/handlers/selection.rs`).
//!
//! Bundled: `wl_data_device_manager` (clipboard / drag-drop),
//! `wp_primary_selection` (middle-click paste), `wlr_data_control`
//! (clipboard managers — CopyQ / cliphist / clipse), the bridging
//! `SelectionHandler` (Wayland ↔ XWayland selection mirror), and
//! `DndGrabHandler`.
//!
//! Driving a full selection round-trip needs a focused client
//! (so `SeatHandler::focus_changed` fires `set_data_device_focus`),
//! which the headless harness can produce by mapping a toplevel.
//! The actual selection content + mime-type negotiation is a
//! bigger lift — these tests cover the global-advertisement +
//! state-init surface so a regression that drops a delegate
//! macro from the W4.2 split shows up at PR time.

use super::fixture::Fixture;

#[test]
fn all_selection_globals_advertised() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    for required in &[
        "wl_data_device_manager",
        "zwp_primary_selection_device_manager_v1",
        "zwlr_data_control_manager_v1",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "{required} must be advertised; saw {:?}",
            names,
        );
    }
}

#[test]
fn data_device_state_initializes_cleanly() {
    // The DataDeviceState struct lives on MargoState and is
    // initialized in MargoState::new. This pins that the field
    // exists + the constructor didn't panic on the
    // delegate_data_device! macro path. Catches regressions where
    // a future Phase-X extraction silently forgets to
    // re-instantiate the state.
    let fx = Fixture::new();
    // Smoke: state field is reachable. Its internals are private
    // to smithay's selection module; we just ensure it's there.
    let _ = &fx.server.state.data_device_state;
    let _ = &fx.server.state.primary_selection_state;
    let _ = &fx.server.state.data_control_state;
}
