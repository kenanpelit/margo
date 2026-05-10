//! Integration coverage for `ColorManagementHandler` (W4.2 Phase 2
//! extracted impl at `state/handlers/color_management.rs`).
//!
//! HDR Phase 1 ships the **protocol module + handler scaffolding
//! only**. The `wp_color_manager_v1` global creation is currently
//! commented out at `protocols/color_management.rs` line ~141
//! pending Phase 2 (linear-light fp16 composite). When that gate
//! flips, the global must appear in the advertised set and these
//! tests should be updated to assert the positive presence.
//!
//! Today the test pins the negative invariant: the global is NOT
//! advertised. Catches the class of regression where someone
//! accidentally uncomments the bind without finishing Phase 2 —
//! HDR-aware clients (Chromium / mpv) would detect a colour-
//! managed compositor, try the HDR decode path, and end up
//! tone-mapping back to sRGB twice.

use super::fixture::Fixture;

#[test]
fn color_management_global_not_advertised_until_phase_2() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        !names.iter().any(|n| n.starts_with("wp_color_manager") || n.starts_with("xx_color_manager")),
        "Phase 2 of HDR isn't shipped yet; the color-management global must stay gated. \
         Saw advertised globals: {:?}",
        names,
    );
}

#[test]
fn color_management_state_exists_for_phase_2_wireup() {
    // The ColorManagementState struct lives on MargoState as the
    // home for the manager state Phase 2 will publish. Pin its
    // existence so a future "delete unused field" cleanup pass
    // doesn't silently break the Phase 2 unblock.
    let fx = Fixture::new();
    let _ = &fx.server.state.color_management_state;
}
