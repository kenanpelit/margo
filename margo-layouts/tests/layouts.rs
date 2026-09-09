//! Integration tests for the 14 tiling algorithms — pure geometry, no
//! Wayland. These pin the invariants every layout must hold (placement,
//! index preservation, containment), a few exact geometries for the
//! master-stack layouts, and the `LayoutId` name/symbol round-trips.

use margo_layouts::{
    ArrangeCtx, GapConfig, LayoutId, MosaicClient, Rect, arrange, mosaic_arrange,
    mosaic_overflow_client, mosaic_stack_peek_rect, place_floating_cascade,
};

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
    LayoutId::Dwindle,
    LayoutId::Floating,
    LayoutId::Overview,
];

/// Layouts that fit every client inside the work area. Excludes the
/// scrollers (which intentionally push clients off-screen).
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
        // Floating is the exception — it positions clients outside the
        // arrange path and returns nothing.
        if layout == LayoutId::Floating {
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
fn all_tileable_has_11_entries_and_excludes_overview() {
    let tileable = LayoutId::all_tileable();
    assert_eq!(tileable.len(), 11);
    assert!(!tileable.contains(&LayoutId::Overview));
    assert!(tileable.contains(&LayoutId::Floating));
    assert!(tileable.contains(&LayoutId::Mosaic));
}

#[test]
fn unknown_name_and_symbol_are_none() {
    assert_eq!(LayoutId::from_name("not-a-layout"), None);
    assert_eq!(LayoutId::from_symbol("??"), None);
}

// ── floating ──────────────────────────────────────────────────────────────────

#[test]
fn floating_arranges_nothing_through_the_normal_path() {
    let gaps = GapConfig::default();
    for n in [0usize, 1, 3, 8] {
        let tiled: Vec<usize> = (0..n).collect();
        let props = props_for(&tiled);
        assert!(
            arrange(LayoutId::Floating, &ctx(&tiled, &gaps, &props, 1, 0.55)).is_empty(),
            "floating produced rects for n={n}"
        );
    }
}

#[test]
fn floating_name_and_symbol_round_trip() {
    assert_eq!(LayoutId::from_name("floating"), Some(LayoutId::Floating));
    assert_eq!(LayoutId::from_symbol("F"), Some(LayoutId::Floating));
    assert_eq!(LayoutId::Floating.name(), "floating");
    assert_eq!(LayoutId::Floating.symbol(), "F");
}

const FWA: Rect = Rect {
    x: 0,
    y: 0,
    width: 1000,
    height: 600,
};

#[test]
fn place_floating_falls_back_to_60_percent_when_no_preferred_size() {
    let r = place_floating_cascade(FWA, None, (0, 0), (0, 0), 0);
    assert_eq!(r, Rect::new(24, 24, 600, 360));
}

#[test]
fn place_floating_cascades_down_and_right_by_32px() {
    assert_eq!(
        place_floating_cascade(FWA, None, (0, 0), (0, 0), 1),
        Rect::new(56, 56, 600, 360)
    );
    assert_eq!(
        place_floating_cascade(FWA, None, (0, 0), (0, 0), 2),
        Rect::new(88, 88, 600, 360)
    );
}

#[test]
fn place_floating_wraps_the_cascade() {
    // 60% box on FWA leaves 216px of vertical slack; step 32 → wrap every 6.
    let base = place_floating_cascade(FWA, None, (0, 0), (0, 0), 0);
    assert_eq!(place_floating_cascade(FWA, None, (0, 0), (0, 0), 6), base);
}

#[test]
fn place_floating_uses_the_committed_size_when_it_fits() {
    let r = place_floating_cascade(FWA, Some((800, 400)), (0, 0), (0, 0), 0);
    assert_eq!(r, Rect::new(24, 24, 800, 400));
}

#[test]
fn place_floating_ignores_a_committed_size_that_does_not_fit() {
    let r = place_floating_cascade(FWA, Some((1200, 700)), (0, 0), (0, 0), 0);
    assert_eq!(r, Rect::new(24, 24, 600, 360)); // fell back to 60%
}

#[test]
fn place_floating_honours_min_constraints() {
    let r = place_floating_cascade(FWA, Some((100, 80)), (400, 300), (0, 0), 0);
    assert_eq!(r, Rect::new(24, 24, 400, 300));
}

#[test]
fn place_floating_keeps_the_rect_inside_the_work_area() {
    for idx in 0..40usize {
        let r = place_floating_cascade(FWA, None, (0, 0), (0, 0), idx);
        assert!(r.x >= FWA.x && r.y >= FWA.y);
        assert!(r.x + r.width <= FWA.x + FWA.width);
        assert!(r.y + r.height <= FWA.y + FWA.height);
    }
}

#[test]
fn place_floating_respects_a_non_zero_work_area_origin() {
    let wa = Rect::new(100, 50, 1000, 600);
    let r = place_floating_cascade(wa, None, (0, 0), (0, 0), 0);
    assert_eq!(r, Rect::new(124, 74, 600, 360));
}

// ── Mosaic (`mosaic_arrange`) ────────────────────────────────────────────────

const MOSAIC_GAPS: GapConfig = GapConfig {
    gappih: 8,
    gappiv: 8,
    gappoh: 0,
    gappov: 0,
};

#[test]
fn mosaic_empty_input_yields_no_rects() {
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &[]);
    assert!(result.is_empty());
}

#[test]
fn mosaic_one_client_gets_exactly_its_ideal_size() {
    let clients = [MosaicClient {
        index: 7,
        id: 7_u64,
        ideal: (400, 300),
        min: (0, 0),
        max: (0, 0),
    }];
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &clients);
    assert_eq!(result.len(), 1);
    let (idx, rect) = result[0];
    assert_eq!(idx, 7);
    assert_eq!((rect.width, rect.height), (400, 300));
}

#[test]
fn mosaic_centers_a_short_stack_vertically_not_just_horizontally() {
    // A lone window (or a couple of short rows) shouldn't pin to the top
    // edge with all the leftover height dumped below it — the article's
    // "windows open in the center of the screen" means centered on both
    // axes. `MOSAIC_GAPS` has zero outer gap, so the packing area is
    // exactly `WA` and the expected offset is exact arithmetic.
    let clients = [MosaicClient {
        index: 0,
        id: 0_u64,
        ideal: (400, 300),
        min: (0, 0),
        max: (0, 0),
    }];
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &clients);
    let (_, rect) = result[0];
    assert_eq!(rect.height, 300);
    assert_eq!(
        rect.y,
        WA.y + (WA.height - rect.height) / 2,
        "a single short client should be centered top-to-bottom, not pinned to the top"
    );
}

#[test]
fn mosaic_two_clients_that_fit_share_one_row() {
    let clients = [
        MosaicClient {
            index: 0,
            id: 0_u64,
            ideal: (300, 300),
            min: (0, 0),
            max: (0, 0),
        },
        MosaicClient {
            index: 1,
            id: 1_u64,
            ideal: (300, 300),
            min: (0, 0),
            max: (0, 0),
        },
    ];
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &clients);
    assert_eq!(result.len(), 2);
    let ys: Vec<i32> = result.iter().map(|(_, r)| r.y).collect();
    assert_eq!(
        ys[0], ys[1],
        "clients that fit side by side must share a row"
    );
    // Neither rect overlaps the other on the x axis.
    let (r0, r1) = (result[0].1, result[1].1);
    assert!(r0.x + r0.width <= r1.x || r1.x + r1.width <= r0.x);
}

#[test]
fn mosaic_wraps_to_a_new_row_when_it_does_not_fit() {
    // Three clients at 400px wide each: two fit (800 < 1000), the third
    // must wrap rather than overlap or be pushed past the work area.
    let make = |i: usize| MosaicClient {
        index: i,
        id: (i) as u64,
        ideal: (400, 200),
        min: (0, 0),
        max: (0, 0),
    };
    let clients = [make(0), make(1), make(2)];
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &clients);
    assert_eq!(result.len(), 3);
    let rows: std::collections::BTreeSet<i32> = result.iter().map(|(_, r)| r.y).collect();
    assert_eq!(rows.len(), 2, "the third client must wrap to a second row");
}

#[test]
fn mosaic_caps_oversized_ideal_width_so_many_windows_form_a_grid_not_a_column() {
    // Simulates windows that just arrived from `scroller`, where a single
    // column commonly runs 80-90% of the work area's width — captured
    // once as each client's `ideal` the moment Mosaic takes over. Without
    // a width-axis cap mirroring the existing height-shrink pass, none of
    // these 9 clients (900px "ideal" against a 1000px-wide WA) could ever
    // share a row, piling all of them into a single column: wasted width
    // at every row's edges, and — for enough windows — rows overflowing
    // past the work area's bottom.
    let clients: Vec<_> = (0..9)
        .map(|i| MosaicClient {
            index: i,
            id: i as u64,
            ideal: (900, 150),
            min: (0, 0),
            max: (0, 0),
        })
        .collect();
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &clients);
    assert_eq!(result.len(), 9);
    let rows: std::collections::BTreeSet<i32> = result.iter().map(|(_, r)| r.y).collect();
    assert!(
        rows.len() < 9,
        "expected multiple clients per row, got {} distinct rows for 9 clients",
        rows.len()
    );
    for (_, r) in &result {
        assert!(
            r.y + r.height <= WA.height,
            "client overflowed past the bottom of the work area: {r:?}"
        );
        assert!(
            r.x >= WA.x && r.x + r.width <= WA.x + WA.width,
            "client overflowed the work area's width: {r:?}"
        );
    }
}

#[test]
fn mosaic_backfills_an_under_filled_row_instead_of_leaving_it_wasted() {
    // Two 700px-wide clients (each forced onto its own row — 700*2+gap >
    // 1000px WA) followed by two 280px-wide clients that each fit
    // alongside one of the wide ones (700+280+gap <= 1000). A strict
    // left-to-right shelf fill only ever looks at the *last* row, so it
    // packs the second narrow client into a brand new third row instead
    // of noticing the first row still had room — exactly the wasted-edge-
    // space complaint this masonry-style best-fit placement exists to
    // fix. Widths are all below the column cap already (well under half
    // the WA width) so the cap never kicks in here — this test is purely
    // about the row-assignment heuristic.
    let make = |i: usize, w: i32| MosaicClient {
        index: i,
        id: i as u64,
        ideal: (w, 200),
        min: (0, 0),
        max: (0, 0),
    };
    let clients = [make(0, 700), make(1, 700), make(2, 280), make(3, 280)];
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &clients);
    assert_eq!(result.len(), 4);
    let rows: std::collections::BTreeSet<i32> = result.iter().map(|(_, r)| r.y).collect();
    assert_eq!(
        rows.len(),
        2,
        "both narrow clients should backfill the two wide clients' rows, not open a third"
    );
}

#[test]
fn mosaic_shrinks_toward_min_height_when_rows_overflow_vertically() {
    // Five 200px-tall clients that genuinely can't share a row — their
    // own `min_width` (900) is the reason, not just an oversized `ideal`,
    // so the column cap in `pack_rows` can't (and shouldn't) shrink them
    // past it. Five separate rows need 1000px into a 600px-tall work
    // area: every row must shrink, but never below the client's own
    // min_height.
    let make = |i: usize| MosaicClient {
        index: i,
        id: (i) as u64,
        ideal: (1000, 200),
        min: (900, 60),
        max: (0, 0),
    };
    let clients: Vec<_> = (0..5).map(make).collect();
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &clients);
    assert_eq!(result.len(), 5);
    let rows: std::collections::BTreeSet<i32> = result.iter().map(|(_, r)| r.y).collect();
    assert_eq!(
        rows.len(),
        5,
        "each client's own min_width must force its own row"
    );
    for (_, rect) in &result {
        assert!(
            rect.height >= 60,
            "shrank a client below its own min_height"
        );
        assert!(rect.height <= 200, "grew a client past its ideal height");
    }
    assert!(
        result.iter().any(|(_, r)| r.height < 200),
        "five 200px rows can't fit in a 600px area without shrinking at least one"
    );
}

#[test]
fn mosaic_never_exceeds_min_or_max_bounds() {
    let clients = [
        MosaicClient {
            index: 0,
            id: 0_u64,
            ideal: (50, 50),
            min: (200, 150),
            max: (0, 0),
        },
        MosaicClient {
            index: 1,
            id: 1_u64,
            ideal: (5000, 5000),
            min: (0, 0),
            max: (300, 250),
        },
    ];
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &clients);
    let by_index = |i: usize| result.iter().find(|(idx, _)| *idx == i).unwrap().1;
    let small = by_index(0);
    assert!(
        small.width >= 200 && small.height >= 150,
        "ignored min bound"
    );
    let big = by_index(1);
    assert!(big.width <= 300 && big.height <= 250, "ignored max bound");
}

#[test]
fn mosaic_keeps_every_rect_inside_the_work_area() {
    let make = |i: usize| MosaicClient {
        index: i,
        id: (i) as u64,
        ideal: (350, 250),
        min: (0, 0),
        max: (0, 0),
    };
    let clients: Vec<_> = (0..6).map(make).collect();
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &clients);
    for (_, r) in &result {
        assert!(r.x >= WA.x && r.y >= WA.y);
        assert!(r.x + r.width <= WA.x + WA.width);
        assert!(r.y + r.height <= WA.y + WA.height);
    }
}

#[test]
fn mosaic_zero_ideal_falls_back_to_a_comfortable_default() {
    let clients = [MosaicClient {
        index: 0,
        id: 0_u64,
        ideal: (0, 0),
        min: (0, 0),
        max: (0, 0),
    }];
    let result = mosaic_arrange(WA, &MOSAIC_GAPS, &clients);
    let (_, rect) = result[0];
    assert!(rect.width > 0 && rect.height > 0);
    assert!(rect.width <= WA.width && rect.height <= WA.height);
}

#[test]
fn mosaic_reserves_the_outer_gap_on_every_edge() {
    // Regression: the compositor draws a client's border *outside*
    // `geom` (border frame = geom expanded by border_width). A client
    // packed flush against `work_area`'s own top edge has its border
    // bleed above it — into the bar's exclusive zone, where the bar
    // then paints over it and the border line never shows. `gappoh`/
    // `gappov` (the *outer* gap, distinct from the inter-window
    // `gappih`/`gappiv` every other mosaic test uses) must be reserved
    // on every edge, the same as `tile()`/`grid()`/every other layout.
    let gaps = GapConfig {
        gappih: 8,
        gappiv: 8,
        gappoh: 20,
        gappov: 15,
    };
    let make = |i: usize| MosaicClient {
        index: i,
        id: (i) as u64,
        ideal: (300, 200),
        min: (0, 0),
        max: (0, 0),
    };
    // Enough clients to fill more than one row/column so both the first
    // and the last row are exercised, not just a single centered one.
    let clients: Vec<_> = (0..6).map(make).collect();
    let result = mosaic_arrange(WA, &gaps, &clients);
    assert_eq!(result.len(), 6);
    for (idx, r) in &result {
        assert!(
            r.x >= WA.x + gaps.gappoh,
            "client {idx} rect {r:?} bleeds past the left outer gap"
        );
        assert!(
            r.y >= WA.y + gaps.gappov,
            "client {idx} rect {r:?} bleeds past the top outer gap \
             (this is what hid under the bar)"
        );
        assert!(
            r.x + r.width <= WA.x + WA.width - gaps.gappoh,
            "client {idx} rect {r:?} bleeds past the right outer gap"
        );
        assert!(
            r.y + r.height <= WA.y + WA.height - gaps.gappov,
            "client {idx} rect {r:?} bleeds past the bottom outer gap"
        );
    }
}

// ── Phase 3: mosaic_overflow_client ─────────────────────────────────────────

#[test]
fn mosaic_overflow_client_none_on_empty_input() {
    assert_eq!(mosaic_overflow_client(WA, &MOSAIC_GAPS, &[]), None);
}

#[test]
fn mosaic_stack_peek_rect_first_slot_sits_flush_in_the_corner() {
    let r = mosaic_stack_peek_rect(WA, &MOSAIC_GAPS, 0);
    assert_eq!(r.width, 72);
    assert_eq!(r.height, 72);
    assert_eq!(r.x + r.width, WA.x + WA.width - MOSAIC_GAPS.gappoh);
    assert_eq!(r.y + r.height, WA.y + WA.height - MOSAIC_GAPS.gappov);
}

#[test]
fn mosaic_stack_peek_rect_later_slots_fan_out_leftward_without_overlap() {
    let a = mosaic_stack_peek_rect(WA, &MOSAIC_GAPS, 0);
    let b = mosaic_stack_peek_rect(WA, &MOSAIC_GAPS, 1);
    let c = mosaic_stack_peek_rect(WA, &MOSAIC_GAPS, 2);
    assert_eq!(a.y, b.y, "the row of peeks stays aligned on y");
    assert_eq!(b.y, c.y);
    assert!(b.x + b.width <= a.x, "slot 1 must not overlap slot 0");
    assert!(c.x + c.width <= b.x, "slot 2 must not overlap slot 1");
}

#[test]
fn mosaic_stack_peek_rect_stays_inside_a_tiny_work_area() {
    let tiny = Rect::new(0, 0, 40, 40);
    let r = mosaic_stack_peek_rect(tiny, &MOSAIC_GAPS, 0);
    assert!(r.width <= tiny.width && r.height <= tiny.height);
    assert!(r.x >= tiny.x && r.y >= tiny.y);
}

#[test]
fn mosaic_overflow_client_none_when_everything_fits() {
    let clients = [
        MosaicClient {
            index: 0,
            id: 1,
            ideal: (300, 200),
            min: (0, 0),
            max: (0, 0),
        },
        MosaicClient {
            index: 1,
            id: 2,
            ideal: (300, 200),
            min: (0, 0),
            max: (0, 0),
        },
    ];
    assert_eq!(mosaic_overflow_client(WA, &MOSAIC_GAPS, &clients), None);
}

#[test]
fn mosaic_overflow_client_none_when_shrinking_alone_makes_it_fit() {
    // Same shape as `mosaic_shrinks_toward_min_height_when_rows_overflow_
    // vertically`: five rows' worth of 200px-tall clients into a 600px-tall
    // work area don't fit at their *ideal* height, but every one has
    // min_h: 60, and 5 * 60 = 300 (+ gaps) fits comfortably — shrinking is
    // enough, nobody needs to leave.
    let make = |i: usize| MosaicClient {
        index: i,
        id: i as u64,
        ideal: (1000, 200),
        min: (0, 60),
        max: (0, 0),
    };
    let clients: Vec<_> = (0..5).map(make).collect();
    assert_eq!(mosaic_overflow_client(WA, &MOSAIC_GAPS, &clients), None);
}

#[test]
fn mosaic_overflow_client_detects_genuine_overflow_and_picks_the_newest() {
    // 10 clients, each with an oversized `ideal` width (1000 — the whole
    // WA) that `pack_rows`'s column cap brings down to ~4 per row, so this
    // packs into 3 rows, not 10. Each row's *min* height alone (250px) is
    // still enough that 3 of them (750px + inter-row gaps) blow past WA's
    // 600px tall work area — so no amount of shrinking fixes it.
    let make = |i: usize, id: u64| MosaicClient {
        index: i,
        id,
        ideal: (1000, 200),
        min: (0, 250),
        max: (0, 0),
    };
    // Ids deliberately out of index order — the *newest* (highest id)
    // must win regardless of position in the queue.
    let clients: Vec<_> = (0..10).map(|i| make(i, 100 - i as u64)).collect();
    let evicted = mosaic_overflow_client(WA, &MOSAIC_GAPS, &clients);
    assert_eq!(evicted, Some(100), "must evict the highest id, not index 0");
}

// ── Rect::clamped_to_actual_size — border/shadow follow the client, not
// just the slot (mtune skin-switch bug: a floating window's slot is
// pinned by its window rule and doesn't shrink when the client itself
// does) ─────────────────────────────────────────────────────────────

const FLOAT_SLOT: Rect = Rect {
    x: 940,
    y: -70,
    width: 640,
    height: 940,
};

#[test]
fn a_smaller_actual_size_shrinks_both_axes() {
    // mtune's mini/strip skin inside a windowrule-pinned float_geom slot.
    let r = FLOAT_SLOT.clamped_to_actual_size(360, 96);
    assert_eq!(r.width, 360);
    assert_eq!(r.height, 96);
    // Anchored at the slot's own top-left, unchanged.
    assert_eq!(r.x, FLOAT_SLOT.x);
    assert_eq!(r.y, FLOAT_SLOT.y);
}

#[test]
fn a_larger_actual_size_never_grows_past_the_slot() {
    // A client whose buffer briefly overshoots the slot (e.g. mid-reflow)
    // is already being clipped elsewhere -- border/shadow must not grow
    // past the compositor-assigned slot to "catch up" with it.
    let r = FLOAT_SLOT.clamped_to_actual_size(2000, 2000);
    assert_eq!(r, FLOAT_SLOT);
}

#[test]
fn a_zero_actual_size_is_ignored_on_both_axes() {
    // `actual` is 0x0 before a client's first buffer commit (or while a
    // configure/ack handshake is in flight) -- must not collapse the
    // border/shadow to nothing for that gap.
    let r = FLOAT_SLOT.clamped_to_actual_size(0, 0);
    assert_eq!(r, FLOAT_SLOT);
}

#[test]
fn the_two_axes_clamp_independently() {
    // A skin that's narrower but not shorter (or vice versa) only
    // shrinks the axis that's actually smaller.
    let narrower_only = FLOAT_SLOT.clamped_to_actual_size(300, 2000);
    assert_eq!(narrower_only.width, 300);
    assert_eq!(narrower_only.height, FLOAT_SLOT.height);

    let shorter_only = FLOAT_SLOT.clamped_to_actual_size(2000, 96);
    assert_eq!(shorter_only.width, FLOAT_SLOT.width);
    assert_eq!(shorter_only.height, 96);
}

#[test]
fn an_exact_match_is_a_no_op() {
    let r = FLOAT_SLOT.clamped_to_actual_size(FLOAT_SLOT.width, FLOAT_SLOT.height);
    assert_eq!(r, FLOAT_SLOT);
}
