//! Tag/monitor move animation tests (`tag`, `toggletag`, `tagview`,
//! `tagmon`).
//!
//! Plain `tag`/`toggletag`/`setclienttags` used to make a window vanish
//! mid-frame the instant its own tags changed — `is_tag_switching`
//! deliberately suppresses the normal move-animation retarget for it
//! (right call: the destination context is unrelated to the source, so
//! animating a raw position lerp between them would be a nonsensical
//! jump). `animate_tag_departure` fills the resulting gap with a
//! fade-out `ClosingClient`, reusing the same off-list snapshot
//! pipeline real window-close already uses.
//!
//! `tagview`/`movetagview` get the opposite fix: `view_tag`'s own
//! incoming-slide staging already gives the moved window a coherent
//! start position, so `tag_view` clears `is_tag_switching` instead of
//! leaving it suppressed, letting the window ride that slide-in like
//! every other newly-visible client.

use super::fixture::Fixture;
use crate::render::open_close::OpenCloseKind;

use super::client::ClientId;

fn map_focused_window(fx: &mut Fixture) -> ClientId {
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    id
}

#[test]
fn toggletag_fades_out_a_visible_window_leaving_the_current_tag() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    map_focused_window(&mut fx);

    let home_tags = fx.server.state.clients[0].tags;
    assert!(fx.server.state.closing_clients.is_empty());

    // Flip to a mask that both keeps a bit and drops `home_tags`, so the
    // window leaves the currently-viewed tag without landing on an
    // empty tagmask (which `toggle_client_tag` rejects as a no-op).
    let other_bit = home_tags.rotate_left(1);
    fx.server.state.toggle_client_tag(home_tags | other_bit);

    assert_eq!(
        fx.server.state.clients[0].tags, other_bit,
        "sanity: the window actually left its home tag"
    );
    assert_eq!(
        fx.server.state.closing_clients.len(),
        1,
        "a visible window leaving the current tag must get a fade snapshot"
    );
    let cc = &fx.server.state.closing_clients[0];
    assert_eq!(cc.kind, OpenCloseKind::Fade);
    assert_eq!(
        cc.tags, home_tags,
        "snapshot must be tagged with the window's *old* tags so it still \
         overlaps the (unchanged) viewed tagset and actually renders"
    );
}

#[test]
fn toggletag_skips_the_fade_when_animations_are_disabled() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    map_focused_window(&mut fx);
    fx.server.state.config.animations = false;

    let home_tags = fx.server.state.clients[0].tags;
    fx.server
        .state
        .toggle_client_tag(home_tags | home_tags.rotate_left(1));

    assert!(
        fx.server.state.closing_clients.is_empty(),
        "no fade snapshot should be queued while animations are off"
    );
}

#[test]
fn tag_focused_fades_out_a_visible_window_moved_to_a_different_tag() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    map_focused_window(&mut fx);

    let home_tags = fx.server.state.clients[0].tags;
    let dest = home_tags.rotate_left(1);
    fx.server.state.tag_focused(dest);

    assert_eq!(fx.server.state.clients[0].tags, dest);
    assert_eq!(fx.server.state.closing_clients.len(), 1);
    assert_eq!(fx.server.state.closing_clients[0].tags, home_tags);
}

#[test]
fn tag_view_clears_is_tag_switching_so_the_move_can_animate() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    map_focused_window(&mut fx);

    let home_tags = fx.server.state.clients[0].tags;
    let dest = home_tags.rotate_left(1);
    fx.server.state.tag_view(dest);

    assert_eq!(
        fx.server.state.clients[0].tags, dest,
        "tag_view must move the window to the destination tag"
    );
    assert_eq!(
        fx.server.state.monitors[0].current_tagset(),
        dest,
        "tag_view must follow the window (unlike plain tag_focused)"
    );
    assert!(
        !fx.server.state.clients[0].is_tag_switching,
        "is_tag_switching must be cleared so the window can ride \
         view_tag's incoming slide instead of snapping"
    );
}

#[test]
fn tag_mon_fades_out_the_source_side_when_moving_across_outputs() {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    map_focused_window(&mut fx);

    fx.server.state.tag_mon(1);

    assert_eq!(fx.server.state.clients[0].monitor, 1);
    assert_eq!(
        fx.server.state.closing_clients.len(),
        1,
        "the source output must get a fade for the departing window"
    );
    assert_eq!(fx.server.state.closing_clients[0].monitor, 0);
    assert_eq!(fx.server.state.closing_clients[0].kind, OpenCloseKind::Fade);
}
