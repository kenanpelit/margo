//! Integration tests for `PointerConstraintsHandler` (W4.2 Phase 2
//! extracted impl at `state/handlers/pointer_constraints.rs`).
//!
//! `wp_pointer_constraints_v1` is what FPS games / Blender / 3D
//! viewports bind to lock or confine the cursor. Margo's handler
//! is split: this file wires the protocol surface; actual
//! enforcement lives in `input_handler::handle_pointer_motion`.
//!
//! The activation half (a constraint becomes ACTIVE only when the
//! constrained surface holds pointer focus) needs a real pointer
//! focus state machine which the headless harness doesn't drive.
//! These tests are scoped to "the handler doesn't panic on
//! lock_pointer when no pointer focus is set" — that's the
//! regression class margo's own commit history has actually had.

use super::fixture::Fixture;

#[test]
fn pointer_constraints_global_advertised() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        names.iter().any(|n| n == "zwp_pointer_constraints_v1"),
        "pointer_constraints global must be available; saw {:?}",
        names,
    );
}

#[test]
fn lock_pointer_without_focus_does_not_panic() {
    // The handler tries to activate the constraint immediately if
    // pointer focus is on the surface; with no focus, it skips —
    // but the protocol object still has to be created cleanly.
    // Catches "early-return short-circuited cleanup" regressions.
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));
    let id = fx.add_client();

    let (_compositor, surface) = fx.client(id).create_surface();
    let pointer = fx.client(id).create_pointer();
    let _locked = fx.client(id).lock_pointer(&surface, &pointer);
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    // The compositor survived the request; that's the assertion.
    // (Activation requires real pointer focus; not driven here.)
}
