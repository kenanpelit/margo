//! Tag-mask bounds. Margo has `MAX_TAGS` (9) tags, and every per-tag table
//! (`Pertag::ltidxs`, `mfacts`, …) has `MAX_TAGS + 1` slots. A tag mask with
//! a bit above bit 8 used to reach `update_pertag_for_tagset`, which turned
//! bit 9 into `curtag = 10` and the next `current_layout()` indexed past the
//! table: `mctl dispatch view 512` crashed the compositor.

use super::fixture::Fixture;
use crate::dispatch::dispatch_action;
use margo_config::Arg;

fn int_arg(i: i32) -> Arg {
    Arg {
        i,
        ..Default::default()
    }
}

fn one_window() -> Fixture {
    let mut fx = Fixture::new();
    fx.add_keyboard();
    fx.add_output("DP-1", (1920, 1080));
    let id = fx.add_client();
    let (_toplevel, surface) = fx.client(id).create_toplevel();
    surface.commit();
    fx.client(id).flush();
    fx.roundtrip(id);
    fx
}

fn assert_pertag_in_bounds(fx: &Fixture) {
    for mon in &fx.server.state.monitors {
        assert!(
            mon.pertag.curtag <= crate::MAX_TAGS,
            "curtag {}",
            mon.pertag.curtag
        );
        assert!(
            mon.pertag.prevtag <= crate::MAX_TAGS,
            "prevtag {}",
            mon.pertag.prevtag
        );
        assert_eq!(
            mon.tagset[0] & !crate::TAGMASK,
            0,
            "tagset[0] {:#x}",
            mon.tagset[0]
        );
        assert_eq!(
            mon.tagset[1] & !crate::TAGMASK,
            0,
            "tagset[1] {:#x}",
            mon.tagset[1]
        );
        let _ = mon.current_layout();
        let _ = mon.current_mfact();
    }
    for c in &fx.server.state.clients {
        assert_ne!(c.tags, 0, "a window must keep at least one tag");
    }
}

#[test]
fn view_of_a_tag_above_max_tags_is_ignored() {
    let mut fx = one_window();
    let before = fx.server.state.monitors[0].current_tagset();

    // 512 = bit 9 = "tag 10", which doesn't exist.
    let _ = dispatch_action(&mut fx.server.state, "view", &int_arg(512));

    assert_pertag_in_bounds(&fx);
    assert_eq!(fx.server.state.monitors[0].current_tagset(), before);
}

#[test]
fn view_with_high_bits_keeps_only_real_tags() {
    let mut fx = one_window();
    // tag 2 plus bit 9 and bit 20.
    let _ = dispatch_action(
        &mut fx.server.state,
        "view",
        &int_arg((1 << 1) | (1 << 9) | (1 << 20)),
    );

    assert_pertag_in_bounds(&fx);
    assert_eq!(fx.server.state.monitors[0].current_tagset(), 1 << 1);
    assert_eq!(fx.server.state.monitors[0].pertag.curtag, 2);
}

#[test]
fn ui_slot_is_masked_too() {
    // Rhai sets the `ui` slot; it goes through the same `tag_arg`.
    let mut fx = one_window();
    let arg = Arg {
        ui: 1 << 20,
        ..Default::default()
    };
    let _ = dispatch_action(&mut fx.server.state, "view", &arg);
    let _ = dispatch_action(&mut fx.server.state, "toggleview", &arg);
    let _ = dispatch_action(&mut fx.server.state, "tag", &arg);
    let _ = dispatch_action(&mut fx.server.state, "tagview", &arg);
    let _ = dispatch_action(&mut fx.server.state, "toggletag", &arg);
    assert_pertag_in_bounds(&fx);
}

#[test]
fn every_tag_action_rejects_out_of_range_masks() {
    let mut fx = one_window();
    for action in [
        "view",
        "toggleview",
        "tag",
        "tagsilent",
        "tagview",
        "toggletag",
    ] {
        let _ = dispatch_action(&mut fx.server.state, action, &int_arg(1 << 12));
        assert_pertag_in_bounds(&fx);
    }
    assert_eq!(
        fx.server.state.clients[0].tags, 1,
        "the window stays on tag 1"
    );
}

#[test]
fn toggleview_of_a_high_bit_does_not_add_it_to_the_tagset() {
    let mut fx = one_window();
    let _ = dispatch_action(
        &mut fx.server.state,
        "toggleview",
        &int_arg((1 << 2) | (1 << 9)),
    );
    assert_pertag_in_bounds(&fx);
    assert_eq!(fx.server.state.monitors[0].current_tagset(), 0b101);
}

#[test]
fn settagset_masks_and_rejects_empty_after_masking() {
    let mut fx = one_window();
    let _ = dispatch_action(&mut fx.server.state, "settagset", &int_arg(1 << 10));
    assert_pertag_in_bounds(&fx);
    assert_eq!(fx.server.state.monitors[0].current_tagset(), 1);

    let _ = dispatch_action(&mut fx.server.state, "settagset", &int_arg(-1));
    assert_pertag_in_bounds(&fx);
    assert_eq!(fx.server.state.monitors[0].current_tagset(), crate::TAGMASK);
}

#[test]
fn setclienttags_cannot_set_bits_above_max_tags() {
    let mut fx = one_window();
    let arg = Arg {
        i: -1,       // keep everything
        i2: 1 << 15, // xor in a non-existent tag
        ..Default::default()
    };
    let _ = dispatch_action(&mut fx.server.state, "setclienttags", &arg);
    assert_eq!(fx.server.state.clients[0].tags & !crate::TAGMASK, 0);
    assert_pertag_in_bounds(&fx);
}

#[test]
fn tagall_views_exactly_the_real_tags() {
    let mut fx = one_window();
    let _ = dispatch_action(&mut fx.server.state, "tagall", &Arg::default());
    assert_eq!(fx.server.state.monitors[0].current_tagset(), crate::TAGMASK);
    assert_pertag_in_bounds(&fx);
}

#[test]
fn state_methods_mask_direct_callers() {
    // Scripting and internal callers reach the state methods without going
    // through `dispatch_action`; they must be safe on their own.
    let mut fx = one_window();
    fx.server.state.view_tag(1 << 9);
    fx.server.state.toggle_view_tag(1 << 9);
    fx.server.state.tag_focused(1 << 9);
    fx.server.state.toggle_client_tag(1 << 9);
    fx.server.state.tag_view(1 << 9);
    fx.server.state.update_pertag_for_tagset(0, 1 << 9);
    assert_pertag_in_bounds(&fx);
}
