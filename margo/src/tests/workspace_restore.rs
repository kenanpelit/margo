//! Workspace restore (`session/windows.rs`) against real Wayland clients:
//! the snapshot's per-tag app lists, moving open windows back, and the
//! placement a relaunched app's window gets on its first map.

use super::fixture::Fixture;
use crate::session::{SessionSnapshot, WindowEntry};

/// Two outputs and one mapped toplevel per app id; clients land at state
/// indices 0, 1, ... on DP-1, tag 1.
fn windows(apps: &[&str]) -> Fixture {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_output("DP-2", (1920, 1080));
    fx.server.state.config.animations = false;
    let id = fx.add_client();
    for app in apps {
        map(&mut fx, id, app);
    }
    fx
}

fn map(fx: &mut Fixture, id: super::client::ClientId, app: &str) {
    let (toplevel, surface) = fx.client(id).create_toplevel();
    toplevel.set_app_id(app.into());
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
}

fn place(fx: &Fixture, app: &str) -> (String, u32) {
    let s = &fx.server.state;
    let c = s.clients.iter().find(|c| c.app_id == app).expect(app);
    (s.monitors[c.monitor].name.clone(), c.tags)
}

#[test]
fn snapshot_lists_every_window_with_its_place() {
    let mut fx = windows(&["kitty", "firefox"]);
    let mon0 = fx.server.state.monitors[0].name.clone();
    let snap = SessionSnapshot::capture(&mut fx.server.state);
    let got: Vec<(&str, &str, u32)> = snap
        .windows
        .iter()
        .map(|w| (w.app_id.as_str(), w.monitor.as_str(), w.tags))
        .collect();
    let kitty_tags = fx.server.state.clients[0].tags;
    assert_eq!(
        got,
        [
            ("kitty", mon0.as_str(), kitty_tags),
            ("firefox", mon0.as_str(), kitty_tags)
        ]
    );
    // Both windows belong to this test process (one pid): only the first
    // carries the launch command, so a restore launches the app once.
    let with_cmd = snap
        .windows
        .iter()
        .filter(|w| !w.command.is_empty())
        .count();
    assert!(
        with_cmd <= 1,
        "one process → at most one launch: {:?}",
        snap.windows
    );
}

#[test]
fn restore_moves_open_windows_back_and_leaves_unsaved_ones() {
    let mut fx = windows(&["kitty", "firefox", "mpv"]);
    let mut snap = SessionSnapshot::capture(&mut fx.server.state);
    snap.windows.retain(|w| w.app_id != "mpv");
    for w in &mut snap.windows {
        w.command.clear(); // never spawn from a test
    }
    // Saved: kitty on DP-2 tag 3, firefox on DP-1 tag 2.
    snap.windows[0].monitor = "DP-2".into();
    snap.windows[0].tags = 1 << 2;
    snap.windows[1].tags = 1 << 1;
    let mpv_before = place(&fx, "mpv");

    let r = fx.server.state.restore_session_windows(&snap);
    assert_eq!((r.moved, r.launched), (2, 0));
    assert_eq!(place(&fx, "kitty"), ("DP-2".into(), 1 << 2));
    assert_eq!(place(&fx, "firefox").1, 1 << 1);
    assert_eq!(place(&fx, "mpv"), mpv_before, "not on the list: untouched");

    // Run again: everything is in place, nothing moves.
    let r = fx.server.state.restore_session_windows(&snap);
    assert_eq!(r.moved, 0);
}

#[test]
fn a_missing_apps_window_lands_on_its_saved_tag() {
    let mut fx = windows(&["kitty"]);
    let snap = SessionSnapshot {
        windows: vec![WindowEntry {
            app_id: "obsidian".into(),
            monitor: "DP-2".into(),
            tags: 1 << 5,
            floating: false,
            command: Vec::new(), // the app reopens it itself
        }],
        ..SessionSnapshot::capture(&mut fx.server.state)
    };
    let r = fx.server.state.restore_session_windows(&snap);
    assert_eq!((r.moved, r.launched), (0, 0));
    let id = fx.add_client();
    map(&mut fx, id, "obsidian");
    assert_eq!(place(&fx, "obsidian"), ("DP-2".into(), 1 << 5));
    // Consumed: a second obsidian window opens normally.
    map(&mut fx, id, "obsidian");
    let second = fx
        .server
        .state
        .clients
        .iter()
        .filter(|c| c.app_id == "obsidian")
        .nth(1)
        .unwrap();
    assert_ne!(second.tags, 1 << 5);
}

#[test]
fn auto_add_queues_new_tiled_apps_once() {
    let mut fx = windows(&[]);
    fx.server.state.config.session_auto_add = true;
    let id = fx.add_client();
    map(&mut fx, id, "kitty");
    map(&mut fx, id, "kitty");
    map(&mut fx, id, "obsidian");
    let queued: Vec<&str> = fx
        .server
        .state
        .session
        .auto_add
        .iter()
        .map(|e| e.app_id.as_str())
        .collect();
    assert_eq!(queued, ["kitty", "obsidian"]);
}

#[test]
fn auto_add_is_off_by_default() {
    let mut fx = windows(&["kitty"]);
    let id = fx.add_client();
    map(&mut fx, id, "obsidian");
    assert!(fx.server.state.session.auto_add.is_empty());
}
