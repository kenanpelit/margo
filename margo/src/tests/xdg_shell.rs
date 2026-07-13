//! Integration tests for `XdgShellHandler` (W4.2 Phase 6 extracted
//! impl at `state/handlers/xdg_shell.rs`).
//!
//! These tests drive the real handler through the Wayland protocol
//! via the [`super::fixture::Fixture`] harness — same code path
//! Firefox / Chromium / Helium / kitty hit at runtime.
//!
//! Margo's xdg_shell flow is **commit-staged**:
//!
//! 1. `xdg_surface.get_toplevel` → `new_toplevel` fires; pushes a
//!    fresh `MargoClient` with `is_initial_map_pending = true`,
//!    BEFORE rule application.
//! 2. First `wl_surface.commit` → compositor commit handler runs
//!    `finalize_initial_map`: reads `app_id`/`title`, applies
//!    window-rules, flips `is_initial_map_pending = false`.
//! 3. Subsequent `set_app_id` / `set_title` updates take effect
//!    on the next commit (margo's `refresh_wayland_toplevel_identity`).
//!
//! That sequence is the load-bearing fix for the "CopyQ flickers
//! between default and rule-driven geometry" symptom — every test
//! below probes one stage of it.
//!
//! Tests intentionally split commits so we can assert each stage
//! independently. `Client::create_toplevel` does NOT commit; tests
//! call `wl_surface.commit(); client.flush();` when they want
//! finalize_initial_map to run.

use super::fixture::Fixture;
use crate::layout::LayoutId;
use crate::state::FocusTarget;

#[test]
fn pre_commit_toplevel_is_pending_initial_map() {
    // Stage 1: role is created but no commit yet. Window-rule
    // engine hasn't run; the deferred-map flag is the
    // truth-source for "this client is in limbo, layout should
    // skip it".
    let mut fx = Fixture::new();
    let id = fx.add_client();

    let (_toplevel, _surface) = fx.client(id).create_toplevel();
    fx.roundtrip(id);

    let clients = &fx.server.state.clients;
    assert_eq!(
        clients.len(),
        1,
        "new_toplevel should push exactly one MargoClient"
    );
    assert!(
        clients[0].is_initial_map_pending,
        "deferred-map invariant: pre-commit toplevel must be flagged pending",
    );
}

#[test]
fn first_commit_finalizes_initial_map() {
    // Stage 2: after the client commits, finalize_initial_map
    // runs and clears the pending flag. This is what unblocks
    // the layout / arrange path for the new client.
    let mut fx = Fixture::new();
    let id = fx.add_client();

    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    let clients = &fx.server.state.clients;
    assert_eq!(clients.len(), 1);
    assert!(
        !clients[0].is_initial_map_pending,
        "first wl_surface.commit must trigger finalize_initial_map and clear the pending flag",
    );
}

#[test]
fn set_app_id_and_title_propagate_after_commit() {
    // Stage 3: identity (app_id / title) is read from
    // XdgToplevelSurfaceData on commit, not on the request itself.
    // Window-rule lookup keys on these strings, so a regression
    // where the refresh path skips a commit silently breaks every
    // rule-keyed-on-app_id config out there.
    let mut fx = Fixture::new();
    let id = fx.add_client();

    let (toplevel, surface) = fx.client(id).create_toplevel();
    toplevel.set_app_id("test.app.id".into());
    toplevel.set_title("Window Title 42".into());
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    let client = fx
        .server
        .state
        .clients
        .first()
        .expect("MargoClient should exist after new_toplevel");
    assert_eq!(client.app_id, "test.app.id");
    assert_eq!(client.title, "Window Title 42");
}

#[test]
fn toplevel_destroy_removes_client() {
    let mut fx = Fixture::new();
    let id = fx.add_client();

    let (toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert_eq!(fx.server.state.clients.len(), 1);

    toplevel.destroy();
    fx.client(id).flush();
    fx.roundtrip(id);

    // `toplevel_destroyed` removes the MargoClient from
    // `state.clients` immediately; the close animation lives in
    // `closing_clients` (not asserted here — render-path coverage
    // is bigger than W1.6's headless scope).
    assert_eq!(
        fx.server.state.clients.len(),
        0,
        "toplevel destroy must drop the MargoClient slot",
    );
}

#[test]
fn two_toplevels_coexist_in_clients_vec() {
    let mut fx = Fixture::new();
    let id = fx.add_client();

    let (_a_t, a_s) = fx.client(id).create_toplevel();
    let (_b_t, b_s) = fx.client(id).create_toplevel();
    a_s.commit();
    b_s.commit();
    fx.client(id).flush();
    fx.roundtrip(id);

    assert_eq!(
        fx.server.state.clients.len(),
        2,
        "scroller smart-insert path should land both toplevels in clients",
    );
}

#[test]
fn destroying_one_of_two_toplevels_keeps_the_other() {
    // Catches "destroy index goes wrong, remove() shifts the
    // wrong client out" class regressions.
    let mut fx = Fixture::new();
    let id = fx.add_client();

    let (a_t, a_s) = fx.client(id).create_toplevel();
    a_t.set_app_id("alpha".into());
    let (b_t, b_s) = fx.client(id).create_toplevel();
    b_t.set_app_id("beta".into());
    a_s.commit();
    b_s.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert_eq!(fx.server.state.clients.len(), 2);

    a_t.destroy();
    fx.client(id).flush();
    fx.roundtrip(id);

    let remaining = &fx.server.state.clients;
    assert_eq!(
        remaining.len(),
        1,
        "one toplevel destroyed, one should remain"
    );
    assert_eq!(
        remaining[0].app_id, "beta",
        "destroying alpha must leave beta — index/shift bug regression",
    );
}

#[test]
fn focus_history_distinguishes_same_app_windows_by_stable_id() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_keyboard();
    let peer = fx.add_client();
    let (_a_t, a_s) = fx.client(peer).create_toplevel();
    let (b_t, b_s) = fx.client(peer).create_toplevel();
    b_t.set_app_id("same.app".into());
    a_s.commit();
    b_s.commit();
    fx.client(peer).flush();
    fx.roundtrip(peer);

    for client in &mut fx.server.state.clients {
        client.app_id = "same.app".into();
    }
    let ids: std::collections::HashSet<_> = fx
        .server
        .state
        .clients
        .iter()
        .map(|client| client.id)
        .collect();
    let history: std::collections::HashSet<_> = fx.server.state.monitors[0]
        .focus_history
        .iter()
        .copied()
        .collect();
    assert_eq!(history, ids);

    let snapshot = fx.server.state.build_state_snapshot();
    let v2 = snapshot["outputs"][0]["focus_history_v2"]
        .as_array()
        .expect("id-bearing focus history");
    assert_eq!(v2.len(), 2);
    assert_ne!(v2[0]["id"], v2[1]["id"]);
}

#[test]
fn focus_history_survives_scroller_slot_insertion() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_keyboard();
    let curtag = fx.server.state.monitors[0].pertag.curtag;
    fx.server.state.monitors[0].pertag.ltidxs[curtag] = LayoutId::Scroller;
    let peer = fx.add_client();

    let (a_t, a_s) = fx.client(peer).create_toplevel();
    a_t.set_app_id("alpha".into());
    a_s.commit();
    fx.client(peer).flush();
    fx.roundtrip(peer);
    let (c_t, c_s) = fx.client(peer).create_toplevel();
    c_t.set_app_id("charlie".into());
    c_s.commit();
    fx.client(peer).flush();
    fx.roundtrip(peer);

    let a_idx = fx
        .server
        .state
        .clients
        .iter()
        .position(|client| client.app_id == "alpha")
        .expect("alpha");
    let a_id = fx.server.state.clients[a_idx].id;
    let c_id = fx
        .server
        .state
        .clients
        .iter()
        .find(|client| client.app_id == "charlie")
        .expect("charlie")
        .id;
    let a_window = fx.server.state.clients[a_idx].window.clone();
    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(a_window)));

    let (b_t, b_s) = fx.client(peer).create_toplevel();
    b_t.set_app_id("beta".into());
    b_s.commit();
    fx.client(peer).flush();
    fx.roundtrip(peer);
    let b_id = fx
        .server
        .state
        .clients
        .iter()
        .find(|client| client.app_id == "beta")
        .expect("beta")
        .id;

    assert_eq!(
        fx.server
            .state
            .clients
            .iter()
            .map(|client| client.id)
            .collect::<Vec<_>>(),
        vec![a_id, b_id, c_id],
    );
    assert_eq!(
        fx.server.state.monitors[0]
            .focus_history
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![b_id, a_id, c_id],
    );
}

#[test]
fn destroying_client_prunes_focus_history_id() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_keyboard();
    let peer = fx.add_client();
    let (a_t, a_s) = fx.client(peer).create_toplevel();
    let (_b_t, b_s) = fx.client(peer).create_toplevel();
    a_s.commit();
    b_s.commit();
    fx.client(peer).flush();
    fx.roundtrip(peer);
    let removed_id = fx.server.state.clients[0].id;

    a_t.destroy();
    fx.client(peer).flush();
    fx.roundtrip(peer);

    assert!(
        fx.server.state.monitors[0]
            .focus_history
            .iter()
            .all(|id| *id != removed_id),
    );
}

#[test]
fn focusing_after_monitor_move_rehomes_history_id() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    fx.add_keyboard();
    let peer = fx.add_client();
    let (_toplevel, surface) = fx.client(peer).create_toplevel();
    surface.commit();
    fx.client(peer).flush();
    fx.roundtrip(peer);
    let client_id = fx.server.state.clients[0].id;
    let window = fx.server.state.clients[0].window.clone();

    fx.server.state.clients[0].monitor = 1;
    fx.server
        .state
        .focus_surface(Some(FocusTarget::Window(window)));

    assert!(
        !fx.server.state.monitors[0]
            .focus_history
            .contains(&client_id)
    );
    assert_eq!(
        fx.server.state.monitors[1].focus_history.front(),
        Some(&client_id),
    );
}
