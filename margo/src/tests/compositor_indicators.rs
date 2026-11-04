//! Compositor state the shell's bar indicators read from `watch state`.
//! Each field must be in the snapshot and a change must push a frame
//! (mark the state dirty).

use margo_config::Arg;

use super::fixture::Fixture;
use crate::dispatch::dispatch_action;

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
