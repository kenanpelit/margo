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

#[test]
fn lone_scroller_column_uses_scroller_proportion_single() {
    let mut fx = Fixture::new();
    windows(&mut fx, 1);
    let curtag = fx.server.state.monitors[0].pertag.curtag;
    fx.server.state.monitors[0].pertag.ltidxs[curtag] = crate::layout::LayoutId::Scroller;
    fx.server.state.config.scroller_ignore_proportion_single = true;
    fx.server.state.arrange_monitor(0);
    let full = fx.server.state.clients[0].geom.width;

    fx.server.state.config.scroller_ignore_proportion_single = false;
    fx.server.state.clients[0].scroller_proportion_single = 0.5;
    fx.server.state.arrange_monitor(0);
    let single = fx.server.state.clients[0].geom.width;

    assert!(single < full, "single={single} full={full}");
    let wa = fx.server.state.monitors[0].work_area;
    let center = fx.server.state.clients[0].geom.x + single / 2;
    assert!(
        (center - (wa.x + wa.width / 2)).abs() <= 2,
        "lone column must stay centred"
    );
}

#[test]
fn cursor_hides_after_timeout_and_returns_on_activity() {
    let mut fx = Fixture::new();
    windows(&mut fx, 1);
    fx.server.state.config.cursor_hide_timeout = 1;
    fx.server.state.note_pointer_activity();
    // Pretend the last activity was long ago so the timer's first check
    // already sees the timeout elapsed.
    fx.server.state.pointer_activity_at =
        std::time::Instant::now() - std::time::Duration::from_secs(5);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fx.dispatch();
    assert!(fx.server.state.cursor_hidden);

    fx.server.state.note_pointer_activity();
    assert!(!fx.server.state.cursor_hidden);

    fx.server.state.config.cursor_hide_timeout = 0;
    fx.server.state.arm_cursor_hide_timer();
    assert!(fx.server.state.cursor_hide_timer.is_none());
}

#[test]
fn new_is_master_puts_the_new_window_first() {
    let mut fx = Fixture::new();
    windows(&mut fx, 1);
    let first = fx.server.state.clients[0].id;
    fx.server.state.config.new_is_master = true;
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    assert_ne!(
        fx.server.state.clients[0].id, first,
        "new window must take index 0"
    );
    assert_eq!(fx.server.state.clients[1].id, first);
}

#[test]
fn single_window_drops_radius_and_border_when_configured() {
    let mut fx = Fixture::new();
    windows(&mut fx, 1);
    fx.server.state.config.border_radius = 12;
    fx.server.state.config.no_radius_when_single = true;
    fx.server.state.config.no_border_when_single = true;
    fx.server.state.arrange_monitor(0);
    assert_eq!(fx.server.state.client_radius(0), 0.0);
    assert!(fx.server.state.hide_border_when_single(0));

    windows_more(&mut fx, 1);
    fx.server.state.arrange_monitor(0);
    assert_eq!(fx.server.state.client_radius(0), 12.0);
    assert!(!fx.server.state.hide_border_when_single(0));
}

fn windows_more(fx: &mut Fixture, n: usize) {
    for _ in 0..n {
        let id = fx.add_client();
        let (_toplevel, surface) = fx.client(id).create_toplevel();
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);
    }
}

#[test]
fn warpcursor_moves_the_pointer_to_the_newly_focused_window() {
    let mut fx = Fixture::new();
    windows(&mut fx, 2);
    fx.server.state.config.warpcursor = true;
    focus(&mut fx, 0);
    let arg = margo_config::Arg {
        i: 1,
        ..Default::default()
    };
    crate::dispatch::dispatch_action(&mut fx.server.state, "focusstack", &arg);
    let idx = fx.server.state.focused_client_idx().unwrap();
    let g = fx.server.state.clients[idx].geom;
    let (px, py) = (
        fx.server.state.input_pointer.x,
        fx.server.state.input_pointer.y,
    );
    assert!((px - (g.x as f64 + g.width as f64 / 2.0)).abs() < 1.0);
    assert!((py - (g.y as f64 + g.height as f64 / 2.0)).abs() < 1.0);
}

#[test]
fn focusdir_is_spatial_and_crosses_tags_at_the_edge() {
    let mut fx = Fixture::new();
    windows(&mut fx, 2);
    let curtag = fx.server.state.monitors[0].pertag.curtag;
    fx.server.state.monitors[0].pertag.ltidxs[curtag] = crate::layout::LayoutId::Tile;
    fx.server.state.arrange_monitor(0);
    // Leftmost window (master in tile).
    let left = (0..2)
        .min_by_key(|&i| fx.server.state.clients[i].geom.x)
        .unwrap();
    focus(&mut fx, left);
    let left_id = fx.server.state.clients[left].id;

    // Nothing left of the master: without cross flags, focus stays.
    fx.server
        .state
        .focus_direction(margo_config::Direction::Left);
    assert_eq!(focused_id(&fx), Some(left_id));

    // Right finds the other window.
    fx.server
        .state
        .focus_direction(margo_config::Direction::Right);
    assert_ne!(focused_id(&fx), Some(left_id));

    // With focus_cross_tag, the edge moves to an occupied tag.
    let home = fx.server.state.monitors[0].current_tagset();
    let other = fx.server.state.focused_client_idx().unwrap();
    fx.server.state.clients[other].tags = home << 1;
    fx.server.state.arrange_monitor(0);
    focus(&mut fx, left);
    fx.server.state.config.focus_cross_tag = true;
    fx.server
        .state
        .focus_direction(margo_config::Direction::Right);
    assert_eq!(fx.server.state.monitors[0].current_tagset(), home << 1);
}
