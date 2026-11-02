//! `session::apply_to_state` against corrupt snapshots.
//!
//! The crash-recovery autosave is applied at startup, so a snapshot with an
//! out-of-range field used to panic margo on every start (a crash loop):
//! `seltags >= 2` indexes `tagset[seltags]` out of bounds, `curtag > MAX_TAGS`
//! indexes past the `Pertag` tables, and NaN / negative `mfacts` went straight
//! into the layouts.

use super::fixture::Fixture;
use crate::session::{
    CURRENT_VERSION, MonitorSnapshot, PertagSnapshot, SessionSnapshot, apply_to_state,
};

fn fixture() -> Fixture {
    let mut fx = Fixture::new();
    fx.add_output("DP-1", (1920, 1080));
    fx
}

fn snapshot(mon: MonitorSnapshot) -> SessionSnapshot {
    SessionSnapshot {
        version: CURRENT_VERSION,
        captured_at: "x".into(),
        monitors: vec![mon],
        scratchpads: vec![],
    }
}

fn monitor(seltags: usize, tagset: [u32; 2], curtag: usize, prevtag: usize) -> MonitorSnapshot {
    MonitorSnapshot {
        name: "DP-1".into(),
        seltags,
        tagset,
        pertag: PertagSnapshot {
            curtag,
            prevtag,
            ltidxs: vec!["tile".into(); crate::MAX_TAGS + 1],
            mfacts: vec![0.55; crate::MAX_TAGS + 1],
            nmasters: vec![1; crate::MAX_TAGS + 1],
        },
    }
}

/// Everything that indexes with a restored value must be in bounds.
fn assert_usable(fx: &Fixture) {
    let m = &fx.server.state.monitors[0];
    assert!(m.seltags < 2, "seltags {}", m.seltags);
    assert!(
        m.pertag.curtag <= crate::MAX_TAGS,
        "curtag {}",
        m.pertag.curtag
    );
    assert!(
        m.pertag.prevtag <= crate::MAX_TAGS,
        "prevtag {}",
        m.pertag.prevtag
    );
    assert_ne!(m.current_tagset(), 0, "the active tagset must show a tag");
    assert_eq!(m.tagset[0] & !crate::TAGMASK, 0);
    assert_eq!(m.tagset[1] & !crate::TAGMASK, 0);
    for f in &m.pertag.mfacts {
        assert!(f.is_finite() && (0.05..=0.95).contains(f), "mfact {f}");
    }
    let _ = m.current_layout();
    let _ = m.current_mfact();
}

#[test]
fn out_of_range_seltags_is_folded_into_the_two_slots() {
    let mut fx = fixture();
    let snap = snapshot(monitor(7, [1 << 2, 1 << 4], 3, 5));

    assert_eq!(apply_to_state(&mut fx.server.state, &snap), 1);

    assert_usable(&fx);
    // 7 & 1 == 1: the second slot is the active one.
    assert_eq!(fx.server.state.monitors[0].current_tagset(), 1 << 4);
    // `settagset ... i2` writes the other slot via `seltags ^ 1` — must not
    // underflow or index out of bounds after the restore.
    fx.server.state.view_tag(1 << 1);
    assert_usable(&fx);
}

#[test]
fn out_of_range_curtag_and_prevtag_are_not_restored() {
    let mut fx = fixture();
    let snap = snapshot(monitor(0, [1 << 2, 1], 99, usize::MAX));

    apply_to_state(&mut fx.server.state, &snap);

    assert_usable(&fx);
    // curtag is re-derived from the single active tag (tag 3).
    assert_eq!(fx.server.state.monitors[0].pertag.curtag, 3);
}

#[test]
fn tagset_bits_above_max_tags_are_dropped() {
    let mut fx = fixture();
    let snap = snapshot(monitor(0, [(1 << 1) | (1 << 12), u32::MAX], 2, 1));

    apply_to_state(&mut fx.server.state, &snap);

    assert_usable(&fx);
    assert_eq!(fx.server.state.monitors[0].tagset, [1 << 1, crate::TAGMASK]);
}

#[test]
fn an_active_tagset_with_no_real_tag_keeps_the_live_one() {
    let mut fx = fixture();
    let live = fx.server.state.monitors[0].tagset;
    let live_sel = fx.server.state.monitors[0].seltags;
    let snap = snapshot(monitor(0, [1 << 20, 0], 1, 1));

    apply_to_state(&mut fx.server.state, &snap);

    assert_usable(&fx);
    assert_eq!(fx.server.state.monitors[0].tagset, live);
    assert_eq!(fx.server.state.monitors[0].seltags, live_sel);
}

#[test]
fn non_finite_and_out_of_range_mfacts_are_sanitised() {
    let mut fx = fixture();
    let live = fx.server.state.monitors[0].pertag.mfacts.clone();
    let mut mon = monitor(0, [1, 2], 1, 1);
    mon.pertag.mfacts = vec![f32::NAN, f32::INFINITY, -3.0, 7.0, 0.3];

    apply_to_state(&mut fx.server.state, &snapshot(mon));

    assert_usable(&fx);
    let m = &fx.server.state.monitors[0].pertag.mfacts;
    assert_eq!(m[0], live[0], "NaN keeps the live value");
    assert_eq!(m[1], live[1], "inf keeps the live value");
    assert_eq!(m[2], 0.05);
    assert_eq!(m[3], 0.95);
    assert_eq!(m[4], 0.3);
}

#[test]
fn a_valid_snapshot_still_round_trips() {
    let mut fx = fixture();
    let snap = snapshot(monitor(1, [1 << 3, 1 << 5], 6, 4));

    apply_to_state(&mut fx.server.state, &snap);

    let m = &fx.server.state.monitors[0];
    assert_eq!(m.seltags, 1);
    assert_eq!(m.tagset, [1 << 3, 1 << 5]);
    assert_eq!(m.pertag.curtag, 6);
    assert_eq!(m.pertag.prevtag, 4);
}
