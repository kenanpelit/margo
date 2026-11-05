//! Window minimize / restore (`state/scratchpad/minimize.rs`) against real
//! Wayland clients: the app's own `xdg_toplevel.set_minimized`, a taskbar's
//! wlr-foreign-toplevel `set_minimized` / `unset_minimized`, the
//! `minimize` / `restore_*` actions and activation.

use smithay::reexports::wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;

use super::client::ClientId;
use super::fixture::Fixture;
use crate::dispatch::{DispatchOutcome, dispatch_action};
use crate::state::FocusTarget;
use margo_config::Arg;

/// One 1080p output, a keyboard, and one mapped toplevel per app id, all
/// from one client; clients land at state indices 0, 1, ...
fn windows(apps: &[&str]) -> (Fixture, ClientId, Vec<XdgToplevel>) {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.server.state.config.animations = false;
    let id = fx.add_client();
    let mut toplevels = Vec::new();
    for app in apps {
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id((*app).into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
        toplevels.push(toplevel);
    }
    (fx, id, toplevels)
}

fn idx(fx: &Fixture, app: &str) -> usize {
    fx.server
        .state
        .clients
        .iter()
        .position(|c| c.app_id == app)
        .unwrap_or_else(|| panic!("no client {app}"))
}

fn focus(fx: &mut Fixture, app: &str) {
    let w = fx.server.state.clients[idx(fx, app)].window.clone();
    fx.server.state.focus_surface(Some(FocusTarget::Window(w)));
}

fn mapped(fx: &Fixture, app: &str) -> bool {
    let w = &fx.server.state.clients[idx(fx, app)].window;
    fx.server.state.space.elements().any(|e| e == w)
}

fn minimized(fx: &Fixture, app: &str) -> bool {
    fx.server.state.clients[idx(fx, app)].is_user_minimized()
}

fn focused_app(fx: &Fixture) -> Option<String> {
    fx.server
        .state
        .focused_client_idx()
        .map(|i| fx.server.state.clients[i].app_id.clone())
}

fn dispatch(fx: &mut Fixture, action: &str, i: i32) {
    let arg = Arg {
        i,
        ..Default::default()
    };
    assert_eq!(
        dispatch_action(&mut fx.server.state, action, &arg),
        DispatchOutcome::Handled
    );
}

#[test]
fn app_minimize_button_hides_the_window_and_hands_focus_on() {
    let (mut fx, id, toplevels) = windows(&["a", "b"]);
    focus(&mut fx, "b");
    // The CSD minimize button: `xdg_toplevel.set_minimized`.
    toplevels[1].set_minimized();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert!(minimized(&fx, "b"));
    assert!(!mapped(&fx, "b"), "a minimized window is unmapped");
    assert_eq!(focused_app(&fx).as_deref(), Some("a"), "focus moves on");
    // The remaining tile takes the whole work area.
    let wa = fx.server.state.monitors[0].work_area;
    let a = fx.server.state.clients[idx(&fx, "a")].geom;
    assert!(a.width > wa.width / 2, "a re-tiled alone: {a:?} in {wa:?}");
    // Still on its tag (the restore target with in-place restore).
    assert_ne!(fx.server.state.clients[idx(&fx, "b")].tags, 0);
}

#[test]
fn minimizing_a_background_window_keeps_focus() {
    let (mut fx, _id, _t) = windows(&["a", "b"]);
    focus(&mut fx, "a");
    let id_b = fx.server.state.clients[idx(&fx, "b")].id;
    dispatch(&mut fx, "minimize", id_b as i32);
    assert!(minimized(&fx, "b"));
    assert_eq!(focused_app(&fx).as_deref(), Some("a"));
}

#[test]
fn restore_last_then_restore_all_follow_minimize_order() {
    let (mut fx, _id, _t) = windows(&["a", "b", "c", "d"]);
    for app in ["c", "a", "b"] {
        focus(&mut fx, app);
        dispatch(&mut fx, "minimize", 0);
    }
    assert!(minimized(&fx, "a") && minimized(&fx, "b") && minimized(&fx, "c"));
    assert_eq!(focused_app(&fx).as_deref(), Some("d"));

    dispatch(&mut fx, "restore_last", 0);
    assert!(!minimized(&fx, "b") && mapped(&fx, "b"));
    assert_eq!(focused_app(&fx).as_deref(), Some("b"));
    assert!(minimized(&fx, "a") && minimized(&fx, "c"));

    dispatch(&mut fx, "restore_all", 0);
    assert!(!minimized(&fx, "a") && !minimized(&fx, "c"));
    assert!(mapped(&fx, "a") && mapped(&fx, "c"));
    assert_eq!(
        focused_app(&fx).as_deref(),
        Some("a"),
        "restore_all goes oldest first, so the newest (a) ends focused"
    );
    // Nothing left: both are no-ops.
    dispatch(&mut fx, "restore_last", 0);
    dispatch(&mut fx, "restore_all", 0);
}

#[test]
fn restore_comes_to_the_current_tag_by_default() {
    let (mut fx, _id, _t) = windows(&["a"]);
    focus(&mut fx, "a");
    dispatch(&mut fx, "minimize", 0);
    fx.server.state.view_tag(1 << 4);
    dispatch(&mut fx, "restore_last", 0);
    let c = &fx.server.state.clients[idx(&fx, "a")];
    assert_eq!(c.tags, 1 << 4, "joined the tag being viewed");
    assert_eq!(fx.server.state.monitors[0].current_tagset(), 1 << 4);
    assert!(mapped(&fx, "a"));
}

#[test]
fn restore_in_place_goes_back_to_the_windows_tag() {
    let (mut fx, _id, _t) = windows(&["a"]);
    fx.server.state.config.minimize_restore_in_place = true;
    focus(&mut fx, "a");
    let tags = fx.server.state.clients[0].tags;
    dispatch(&mut fx, "minimize", 0);
    fx.server.state.view_tag(1 << 4);
    let id = fx.server.state.clients[0].id;
    dispatch(&mut fx, "restore_window", id as i32);
    assert_eq!(fx.server.state.clients[0].tags, tags, "kept its tag");
    assert_eq!(
        fx.server.state.monitors[0].current_tagset(),
        tags,
        "the view followed it"
    );
    assert!(mapped(&fx, "a"));
    assert_eq!(focused_app(&fx).as_deref(), Some("a"));
}

#[test]
fn activation_restores_a_minimized_window() {
    let (mut fx, _id, _t) = windows(&["a", "b"]);
    focus(&mut fx, "b");
    dispatch(&mut fx, "minimize", 0);
    // A dock click / `focuswindowid` / xdg-activation.
    let id_b = fx.server.state.clients[idx(&fx, "b")].id;
    fx.server.state.activate_window_id(id_b);
    assert!(!minimized(&fx, "b") && mapped(&fx, "b"));
    assert_eq!(focused_app(&fx).as_deref(), Some("b"));
}

#[test]
fn a_floating_window_comes_back_floating() {
    let (mut fx, _id, _t) = windows(&["a", "f"]);
    let i = idx(&fx, "f");
    fx.server.state.clients[i].is_floating = true;
    focus(&mut fx, "f");
    dispatch(&mut fx, "minimize", 0);
    dispatch(&mut fx, "restore_last", 0);
    let c = &fx.server.state.clients[i];
    assert!(c.is_floating && !c.is_minimized);
}

#[test]
fn minimize_on_a_shown_scratchpad_tucks_it_away() {
    let (mut fx, _id, _t) = windows(&["a", "pad"]);
    fx.server
        .state
        .toggle_named_scratchpad(Some("^pad$"), None, None);
    let i = idx(&fx, "pad");
    assert!(fx.server.state.clients[i].is_scratchpad_show);
    dispatch(&mut fx, "minimize", 0);
    let c = &fx.server.state.clients[i];
    assert!(c.is_in_scratchpad && !c.is_scratchpad_show && c.is_minimized);
    assert!(
        !c.is_user_minimized(),
        "still a scratchpad, not a minimized window"
    );
    // restore_* leave scratchpads to their own toggle.
    dispatch(&mut fx, "restore_all", 0);
    assert!(!fx.server.state.clients[i].is_scratchpad_show);
}

#[test]
fn minimizing_a_group_member_takes_it_out_of_the_group() {
    let (mut fx, _id, _t) = windows(&["a", "b"]);
    focus(&mut fx, "a");
    fx.server.state.toggle_group();
    assert!(fx.server.state.clients.iter().all(|c| c.group_id.is_some()));
    let active = fx
        .server
        .state
        .clients
        .iter()
        .position(|c| c.group_active)
        .expect("an active member");
    let other = 1 - active;
    let active_app = fx.server.state.clients[active].app_id.clone();
    focus(&mut fx, &active_app);
    dispatch(&mut fx, "minimize", 0);
    let state = &fx.server.state;
    assert!(
        state.clients.iter().all(|c| c.group_id.is_none()),
        "dissolved"
    );
    assert!(state.clients[other].is_visible_on(0, state.monitors[0].current_tagset()));
    let app = state.clients[other].app_id.clone();
    assert!(mapped(&fx, &app), "the survivor fills the slot");
}

#[test]
fn taskbar_minimize_and_restore_round_trip_with_state() {
    let (mut fx, _id, _t) = windows(&["a", "b"]);
    let bar = fx.add_client();
    fx.client(bar).bind_foreign_toplevel_manager();
    fx.roundtrip(bar);
    crate::protocols::wlr_foreign_toplevel::refresh(&mut fx.server.state);
    fx.roundtrip(bar);
    assert!(
        !fx.client(bar)
            .state
            .foreign_toplevels
            .by_app_id("b")
            .minimized()
    );

    let handle = fx
        .client(bar)
        .state
        .foreign_toplevels
        .by_app_id("b")
        .handle
        .clone();
    handle.set_minimized();
    fx.client(bar).flush();
    fx.roundtrip(bar);
    assert!(minimized(&fx, "b"), "the taskbar's minimize is honoured");
    crate::protocols::wlr_foreign_toplevel::refresh(&mut fx.server.state);
    fx.roundtrip(bar);
    assert!(
        fx.client(bar)
            .state
            .foreign_toplevels
            .by_app_id("b")
            .minimized(),
        "the minimized state is sent back"
    );

    handle.unset_minimized();
    fx.client(bar).flush();
    fx.roundtrip(bar);
    assert!(!minimized(&fx, "b") && mapped(&fx, "b"));
    crate::protocols::wlr_foreign_toplevel::refresh(&mut fx.server.state);
    fx.roundtrip(bar);
    assert!(
        !fx.client(bar)
            .state
            .foreign_toplevels
            .by_app_id("b")
            .minimized()
    );
}

#[test]
fn snapshot_reports_minimized_order() {
    let (mut fx, _id, _t) = windows(&["a", "b"]);
    focus(&mut fx, "b");
    dispatch(&mut fx, "minimize", 0);
    focus(&mut fx, "a");
    dispatch(&mut fx, "minimize", 0);
    let snap = fx.server.state.build_state_snapshot();
    let order = |app: &str| {
        snap["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["app_id"] == app)
            .unwrap()["minimized_order"]
            .as_u64()
            .unwrap()
    };
    assert!(order("a") > order("b") && order("b") > 0);
    dispatch(&mut fx, "restore_all", 0);
    let snap = fx.server.state.build_state_snapshot();
    assert!(
        snap["clients"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["minimized_order"] == 0 && c["minimized"] == false)
    );
}
