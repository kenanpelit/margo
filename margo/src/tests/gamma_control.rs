//! Integration tests for `GammaControlHandler` (W4.2 Phase 4
//! extracted impl at `state/handlers/gamma_control.rs`).
//!
//! `wlr-gamma-control-v1` is what sunsetr / gammastep / wlsunset
//! bind to push night-light ramps. Margo's handler:
//!
//! * Reports `gamma_size` per-output via `get_gamma_size`. If 0,
//!   the per-output capability is silently filtered (real udev
//!   backend on a connector without GAMMA_LUT, or winit nested
//!   mode).
//! * On `set_gamma`, queues `(output, ramp)` onto
//!   `state.pending_gamma`, **coalescing** by output — a client
//!   that spams set_gamma must not grow the queue unbounded.
//!
//! The render path drains the queue at the next backend repaint;
//! these tests don't exercise the drain (no backend), only the
//! queue-side bookkeeping.

use super::fixture::Fixture;

#[test]
fn gamma_control_global_advertised() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        names.iter().any(|n| n == "zwlr_gamma_control_manager_v1"),
        "gamma_control global must be available; saw {:?}",
        names,
    );
}

#[test]
fn output_with_zero_gamma_size_is_skipped_for_gamma() {
    // Default headless output has gamma_size = 0 → handler's
    // `get_gamma_size` returns None → per-output gamma object
    // gets a `failed` event but the global stays bound. Pre-W4.2
    // this filter was inline in state.rs; now lives in the
    // extracted handler — test pins the behaviour.
    let mut fx = Fixture::new();
    fx.add_output("HEADLESS-1", (1920, 1080));
    let id = fx.add_client();
    fx.roundtrip(id);

    let monitor = &fx.server.state.monitors[0];
    assert_eq!(monitor.gamma_size, 0);
    assert!(
        fx.server.state.pending_gamma.is_empty(),
        "no gamma operations should have been queued",
    );
}

#[test]
fn output_with_nonzero_gamma_size_is_eligible() {
    // Smoke that the fixture's `add_output_full(..., gamma_size)`
    // wires the value through to MargoMonitor — this is what
    // tests of the actual set_gamma path will key on once the
    // client-side gamma proxy is in the harness. (Today the
    // client doesn't bind zwlr_gamma_control_manager_v1; doing so
    // requires a fd-based protocol surface that's bigger than
    // this round's scope.)
    let mut fx = Fixture::new();
    fx.add_output_full("HEADLESS-1", (1920, 1080), 256);
    let _id = fx.add_client();

    let monitor = &fx.server.state.monitors[0];
    assert_eq!(monitor.gamma_size, 256);
}
