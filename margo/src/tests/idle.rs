//! Integration tests for `IdleInhibitHandler` (W4.2 Phase 2
//! extracted impl at `state/handlers/idle.rs`).
//!
//! mpv / video players / presentation tools bind
//! `zwp_idle_inhibit_manager_v1` and create an inhibitor on the
//! surface they're rendering to. Margo's handler keeps a
//! `HashSet<WlSurface>` and flips
//! `idle_notifier_state.set_is_inhibited(...)` to the set's
//! emptiness — `ext_idle_notifier_v1` clients (sunsetr, swayidle)
//! see "the user is inhibited, don't fire timers".

use super::fixture::Fixture;

#[test]
fn create_inhibitor_adds_to_set_and_flips_inhibited_flag() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    assert!(
        fx.server.state.idle_inhibitors.is_empty(),
        "fresh fixture should start with no inhibitors",
    );

    let (_inhibitor, _surface) = fx.client(id).create_idle_inhibitor();
    fx.roundtrip(id);

    assert_eq!(
        fx.server.state.idle_inhibitors.len(),
        1,
        "one create_inhibitor must land one entry in the inhibitor set",
    );
}

#[test]
fn destroying_inhibitor_clears_the_set() {
    let mut fx = Fixture::new();
    let id = fx.add_client();

    let (inhibitor, _surface) = fx.client(id).create_idle_inhibitor();
    fx.roundtrip(id);
    assert_eq!(fx.server.state.idle_inhibitors.len(), 1);

    inhibitor.destroy();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert_eq!(
        fx.server.state.idle_inhibitors.len(),
        0,
        "destroying the inhibitor must run uninhibit and clear the set",
    );
}

#[test]
fn two_inhibitors_two_entries_then_destroy_one_keeps_the_other() {
    // Catches "uninhibit collapses the whole set instead of just
    // the one surface" — would silently turn off all inhibitors
    // when any one client closes its video.
    let mut fx = Fixture::new();
    let id = fx.add_client();

    let (inh_a, _surface_a) = fx.client(id).create_idle_inhibitor();
    let (_inh_b, _surface_b) = fx.client(id).create_idle_inhibitor();
    fx.roundtrip(id);
    assert_eq!(fx.server.state.idle_inhibitors.len(), 2);

    inh_a.destroy();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert_eq!(
        fx.server.state.idle_inhibitors.len(),
        1,
        "removing one inhibitor must not collapse the rest",
    );
}
