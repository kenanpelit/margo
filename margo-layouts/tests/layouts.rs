//! Integration tests for the 14 tiling algorithms — pure geometry, no
//! Wayland. These pin the invariants every layout must hold (placement,
//! index preservation, containment), a few exact geometries for the
//! master-stack layouts, and the `LayoutId` name/symbol round-trips.

use margo_layouts::{ArrangeCtx, GapConfig, LayoutId, Rect, arrange};

const WA: Rect = Rect {
    x: 0,
    y: 0,
    width: 1000,
    height: 600,
};

/// Build a context for `work_area = WA` with the given clients. `props`
/// must outlive the context, so callers pass it in.
fn ctx<'a>(
    tiled: &'a [usize],
    gaps: &'a GapConfig,
    props: &'a [f32],
    nmaster: u32,
    mfact: f32,
) -> ArrangeCtx<'a> {
    ArrangeCtx {
        work_area: WA,
        tiled,
        nmaster,
        mfact,
        gaps,
        scroller_proportions: props,
        default_scroller_proportion: 0.8,
        focused_tiled_pos: if tiled.is_empty() { None } else { Some(0) },
        scroller_structs: 24,
        scroller_focus_center: true,
        scroller_prefer_center: true,
        scroller_prefer_overspread: false,
        canvas_pan: (0.0, 0.0),
    }
}

/// Every layout the dispatcher knows about, including `Overview`.
const ALL_LAYOUTS: &[LayoutId] = &[
    LayoutId::Tile,
    LayoutId::Scroller,
    LayoutId::Grid,
    LayoutId::Monocle,
    LayoutId::Deck,
    LayoutId::CenterTile,
    LayoutId::RightTile,
    LayoutId::TgMix,
    LayoutId::Canvas,
    LayoutId::Dwindle,
    LayoutId::Overview,
];

/// Layouts that fit every client inside the work area. Excludes the
/// scrollers (which intentionally push clients off-screen) and Canvas
/// (which arranges via a separate render-time path and returns nothing).
const CONTAINED: &[LayoutId] = &[
    LayoutId::Tile,
    LayoutId::RightTile,
    LayoutId::Monocle,
    LayoutId::Grid,
    LayoutId::Deck,
    LayoutId::CenterTile,
    LayoutId::TgMix,
    LayoutId::Dwindle,
    LayoutId::Overview,
];

fn props_for(tiled: &[usize]) -> Vec<f32> {
    vec![0.8; tiled.len()]
}

#[test]
fn empty_tiled_yields_no_rects_for_every_layout() {
    let gaps = GapConfig::default();
    let props: [f32; 0] = [];
    let c = ctx(&[], &gaps, &props, 1, 0.55);
    for &layout in ALL_LAYOUTS {
        assert!(
            arrange(layout, &c).is_empty(),
            "{layout:?} produced rects for an empty client list"
        );
    }
}

#[test]
fn each_layout_places_every_client_exactly_once() {
    let gaps = GapConfig::default();
    let tiled = [10usize, 11, 12, 13, 14];
    let props = props_for(&tiled);
    let c = ctx(&tiled, &gaps, &props, 1, 0.55);

    for &layout in ALL_LAYOUTS {
        // Canvas is the one exception — it positions clients at render
        // time and returns nothing through the arrange path.
        if layout == LayoutId::Canvas {
            assert!(arrange(layout, &c).is_empty());
            continue;
        }
        let out = arrange(layout, &c);
        assert_eq!(out.len(), tiled.len(), "{layout:?} dropped/added clients");
        let mut got: Vec<usize> = out.iter().map(|(idx, _)| *idx).collect();
        got.sort_unstable();
        assert_eq!(got, tiled.to_vec(), "{layout:?} placed the wrong indices");
    }
}

#[test]
fn contained_layouts_keep_every_rect_inside_the_work_area() {
    let gaps = GapConfig::default();
    for n in [1usize, 2, 3, 5, 8] {
        let tiled: Vec<usize> = (0..n).collect();
        let props = props_for(&tiled);
        let c = ctx(&tiled, &gaps, &props, 1, 0.55);
        for &layout in CONTAINED {
            for (idx, r) in arrange(layout, &c) {
                assert!(
                    r.x >= WA.x
                        && r.y >= WA.y
                        && r.x + r.width <= WA.x + WA.width
                        && r.y + r.height <= WA.y + WA.height,
                    "{layout:?} n={n} client {idx} escaped the work area: {r:?}"
                );
                assert!(
                    r.width > 0 && r.height > 0,
                    "{layout:?} gave client {idx} a degenerate rect: {r:?}"
                );
            }
        }
    }
}

#[test]
fn tile_single_client_fills_the_work_area() {
    let gaps = GapConfig::default();
    let tiled = [7usize];
    let props = props_for(&tiled);
    let out = arrange(LayoutId::Tile, &ctx(&tiled, &gaps, &props, 1, 0.55));
    assert_eq!(out, vec![(7, WA)]);
}

#[test]
fn tile_puts_master_left_of_stack() {
    let gaps = GapConfig::default();
    let tiled = [1usize, 2];
    let props = props_for(&tiled);
    let out = arrange(LayoutId::Tile, &ctx(&tiled, &gaps, &props, 1, 0.5));
    let master = out[0].1;
    let stack = out[1].1;
    assert!(
        master.x < stack.x,
        "master {master:?} should sit left of stack {stack:?}"
    );
    // No overlap horizontally.
    assert!(master.x + master.width <= stack.x);
}

#[test]
fn right_tile_puts_master_right_of_stack() {
    let gaps = GapConfig::default();
    let tiled = [1usize, 2];
    let props = props_for(&tiled);
    let out = arrange(LayoutId::RightTile, &ctx(&tiled, &gaps, &props, 1, 0.5));
    let master = out[0].1;
    let stack = out[1].1;
    assert!(
        master.x > stack.x,
        "master {master:?} should sit right of stack {stack:?}"
    );
}

#[test]
fn tile_mfact_controls_master_width() {
    let gaps = GapConfig::default();
    let tiled = [1usize, 2];
    let props = props_for(&tiled);
    let narrow = arrange(LayoutId::Tile, &ctx(&tiled, &gaps, &props, 1, 0.4))[0].1;
    let wide = arrange(LayoutId::Tile, &ctx(&tiled, &gaps, &props, 1, 0.6))[0].1;
    assert!(
        wide.width > narrow.width,
        "mfact 0.6 master ({}) should be wider than mfact 0.4 ({})",
        wide.width,
        narrow.width
    );
}

#[test]
fn monocle_maximises_every_client_to_the_same_rect() {
    let gaps = GapConfig::default();
    let tiled = [3usize, 4, 5];
    let props = props_for(&tiled);
    let out = arrange(LayoutId::Monocle, &ctx(&tiled, &gaps, &props, 1, 0.55));
    for (_, r) in &out {
        assert_eq!(*r, WA, "monocle client not maximised: {r:?}");
    }
}

#[test]
fn outer_gaps_inset_the_monocle_rect() {
    let gaps = GapConfig {
        gappih: 0,
        gappiv: 0,
        gappoh: 10,
        gappov: 20,
    };
    let tiled = [1usize];
    let props = props_for(&tiled);
    let out = arrange(LayoutId::Monocle, &ctx(&tiled, &gaps, &props, 1, 0.55));
    // x+oh, y+ov, width-2*oh, height-2*ov.
    assert_eq!(out[0].1, Rect::new(10, 20, 1000 - 20, 600 - 40));
}

#[test]
fn canvas_arranges_nothing_through_the_normal_path() {
    let gaps = GapConfig::default();
    let tiled = [1usize, 2, 3];
    let props = props_for(&tiled);
    assert!(arrange(LayoutId::Canvas, &ctx(&tiled, &gaps, &props, 1, 0.55)).is_empty());
}

#[test]
fn nmaster_at_least_client_count_makes_one_column() {
    // With nmaster >= n, every client is a master → a single column,
    // so all rects share the same x and width.
    let gaps = GapConfig::default();
    let tiled = [1usize, 2, 3];
    let props = props_for(&tiled);
    let out = arrange(LayoutId::Tile, &ctx(&tiled, &gaps, &props, 5, 0.55));
    let x0 = out[0].1.x;
    let w0 = out[0].1.width;
    for (_, r) in &out {
        assert_eq!(r.x, x0);
        assert_eq!(r.width, w0);
    }
    // And the single column spans the full width (no stack).
    assert_eq!(w0, WA.width);
}

// ── LayoutId metadata ─────────────────────────────────────────────────────────

#[test]
fn layout_names_round_trip() {
    for &l in LayoutId::all_tileable() {
        assert_eq!(
            LayoutId::from_name(l.name()),
            Some(l),
            "name round-trip failed for {l:?}"
        );
    }
}

#[test]
fn layout_symbols_round_trip() {
    for &l in LayoutId::all_tileable() {
        assert_eq!(
            LayoutId::from_symbol(l.symbol()),
            Some(l),
            "symbol round-trip failed for {l:?}"
        );
    }
}

#[test]
fn all_tileable_has_14_entries_and_excludes_overview() {
    let tileable = LayoutId::all_tileable();
    assert_eq!(tileable.len(), 14);
    assert!(!tileable.contains(&LayoutId::Overview));
}

#[test]
fn unknown_name_and_symbol_are_none() {
    assert_eq!(LayoutId::from_name("not-a-layout"), None);
    assert_eq!(LayoutId::from_symbol("??"), None);
}
