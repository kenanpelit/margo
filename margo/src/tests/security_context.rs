//! Sandboxed (restricted) clients.
//!
//! Clients that connect through a `wp_security_context_v1` listener —
//! Flatpak apps and other sandboxes — must not see the globals that would
//! let them escape the sandbox: read the screen or the clipboard, inject
//! input, lock/unlock the session, reconfigure outputs, list every window.
//! Same set niri hides (plus margo's extra ext-foreign-toplevel-list,
//! toplevel capture source and output-power globals).

use super::fixture::Fixture;

/// Globals a restricted client must not see.
const PRIVILEGED: &[&str] = &[
    "zwlr_layer_shell_v1",
    "ext_session_lock_manager_v1",
    "zwlr_data_control_manager_v1",
    "ext_data_control_manager_v1",
    "wp_security_context_manager_v1",
    "zwp_input_method_manager_v2",
    "zwp_virtual_keyboard_manager_v1",
    "zwlr_virtual_pointer_manager_v1",
    "zwlr_foreign_toplevel_manager_v1",
    "ext_foreign_toplevel_list_v1",
    "ext_workspace_manager_v1",
    "zwlr_output_manager_v1",
    "zwlr_output_power_manager_v1",
    "zwlr_screencopy_manager_v1",
    "ext_output_image_capture_source_manager_v1",
    "ext_foreign_toplevel_image_capture_source_manager_v1",
    "ext_image_copy_capture_manager_v1",
    "zwlr_gamma_control_manager_v1",
];

/// Globals an ordinary app needs; a restricted client keeps them.
const ORDINARY: &[&str] = &[
    "wl_compositor",
    "wl_shm",
    "wl_seat",
    "wl_data_device_manager",
    "xdg_wm_base",
    "zxdg_decoration_manager_v1",
    "wp_viewporter",
    "zwp_text_input_manager_v3",
    "xdg_activation_v1",
];

#[test]
fn an_unrestricted_client_sees_the_privileged_globals() {
    // Guards the names in PRIVILEGED: each one must really exist, or the
    // restricted test below would pass vacuously.
    let mut fx = Fixture::new();
    let id = fx.add_client();
    let names = fx.client(id).global_names();
    let missing: Vec<_> = PRIVILEGED
        .iter()
        .filter(|g| !names.iter().any(|n| n == *g))
        .collect();
    assert!(missing.is_empty(), "not advertised at all: {missing:?}");
}

#[test]
fn a_restricted_client_does_not_see_privileged_globals() {
    let mut fx = Fixture::new();
    let id = fx.add_restricted_client();
    let names = fx.client(id).global_names();
    let leaked: Vec<_> = PRIVILEGED
        .iter()
        .filter(|g| names.iter().any(|n| n == *g))
        .collect();
    assert!(
        leaked.is_empty(),
        "visible to a sandboxed client: {leaked:?}"
    );
}

#[test]
fn a_restricted_client_keeps_the_ordinary_globals() {
    let mut fx = Fixture::new();
    let id = fx.add_restricted_client();
    let names = fx.client(id).global_names();
    let missing: Vec<_> = ORDINARY
        .iter()
        .filter(|g| !names.iter().any(|n| n == *g))
        .collect();
    assert!(
        missing.is_empty(),
        "hidden from a sandboxed client: {missing:?}"
    );
}
