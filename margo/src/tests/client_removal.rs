//! Closing a window: what happens to focus, layout and the close
//! animation. Both the Wayland (`toplevel_destroyed`) and the X11
//! (`remove_x11_window`) paths go through `MargoState::remove_client`;
//! the fixture can only create Wayland toplevels, so the X11 side is
//! covered by calling `remove_client` directly, which is the whole of
//! what `remove_x11_window` does for a managed window.

use smithay::reexports::wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_client::protocol::wl_surface::WlSurface;

use super::client::ClientId;
use super::fixture::Fixture;
use crate::state::FocusTarget;

struct Win {
    peer: ClientId,
    toplevel: XdgToplevel,
    _surface: WlSurface,
}

fn open(fx: &mut Fixture) -> Win {
    let peer = fx.add_client();
    let (toplevel, surface) = fx.client(peer).create_toplevel();
    surface.commit();
    fx.client(peer).flush();
    fx.roundtrip(peer);
    Win {
        peer,
        toplevel,
        _surface: surface,
    }
}

fn close(fx: &mut Fixture, win: &Win) {
    win.toplevel.destroy();
    fx.client(win.peer).flush();
    fx.roundtrip(win.peer);
}

fn focus(fx: &mut Fixture, id: u64) {
    let w = fx.server.state.clients[idx_of(fx, id)].window.clone();
    fx.server.state.focus_surface(Some(FocusTarget::Window(w)));
}

fn focused_id(fx: &Fixture) -> Option<u64> {
    let idx = fx.server.state.focused_client_idx()?;
    Some(fx.server.state.clients[idx].id)
}

fn idx_of(fx: &Fixture, id: u64) -> usize {
    fx.server
        .state
        .clients
        .iter()
        .position(|c| c.id == id)
        .expect("client still present")
}

fn desk(outputs: usize) -> Fixture {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    for i in 0..outputs {
        fx.add_output(&format!("DP-{}", i + 1), (1920, 1080));
    }
    fx.server.state.config.animations = false;
    fx
}

/// Closing a window that doesn't have focus must leave focus where it
/// is. It used to refocus `prev_selected`, so with A focused after C,
/// closing the background window B jumped focus to C.
#[test]
fn closing_a_background_window_keeps_focus() {
    let mut fx = desk(1);
    let a = open(&mut fx);
    let b = open(&mut fx);
    let c = open(&mut fx);
    let ids: Vec<u64> = fx.server.state.clients.iter().map(|c| c.id).collect();
    let (a_id, c_id) = (ids[0], ids[2]);
    focus(&mut fx, c_id);
    focus(&mut fx, a_id);
    assert_eq!(focused_id(&fx), Some(a_id));

    close(&mut fx, &b);

    assert_eq!(fx.server.state.clients.len(), 2);
    assert_eq!(
        focused_id(&fx),
        Some(a_id),
        "closing an unfocused window must not move keyboard focus"
    );
    let _ = (a, c);
}

/// Closing the focused window still hands focus to the previous one.
#[test]
fn closing_the_focused_window_recalls_previous_focus() {
    let mut fx = desk(1);
    let _a = open(&mut fx);
    let b = open(&mut fx);
    let _c = open(&mut fx);
    let ids: Vec<u64> = fx.server.state.clients.iter().map(|c| c.id).collect();
    focus(&mut fx, ids[2]);
    focus(&mut fx, ids[1]);

    close(&mut fx, &b);

    assert_eq!(focused_id(&fx), Some(ids[2]));
}

/// Put client `id` on monitor `mon` and re-lay everything out.
fn move_to(fx: &mut Fixture, id: u64, mon: usize) {
    let idx = idx_of(fx, id);
    let tags = fx.server.state.monitors[mon].current_tagset();
    fx.server.state.clients[idx].monitor = mon;
    fx.server.state.clients[idx].tags = tags;
    fx.server.state.arrange_all();
}

/// Closing a window on a monitor other than the focused one must
/// re-arrange *that* monitor, not the focused one, and must not pull
/// keyboard focus over to it.
#[test]
fn closing_a_window_on_another_monitor_rearranges_its_monitor() {
    let mut fx = desk(2);
    let _a = open(&mut fx);
    let b = open(&mut fx);
    let _c = open(&mut fx);
    let ids: Vec<u64> = fx.server.state.clients.iter().map(|c| c.id).collect();
    move_to(&mut fx, ids[1], 1);
    move_to(&mut fx, ids[2], 1);
    focus(&mut fx, ids[0]);
    let half = fx.server.state.clients[idx_of(&fx, ids[2])].geom;

    close(&mut fx, &b);

    let after = fx.server.state.clients[idx_of(&fx, ids[2])].geom;
    fx.server.state.arrange_monitor(1);
    let settled = fx.server.state.clients[idx_of(&fx, ids[2])].geom;
    assert_ne!(
        half, settled,
        "the survivor should grow into the freed space"
    );
    assert_eq!(
        after, settled,
        "the closed window's monitor must be re-arranged on close"
    );
    assert_eq!(focused_id(&fx), Some(ids[0]));
}

/// The shared removal path recalls focus for the focused window (the X11
/// path used to skip this and leave focus on a dead window).
#[test]
fn remove_client_recalls_focus() {
    let mut fx = desk(1);
    let _a = open(&mut fx);
    let _b = open(&mut fx);
    let ids: Vec<u64> = fx.server.state.clients.iter().map(|c| c.id).collect();
    focus(&mut fx, ids[0]);
    focus(&mut fx, ids[1]);

    fx.server.state.remove_client(idx_of(&fx, ids[1]));

    assert_eq!(fx.server.state.clients.len(), 1);
    assert_eq!(focused_id(&fx), Some(ids[0]));
}

/// The shared removal path queues the close animation (the X11 path used
/// to skip it, so X11 windows vanished without one).
#[test]
fn remove_client_queues_the_close_animation() {
    let mut fx = desk(1);
    let _a = open(&mut fx);
    fx.server.state.config.animations = true;
    fx.server.state.config.animation_duration_close = 200;
    assert!(fx.server.state.closing_clients.is_empty());

    fx.server.state.remove_client(0);

    assert!(fx.server.state.clients.is_empty());
    assert_eq!(fx.server.state.closing_clients.len(), 1);
}
