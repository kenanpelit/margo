//! Tabbed window group tests (`togglegroup` family).
//!
//! Groups are purely additive: nothing groups a window until a verb or
//! a `group:1` windowrule fires. These tests drive **real**
//! xdg_toplevels through the fixture so the windows live in
//! `state.clients`, are mapped, and hold keyboard focus — then exercise
//! the group state machine and assert the invariants the layout/render
//! paths rely on:
//!
//!   * grouping collapses N windows to ONE tiled slot (cardinality),
//!   * a group always has exactly one active member,
//!   * cycling wraps and re-homes focus,
//!   * ungrouping / closing a member restores the other tiles,
//!   * a group of one dissolves back to a plain window.

use super::client::ClientId;
use super::fixture::Fixture;

/// Map a single focused toplevel; drive the deferred-map flow to
/// completion so `finalize_initial_map` runs (maps + focuses).
fn map_window(fx: &mut Fixture) -> ClientId {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    id
}

/// Focus the client at `idx` by handing keyboard focus to its window,
/// the same path `focus_stack` uses. Lets a test pick which window
/// `togglegroup` (which acts on the *focused* client) operates on.
fn focus_client(fx: &mut Fixture, idx: usize) {
    let window = fx.server.state.clients[idx].window.clone();
    fx.server
        .state
        .focus_surface(Some(crate::state::FocusTarget::Window(window)));
}

/// Count clients laid out as tiles on monitor 0's current tagset —
/// i.e. how many slots the layout pass will allocate. Hidden group
/// members are excluded by `is_visible_on`, so this is the cardinality
/// the arrange path sees.
fn tiled_count(fx: &Fixture) -> usize {
    let tagset = fx.server.state.monitors[0].current_tagset();
    fx.server
        .state
        .clients
        .iter()
        .filter(|c| c.is_visible_on(0, tagset) && c.is_tiled())
        .count()
}

fn group_of(fx: &Fixture, idx: usize) -> Option<u32> {
    fx.server.state.clients[idx].group_id
}

fn active_members(fx: &Fixture, gid: u32) -> usize {
    fx.server
        .state
        .clients
        .iter()
        .filter(|c| c.group_id == Some(gid) && c.group_active)
        .count()
}

fn members(fx: &Fixture, gid: u32) -> usize {
    fx.server
        .state
        .clients
        .iter()
        .filter(|c| c.group_id == Some(gid))
        .count()
}

/// Map three windows on a single output and return their `ClientId`s.
fn three_windows(fx: &mut Fixture) -> [ClientId; 3] {
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    let a = map_window(fx);
    let b = map_window(fx);
    let c = map_window(fx);
    [a, b, c]
}

#[test]
fn no_groups_by_default() {
    // Additivity: mapping windows never groups them.
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    assert!(
        fx.server.state.clients.iter().all(|c| c.group_id.is_none()),
        "fresh windows must be ungrouped",
    );
    assert_eq!(tiled_count(&fx), 3, "three ungrouped windows = three slots");
}

#[test]
fn togglegroup_collapses_to_one_slot() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);
    assert_eq!(tiled_count(&fx), 3);

    // Focus window 0 and group it with its neighbour (window 1).
    focus_client(&mut fx, 0);
    fx.server.state.toggle_group();

    let gid = group_of(&fx, 0).expect("window 0 should now be grouped");
    assert_eq!(
        group_of(&fx, 1),
        Some(gid),
        "neighbour joins the same group"
    );
    assert_eq!(members(&fx, gid), 2, "group holds exactly two members");
    assert_eq!(
        active_members(&fx, gid),
        1,
        "exactly one active member invariant",
    );
    // Two grouped (one slot) + one ungrouped = two slots.
    assert_eq!(
        tiled_count(&fx),
        2,
        "a group of two collapses to a single tiled slot",
    );
}

#[test]
fn changegroupactive_cycles_and_wraps() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    // Group all three: group 0+1, then add 2 into the same group.
    focus_client(&mut fx, 0);
    fx.server.state.toggle_group(); // 0 joins 1
    let gid = group_of(&fx, 0).unwrap();
    // Make 1's group the neighbour of 2 by focusing 2 and merging.
    focus_client(&mut fx, 2);
    fx.server.state.toggle_group(); // 2 joins the visible neighbour's group
    assert_eq!(members(&fx, gid), 3, "all three windows in one group");
    assert_eq!(tiled_count(&fx), 1, "three-member group = one slot");

    // The active member is the one the user just grouped (2).
    let active0 = (0..3).find(|&i| fx.server.state.clients[i].group_active);
    assert!(active0.is_some(), "one active member");

    // Cycle forward three times → wraps back to where we started.
    let start = active0.unwrap();
    fx.server.state.change_group_active(1);
    fx.server.state.change_group_active(1);
    fx.server.state.change_group_active(1);
    let after = (0..3)
        .find(|&i| fx.server.state.clients[i].group_active)
        .unwrap();
    assert_eq!(after, start, "cycling N times wraps to the start");
    assert_eq!(active_members(&fx, gid), 1, "still exactly one active");

    // Backward cycle moves to a different member.
    fx.server.state.change_group_active(-1);
    let back = (0..3)
        .find(|&i| fx.server.state.clients[i].group_active)
        .unwrap();
    assert_ne!(back, start, "prev moves off the current member");
}

#[test]
fn ungroup_restores_tiles() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    focus_client(&mut fx, 0);
    fx.server.state.toggle_group(); // 0 + 1 grouped
    assert_eq!(tiled_count(&fx), 2);

    // Ungroup window 0 → its group had two members, so removing one
    // leaves a group of one which dissolves: both windows ungrouped.
    focus_client(&mut fx, 0);
    fx.server.state.toggle_group();
    assert!(
        fx.server.state.clients.iter().all(|c| c.group_id.is_none()),
        "dissolving the last pair leaves no groups",
    );
    assert_eq!(tiled_count(&fx), 3, "all three windows tile again");
}

#[test]
fn closing_active_member_dissolves_group_of_one() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));

    // Keep the toplevel handle for the window we'll close.
    let id = fx.add_client();
    let (toplevel0, surface0) = fx.client(id).create_toplevel();
    surface0.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    let id1 = map_window(&mut fx);

    // Group 0 + 1, with 0 active.
    focus_client(&mut fx, 0);
    fx.server.state.toggle_group();
    let gid = group_of(&fx, 0).unwrap();
    fx.server.state.activate_group_member(0);
    assert!(fx.server.state.clients[0].group_active);
    assert_eq!(members(&fx, gid), 2);

    // Close the active member via the real destroy path.
    toplevel0.destroy();
    fx.client(id).flush();
    fx.roundtrip(id1);

    // The group now has one member left → it dissolves to a plain
    // window (no "group of one"), and the survivor tiles normally.
    assert!(
        fx.server
            .state
            .clients
            .iter()
            .all(|c| c.group_id != Some(gid)),
        "a one-member group dissolves after the active member closes",
    );
    assert_eq!(fx.server.state.clients.len(), 1);
    assert_eq!(tiled_count(&fx), 1);
}

#[test]
fn lockgroups_blocks_grouping() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    fx.server
        .state
        .set_groups_locked(crate::state::GroupLock::On);
    focus_client(&mut fx, 0);
    fx.server.state.toggle_group();
    assert!(
        fx.server.state.clients.iter().all(|c| c.group_id.is_none()),
        "togglegroup is a no-op while groups are locked",
    );

    // Unlock and confirm grouping works again.
    fx.server
        .state
        .set_groups_locked(crate::state::GroupLock::Off);
    focus_client(&mut fx, 0);
    fx.server.state.toggle_group();
    assert!(group_of(&fx, 0).is_some(), "grouping resumes once unlocked",);
}

#[test]
fn single_window_togglegroup_is_noop() {
    // A lone window has no neighbour to merge with.
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    let _ = map_window(&mut fx);

    focus_client(&mut fx, 0);
    fx.server.state.toggle_group();
    assert!(
        fx.server.state.clients[0].group_id.is_none(),
        "togglegroup with no neighbour leaves the window ungrouped",
    );
}

#[test]
fn hidden_members_are_not_visible() {
    let mut fx = Fixture::new();
    let _ = three_windows(&mut fx);

    focus_client(&mut fx, 0);
    fx.server.state.toggle_group(); // 0 + 1, one active
    let gid = group_of(&fx, 0).unwrap();
    let tagset = fx.server.state.monitors[0].current_tagset();

    // Exactly one of the two members reports visible; the other is the
    // collapsed (hidden) tab.
    let visible = (0..3)
        .filter(|&i| {
            fx.server.state.clients[i].group_id == Some(gid)
                && fx.server.state.clients[i].is_visible_on(0, tagset)
        })
        .count();
    assert_eq!(visible, 1, "only the active member of a group is visible");
}
