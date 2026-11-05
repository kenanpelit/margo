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

// ── ConfigureRequest decisions (pure; no XWayland needed) ──────────────────

use crate::layout::Rect;
use crate::state::{X11ConfigureReply, X11ManagedGeometry, x11_configure_reply};

fn managed(is_floating: bool, fullscreen: bool) -> Option<X11ManagedGeometry> {
    Some(X11ManagedGeometry {
        is_floating,
        fullscreen,
        geom: Rect::new(100, 100, 400, 300),
    })
}

const CURRENT: Rect = Rect {
    x: 100,
    y: 100,
    width: 400,
    height: 300,
};

#[test]
fn a_floating_x11_window_self_configure_is_kept_as_its_float_geometry() {
    // A Wine / Steam / Java dialog growing to fit its content: the new rect
    // must become the floating geometry, or the next arrange snaps it back.
    assert_eq!(
        x11_configure_reply(
            managed(true, false),
            CURRENT,
            Some(150),
            None,
            Some(640),
            Some(480)
        ),
        X11ConfigureReply::Float(Rect::new(150, 100, 640, 480))
    );
}

#[test]
fn a_floating_x11_self_configure_is_clamped() {
    assert_eq!(
        x11_configure_reply(
            managed(true, false),
            CURRENT,
            Some(i32::MIN),
            Some(i32::MAX),
            Some(1),
            Some(u32::MAX)
        ),
        X11ConfigureReply::Float(Rect::new(-(1 << 20), 1 << 20, 50, 1 << 20))
    );
}

#[test]
fn tiled_and_fullscreen_x11_windows_keep_the_compositor_geometry() {
    for (floating, fullscreen) in [(false, false), (false, true), (true, true)] {
        assert_eq!(
            x11_configure_reply(
                managed(floating, fullscreen),
                CURRENT,
                Some(0),
                Some(0),
                Some(10),
                Some(10)
            ),
            X11ConfigureReply::Reassert(CURRENT),
            "floating={floating} fullscreen={fullscreen}"
        );
    }
}

#[test]
fn an_unmanaged_x11_window_gets_its_request_echoed() {
    assert_eq!(
        x11_configure_reply(None, CURRENT, None, Some(20), None, Some(200)),
        X11ConfigureReply::Echo(Rect::new(100, 20, 400, 200))
    );
}
