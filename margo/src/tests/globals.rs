//! First integration test: a fresh client sees every global the
//! compositor advertises at startup.
//!
//! What this catches:
//!
//! * Cross-handler regressions where a recent extraction (W4.2)
//!   forgot to call the relevant `delegate_*!` macro — the global
//!   wouldn't bind, the client wouldn't see it, this test would
//!   fail.
//! * Off-by-one breakage in `MargoState::new`'s registration
//!   order: globals registered after the listening socket opens
//!   are visible only to *post-startup* clients. We don't open a
//!   socket here; clients are inserted directly via
//!   `display_handle.insert_client`, but the same bug class
//!   shows up.
//!
//! It is intentionally permissive about *additional* globals so
//! tests don't break every time a new protocol module lands —
//! only the protocol surface margo *guarantees* is checked.

use super::fixture::Fixture;

/// Globals every margo build is expected to advertise on a fresh
/// client connection in **headless test mode** (no DRM backend, no
/// XWayland). Anything in this list is part of margo's guaranteed
/// public surface for clients (Firefox / Chromium / noctalia / mpv
/// / kitty) — adding here means promising the global is always
/// available; removing means deliberately taking a feature out.
///
/// **Deliberately NOT in this list:**
/// * `xwayland_shell_v1` — only stood up when XWayland actually
///   spawns; headless tests don't.
/// * `wp_color_manager_v1` — HDR Phase 1 protocol module is
///   compiled but the global is gated until Phase 2 (see
///   `protocols/color_management.rs` line 141 comment). When the
///   gate flips, add it here.
/// * `ext_image_capture_source_manager_v1` — there's no top-level
///   manager global; only the per-source-type sub-managers
///   (output / foreign_toplevel) and the copy-capture manager are
///   bindable. The sub-managers ARE in this list below.
/// * `wp_linux_dmabuf_v1` / `wp_linux_drm_syncobj_manager_v1` —
///   only advertised when the udev backend has imported a real
///   DRM node. Headless tests skip the backend so neither global
///   is up.
const REQUIRED_GLOBALS: &[&str] = &[
    "wl_compositor",
    "wl_subcompositor",
    "wl_shm",
    "wl_seat",
    "wl_data_device_manager",
    "xdg_wm_base",
    "wp_viewporter",
    "zxdg_decoration_manager_v1",
    "zwlr_layer_shell_v1",
    "zwp_primary_selection_device_manager_v1",
    "zwlr_data_control_manager_v1",
    "zwp_pointer_constraints_v1",
    "zwp_relative_pointer_manager_v1",
    "ext_idle_notifier_v1",
    "zwp_idle_inhibit_manager_v1",
    "wp_presentation",
    "ext_session_lock_manager_v1",
    "xdg_activation_v1",
    "zwp_text_input_manager_v3",
    "zwp_input_method_manager_v2",
    "zwlr_gamma_control_manager_v1",
    "zwlr_screencopy_manager_v1",
    "ext_output_image_capture_source_manager_v1",
    "ext_foreign_toplevel_image_capture_source_manager_v1",
    "ext_image_copy_capture_manager_v1",
    "ext_foreign_toplevel_list_v1",
    "zdwl_ipc_manager_v2",
    "zxdg_output_manager_v1",
    "zwlr_output_manager_v1",
];

#[test]
fn fresh_client_sees_all_required_globals() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let advertised = fx.client(id).global_names();

    let mut missing: Vec<&str> = REQUIRED_GLOBALS
        .iter()
        .copied()
        .filter(|name| !advertised.iter().any(|adv| adv == name))
        .collect();

    if !missing.is_empty() {
        missing.sort_unstable();
        panic!(
            "compositor failed to advertise {} required global(s):\n  {}\n\nadvertised:\n  {}",
            missing.len(),
            missing.join("\n  "),
            advertised.join("\n  ")
        );
    }
}

#[test]
fn second_client_sees_the_same_globals_as_the_first() {
    // Two clients connecting back-to-back must see an identical
    // global set. A regression here would mean a global was
    // registered late (post-first-bind) — clients connected
    // earlier wouldn't see it. The fixture inserts both clients
    // before any are dispatched, so this asserts the registration
    // order in `MargoState::new` is observed atomically.
    let mut fx = Fixture::new();
    let id_a = fx.add_client();
    let id_b = fx.add_client();
    fx.roundtrip(id_a);
    fx.roundtrip(id_b);
    let names_a = fx.client(id_a).global_names();
    let names_b = fx.client(id_b).global_names();
    let mut sorted_a = names_a.clone();
    let mut sorted_b = names_b.clone();
    sorted_a.sort();
    sorted_b.sort();
    assert_eq!(
        sorted_a, sorted_b,
        "client B saw a different global set than client A (set diverged after first bind)",
    );
}

#[test]
fn fixture_dispatches_without_clients() {
    // Pure-server smoke: stand up a Fixture, call dispatch a few
    // times without inserting any clients. Catches a regression
    // where MargoState::new panicked on a missing initial output
    // (we don't add any here) or where dispatch deadlocked when
    // the client list was empty.
    let mut fx = Fixture::new();
    for _ in 0..5 {
        fx.dispatch();
    }
}
