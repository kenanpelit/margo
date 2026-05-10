//! Integration coverage for `XwmHandler` + `XWaylandShellHandler`
//! (W4.2 Phase 5 extracted impl at `state/handlers/x11.rs`).
//!
//! `xwayland_shell_v1` is the wayland-side anchor smithay needs
//! to track which `wl_surface` belongs to which X11 window. The
//! global is **only stood up when XWayland actually starts** —
//! the in-tree `XWayland::spawn(...)` path or the
//! `--xwayland-satellite[=BINARY]` companion process. Headless
//! tests don't spawn either, so the global isn't advertised and
//! `state.xwm` stays `None`.
//!
//! `XwmHandler` itself fires from X11Wm callbacks; without a
//! running Xwayland subprocess + a real X11 client there's
//! nothing to call into the handler. Coverage of the actual map
//! / unmap / configure / selection-bridging paths needs an
//! `XWayland::spawn` + DISPLAY socket setup, which is doable but
//! belongs in a separate "x11_with_subprocess" test module
//! shielded behind a `#[cfg]` so it doesn't run in basic CI.

use super::fixture::Fixture;

#[test]
fn xwayland_shell_global_not_advertised_when_xwayland_inactive() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        !names.iter().any(|n| n == "xwayland_shell_v1"),
        "without an XWayland subprocess running, the xwayland_shell global must stay gated; saw {:?}",
        names,
    );
}

#[test]
fn xwm_starts_as_none_in_headless_mode() {
    // The compositor's X11Wm handle is only Some(...) once
    // smithay's XWayland spawn callback runs and the X11 server
    // is ready. Pin that the headless path leaves it None so
    // tests of the (rare) "is xwayland up?" branch always
    // observe the canonical negative state.
    let fx = Fixture::new();
    assert!(
        fx.server.state.xwm.is_none(),
        "without XWayland::spawn, state.xwm must be None",
    );
}
