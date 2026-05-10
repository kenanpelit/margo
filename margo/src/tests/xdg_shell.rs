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
    assert_eq!(remaining.len(), 1, "one toplevel destroyed, one should remain");
    assert_eq!(
        remaining[0].app_id, "beta",
        "destroying alpha must leave beta — index/shift bug regression",
    );
}
