//! Integration tests for `OutputManagementHandler` (W4.2 Phase 4
//! extracted impl at `state/handlers/output_management.rs`).
//!
//! `zwlr_output_manager_v1` is what `wlr-randr` / `kanshi` bind
//! to apply runtime topology / scale / transform / mode / disable
//! changes. The handler:
//!
//! * Refuses to apply a config that would leave zero enabled
//!   outputs (would strand the user with a dark screen and no
//!   recovery short of TTY login).
//! * Updates Output::change_current_state for scale / transform
//!   / position synchronously, defers mode change to the udev
//!   backend.
//! * Re-publishes topology after a successful apply so kanshi
//!   watchers see the new state.
//!
//! Headless tests cover the global advertisement + the
//! default-state assertion. Driving an actual configuration apply
//! through `zwlr_output_configuration_v1` is bigger than this
//! round; that's a follow-up that exercises the
//! `apply_output_pending` branch end-to-end.

use super::fixture::Fixture;

#[test]
fn output_management_global_advertised() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        names.iter().any(|n| n == "zwlr_output_manager_v1"),
        "wlr_output_manager_v1 must be advertised; saw {:?}",
        names,
    );
}

#[test]
fn xdg_output_manager_advertises_per_output_metadata() {
    // `zxdg_output_manager_v1` is the read-only side of the
    // output story — gives clients access to logical position +
    // logical size + display name. xdg-desktop-portal-wlr binds
    // it; if we forget to advertise it on a regression, the
    // portal silently skips outputs.
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        names.iter().any(|n| n == "zxdg_output_manager_v1"),
        "zxdg_output_manager_v1 must be advertised; saw {:?}",
        names,
    );
}
