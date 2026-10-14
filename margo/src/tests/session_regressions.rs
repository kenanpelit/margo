//! Regression coverage for focus-restore on tag return, `smartkillclient`
//! and the `confine_pointer` clamp box.

use super::client::ClientId;
use super::fixture::Fixture;
use crate::state::FocusTarget;

fn windows(fx: &mut Fixture, n: usize) -> Vec<ClientId> {
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.server.state.config.animations = false;
    (0..n)
        .map(|_| {
            let id = fx.add_client();
            let (_toplevel, surface) = fx.client(id).create_toplevel();
            surface.commit();
            fx.client(id).flush();
            fx.roundtrip(id);
            id
        })
        .collect()
}

fn focus(fx: &mut Fixture, idx: usize) {
    let w = fx.server.state.clients[idx].window.clone();
    fx.server.state.focus_surface(Some(FocusTarget::Window(w)));
}

fn focused_id(fx: &Fixture) -> Option<u64> {
    let idx = fx.server.state.focused_client_idx()?;
    Some(fx.server.state.clients[idx].id)
}

/// Returning to a tag must focus the window that was last focused there,
/// not the oldest one (the old `focus_first_visible_or_clear`).
#[test]
fn returning_to_a_tag_focuses_its_most_recent_window() {
    let mut fx = Fixture::new();
    windows(&mut fx, 3);
    focus(&mut fx, 2);
    focus(&mut fx, 1);
    let expected = fx.server.state.clients[1].id;

    let home = fx.server.state.monitors[0].current_tagset();
    fx.server.state.view_tag(home << 1);
    fx.server.state.view_tag(home);

    assert_eq!(focused_id(&fx), Some(expected));
}

#[test]
fn smartkill_on_a_multi_tag_window_only_drops_the_viewed_tag() {
    let mut fx = Fixture::new();
    let ids = windows(&mut fx, 1);
    let home = fx.server.state.monitors[0].current_tagset();
    fx.server.state.clients[0].tags = home | (home << 1);
    focus(&mut fx, 0);

    fx.server.state.smart_kill_client();
    fx.roundtrip(ids[0]);

    assert_eq!(fx.server.state.clients[0].tags, home << 1);
    assert!(
        !fx.client(ids[0])
            .state
            .toplevels
            .values()
            .any(|t| t.close_requested),
        "a window still on another tag must not be closed"
    );
}

#[test]
fn smartkill_on_a_single_tag_window_closes_it() {
    let mut fx = Fixture::new();
    let ids = windows(&mut fx, 1);
    focus(&mut fx, 0);

    fx.server.state.smart_kill_client();
    fx.roundtrip(ids[0]);

    assert!(
        fx.client(ids[0])
            .state
            .toplevels
            .values()
            .any(|t| t.close_requested)
    );
}

#[test]
fn confine_box_insets_by_border_plus_margin_and_never_inverts() {
    let mut fx = Fixture::new();
    windows(&mut fx, 1);
    let client = &mut fx.server.state.clients[0];
    client.border_width = 2;
    client.geom = crate::layout::Rect::new(100, 200, 800, 600);
    let inset = 2.0 + crate::state::MargoClient::CONFINE_POINTER_MARGIN;
    assert_eq!(
        client.confine_box(),
        (100.0 + inset, 200.0 + inset, 900.0 - inset, 800.0 - inset)
    );

    client.geom = crate::layout::Rect::new(0, 0, 4, 4);
    let (min_x, min_y, max_x, max_y) = client.confine_box();
    assert!(max_x >= min_x && max_y >= min_y);
}
