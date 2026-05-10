//! Integration tests for `ScreencopyHandler` (W4.2 Phase 4
//! extracted impl at `state/handlers/screencopy.rs`).
//!
//! `zwlr-screencopy-unstable-v1` is what `grim` / `wf-recorder` /
//! OBS bind for full-output / region capture. Margo's handler
//! defers the actual buffer copy to the backend's render path:
//! `frame()` pushes the screencopy onto `state.screencopy_state`,
//! which the udev backend drains at the next repaint.
//!
//! Without a render backend the drain side is unobservable, but
//! the bind / push surface IS — that's what these smoke tests
//! pin.

use super::fixture::Fixture;

#[test]
fn screencopy_global_advertised() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        names.iter().any(|n| n == "zwlr_screencopy_manager_v1"),
        "screencopy global must be available; saw {:?}",
        names,
    );
}

#[test]
fn ext_image_copy_capture_globals_advertised() {
    // The newer ext-image-copy-capture stack (Phase 7 work) sits
    // alongside the wlr-screencopy global; both must bind so
    // xdp-wlr 0.8+ can pick the modern path while older clients
    // (grim) still hit the wlr one.
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    for required in &[
        "ext_output_image_capture_source_manager_v1",
        "ext_foreign_toplevel_image_capture_source_manager_v1",
        "ext_image_copy_capture_manager_v1",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "{required} must be advertised; saw {:?}",
            names,
        );
    }
}
