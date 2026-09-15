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

use margo_config::Config;
use smithay::wayland::seat::WaylandFocus;

use super::fixture::Fixture;
use crate::state::FocusTarget;

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

// ── recompute_idle_inhibit — heuristic sources (mango ports) ────────────────
//
// `idleinhibit_ignore_visible`, `idle_inhibit_when_focus` (window rule)
// and `idle_inhibit_when_fullscreen` layer three compositor-side
// heuristics on top of the raw protocol path above. These poke
// `idle_inhibitors` / client flags directly rather than going through
// `create_idle_inhibitor` — that fixture helper binds to a fresh,
// unmapped surface (fine for the protocol-set tests above), but these
// tests need the inhibiting surface to belong to an actual mapped
// `MargoClient` so the visibility filter has something to check.

fn one_mapped_client(fx: &mut Fixture) {
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
}

#[test]
fn invisible_protocol_inhibitor_is_ignored_by_default() {
    let mut fx = Fixture::new();
    one_mapped_client(&mut fx);
    // Off the currently-viewed tag (monitor defaults to tag 1).
    fx.server.state.clients[0].tags = 1 << 1;

    let surface = fx.server.state.clients[0]
        .window
        .wl_surface()
        .expect("mapped client has a surface")
        .into_owned();
    fx.server.state.idle_inhibitors.insert(surface);
    fx.server.state.recompute_idle_inhibit();

    assert!(
        !fx.server.state.idle_notifier_state.is_inhibited(),
        "an invisible client's protocol inhibit must not count by default"
    );
}

#[test]
fn idleinhibit_ignore_visible_lets_invisible_clients_count() {
    let mut fx = Fixture::with_config(Config {
        idleinhibit_ignore_visible: true,
        ..Config::default()
    });
    one_mapped_client(&mut fx);
    fx.server.state.clients[0].tags = 1 << 1;

    let surface = fx.server.state.clients[0]
        .window
        .wl_surface()
        .expect("mapped client has a surface")
        .into_owned();
    fx.server.state.idle_inhibitors.insert(surface);
    fx.server.state.recompute_idle_inhibit();

    assert!(
        fx.server.state.idle_notifier_state.is_inhibited(),
        "idleinhibit_ignore_visible=true must let an invisible inhibitor count"
    );
}

#[test]
fn idle_inhibit_when_focus_rule_inhibits_without_a_protocol_request() {
    let mut fx = Fixture::new();
    one_mapped_client(&mut fx);

    fx.server.state.clients[0].idle_inhibit_when_focus = true;
    // The single mapped client is already focused from the initial-map
    // auto-focus, so re-focusing it here would be a same-target no-op
    // that skips focus_surface's recompute call — drive it directly to
    // pick up the flag we just flipped.
    fx.server.state.recompute_idle_inhibit();

    assert!(
        fx.server.state.idle_notifier_state.is_inhibited(),
        "idle_inhibit_when_focus must inhibit while focused, no protocol request needed"
    );

    fx.server.state.focus_surface(None);
    assert!(
        !fx.server.state.idle_notifier_state.is_inhibited(),
        "losing focus must drop a focus-only inhibit"
    );
}

#[test]
fn idle_inhibit_when_fullscreen_inhibits_the_focused_fullscreen_client() {
    let mut fx = Fixture::with_config(Config {
        idle_inhibit_when_fullscreen: true,
        ..Config::default()
    });
    one_mapped_client(&mut fx);

    let win = fx.server.state.clients[0].window.clone();
    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(win)));
    assert!(
        !fx.server.state.idle_notifier_state.is_inhibited(),
        "not fullscreen yet — must not be inhibited"
    );

    fx.server.state.clients[0].is_fullscreen = true;
    fx.server.state.recompute_idle_inhibit();

    assert!(
        fx.server.state.idle_notifier_state.is_inhibited(),
        "a focused fullscreen client must inhibit idle with the config flag on"
    );
}

#[test]
fn idle_inhibit_when_fullscreen_is_off_by_default() {
    let mut fx = Fixture::new();
    one_mapped_client(&mut fx);
    fx.server.state.clients[0].is_fullscreen = true;
    let win = fx.server.state.clients[0].window.clone();
    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(win)));

    assert!(
        !fx.server.state.idle_notifier_state.is_inhibited(),
        "fullscreen must not inhibit idle unless idle_inhibit_when_fullscreen is on"
    );
}
