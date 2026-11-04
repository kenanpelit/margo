//! Compositor state the shell's bar indicators read from `watch state`.
//! Each field must be in the snapshot and a change must push a frame
//! (mark the state dirty).

use margo_config::Arg;

use super::fixture::Fixture;
use crate::dispatch::dispatch_action;
use crate::layout::LayoutId;

/// Map one xdg toplevel with `app_id` on a fresh client connection.
fn map_window(fx: &mut Fixture, app_id: &str) -> u64 {
    let peer = fx.add_client();
    let (toplevel, surface) = fx.client(peer).create_toplevel();
    toplevel.set_app_id(app_id.into());
    surface.commit();
    fx.client(peer).flush();
    fx.roundtrip(peer);
    fx.server
        .state
        .clients
        .iter()
        .find(|c| c.app_id == app_id)
        .expect("mapped client")
        .id
}

fn arg_v(v: &str) -> Arg {
    Arg {
        v: Some(v.to_string()),
        ..Arg::default()
    }
}

#[test]
fn keymode_is_published_and_a_change_pushes_a_frame() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    assert_eq!(
        fx.server.state.build_state_snapshot()["keymode"],
        "default",
        "a fresh session starts in the default keymode"
    );

    fx.server.state.state_dirty.set(false);
    dispatch_action(&mut fx.server.state, "setkeymode", &arg_v("resize"));
    assert!(
        fx.server.state.state_dirty.get(),
        "a keymode change pushes a frame"
    );
    assert_eq!(fx.server.state.build_state_snapshot()["keymode"], "resize");

    fx.server.state.state_dirty.set(false);
    dispatch_action(&mut fx.server.state, "setkeymode", &arg_v("resize"));
    assert!(
        !fx.server.state.state_dirty.get(),
        "re-entering the same keymode is not a change"
    );
}

#[test]
fn scroller_viewport_lists_every_column_relative_to_the_work_area() {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx.add_keyboard();
    let curtag = fx.server.state.monitors[0].pertag.curtag;
    fx.server.state.monitors[0].pertag.ltidxs[curtag] = LayoutId::Scroller;
    let ids = [
        map_window(&mut fx, "alpha"),
        map_window(&mut fx, "beta"),
        map_window(&mut fx, "gamma"),
    ];
    fx.server.state.arrange_monitor(0);

    let snap = fx.server.state.build_state_snapshot();
    let scroller = &snap["outputs"][0]["scroller"];
    let work_w = fx.server.state.monitors[0].work_area.width;
    assert_eq!(scroller["viewport_width"], work_w);
    let cols = scroller["columns"].as_array().expect("columns");
    assert_eq!(cols.len(), 3, "one column per tiled window");
    let col_ids: Vec<u64> = cols.iter().map(|c| c["id"].as_u64().unwrap()).collect();
    assert_eq!(col_ids, ids, "columns in strip order");
    let xs: Vec<i64> = cols.iter().map(|c| c["x"].as_i64().unwrap()).collect();
    assert!(
        xs.windows(2).all(|w| w[0] < w[1]),
        "strip runs left to right: {xs:?}"
    );
    // Three default-proportion columns never fit one 1080p viewport, so at
    // least one column reaches past an edge.
    let off = cols.iter().any(|c| {
        let x = c["x"].as_i64().unwrap();
        let w = c["width"].as_i64().unwrap();
        x < 0 || x + w > i64::from(work_w)
    });
    assert!(off, "some column is outside the viewport: {cols:?}");
    assert_eq!(
        cols.iter().filter(|c| c["focused"] == true).count(),
        1,
        "exactly one focused column"
    );

    // Not the scroller layout -> no viewport object.
    fx.server.state.monitors[0].pertag.ltidxs[curtag] = LayoutId::Tile;
    fx.server.state.arrange_monitor(0);
    assert!(fx.server.state.build_state_snapshot()["outputs"][0]["scroller"].is_null());
}
