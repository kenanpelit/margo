//! Layout-algorithm snapshot test suite.
//!
//! W1.1 from the road map's *catch-and-surpass-niri plan*. niri's
//! defining test-quality lead is its 5,280-file `src/tests/snapshots/`
//! directory — visual / structural regressions get caught at PR time,
//! not at user-reload time.
//!
//! Margo's 14 layout algorithms are pure functions:
//!
//! ```ignore
//! fn tile(ctx: &ArrangeCtx) -> Vec<(usize, Rect)>
//! ```
//!
//! That maps perfectly onto `insta`'s string-snapshot model — feed
//! a fixture context in, format the resulting `(idx, x, y, w, h)`
//! list as a stable text grid, lock it with `assert_snapshot!`, and
//! commit the resulting `.snap` files alongside the source.  A
//! geometry regression in any layout becomes a single text diff at
//! review time.
//!
//! ## Why text snapshots, not PNG snapshots
//!
//! Two reasons:
//!
//! 1. **Reviewability.** A text snapshot diff on PR review tells the
//!    reviewer "client 2 moved from x=960 to x=900" — they can
//!    decide intent in one glance. A PNG diff requires opening the
//!    image, eyeballing pixels, deciding if the difference is
//!    intentional. niri's PNG snapshots exist for a reason
//!    (animation curves, focus-ring colors), but those aren't what
//!    catches layout-arithmetic regressions.
//! 2. **Determinism.** Layout algorithms are floating-point + integer
//!    rounding only. The output is identical across platforms,
//!    architectures, and renderer backends. PNG snapshots would
//!    drift on a different GLES driver and we'd be debugging Mesa
//!    versions instead of layout math.
//!
//! ## Adding new scenarios
//!
//! 1. Build an `ArrangeCtx` via [`Fixture::ctx`] (or extend the
//!    fixture helpers if your scenario needs a new shape).
//! 2. Call the layout function (`tile(&ctx)`, `scroller(&ctx)`, …).
//! 3. Wrap the result in [`format_arranged`] for stable text output.
//! 4. `assert_snapshot!(format_arranged(&result));` — file-based,
//!    no inline content. The bare-arg form lands the snapshot in
//!    `margo/src/layout/snapshots/<test_name>.snap` so you don't
//!    have to fight raw-string indentation.
//! 5. First run: `INSTA_UPDATE=always cargo test … layout::snapshot_tests::`
//!    writes the baseline `.snap` files. Review them with
//!    `git diff -- margo/src/layout/snapshots/` and commit. Future
//!    runs gate against the committed values.
//! 6. Subsequent intentional changes: bump the algorithm, run
//!    `cargo insta review` (after `cargo install cargo-insta`),
//!    accept the diff per-snapshot, commit.
//!
//! Naming convention: `<layout>_<window_count>_<modifier>` so the
//! generated snapshot files sort by-layout alphabetically.
//! `tile_3wins`, `scroller_5wins_focus_centered`.

#![allow(clippy::needless_range_loop)] // tests are explicit; readability wins

use super::*;

// ── Fixture: build canonical `ArrangeCtx` instances ─────────────────────────

/// Common fixture sizes — drawn from real-world hardware. Each
/// scenario picks one based on what the test is exercising.
const HD_1080P: (i32, i32) = (1920, 1080);
const QHD: (i32, i32) = (2560, 1440);
const PORTRAIT: (i32, i32) = (1200, 1920);

/// Builder that holds the bits an `ArrangeCtx` needs alive
/// (`tiled` slice, `scroller_proportions` slice). The slice fields
/// of `ArrangeCtx` borrow from these so a single fixture builds
/// many contexts back-to-back without re-allocating.
struct Fixture {
    tiled: Vec<usize>,
    scroller_proportions: Vec<f32>,
    gaps: GapConfig,
    work_area: Rect,
}

impl Fixture {
    fn with_windows(work_area_size: (i32, i32), n_windows: usize) -> Self {
        Fixture {
            tiled: (0..n_windows).collect(),
            scroller_proportions: vec![0.8; n_windows],
            gaps: GapConfig {
                gappih: 8,
                gappiv: 8,
                gappoh: 8,
                gappov: 8,
            },
            work_area: Rect::new(0, 0, work_area_size.0, work_area_size.1),
        }
    }

    fn with_proportions(mut self, props: &[f32]) -> Self {
        self.scroller_proportions = props.to_vec();
        self
    }

    fn with_gap(mut self, gap: i32) -> Self {
        self.gaps = GapConfig {
            gappih: gap,
            gappiv: gap,
            gappoh: gap,
            gappov: gap,
        };
        self
    }

    fn no_gap(mut self) -> Self {
        self.gaps = GapConfig::default();
        self
    }

    fn ctx(&self) -> ArrangeCtxBuilder<'_> {
        ArrangeCtxBuilder {
            fixture: self,
            nmaster: 1,
            mfact: 0.55,
            focused_tiled_pos: None,
            scroller_structs: 24,
            scroller_focus_center: true,
            scroller_prefer_center: true,
            scroller_prefer_overspread: false,
        }
    }
}

/// Per-call adjustments that don't affect the underlying slice
/// memory. Lets each test set just the knobs it cares about.
struct ArrangeCtxBuilder<'a> {
    fixture: &'a Fixture,
    nmaster: u32,
    mfact: f32,
    focused_tiled_pos: Option<usize>,
    scroller_structs: i32,
    scroller_focus_center: bool,
    scroller_prefer_center: bool,
    scroller_prefer_overspread: bool,
}

impl<'a> ArrangeCtxBuilder<'a> {
    fn nmaster(mut self, n: u32) -> Self {
        self.nmaster = n;
        self
    }
    fn mfact(mut self, f: f32) -> Self {
        self.mfact = f;
        self
    }
    fn focused(mut self, pos: usize) -> Self {
        self.focused_tiled_pos = Some(pos);
        self
    }
    fn scroller_focus_center(mut self, on: bool) -> Self {
        self.scroller_focus_center = on;
        self
    }
    fn build(&self) -> ArrangeCtx<'a> {
        ArrangeCtx {
            work_area: self.fixture.work_area,
            tiled: &self.fixture.tiled,
            nmaster: self.nmaster,
            mfact: self.mfact,
            gaps: &self.fixture.gaps,
            scroller_proportions: &self.fixture.scroller_proportions,
            default_scroller_proportion: 0.8,
            focused_tiled_pos: self.focused_tiled_pos,
            scroller_structs: self.scroller_structs,
            scroller_focus_center: self.scroller_focus_center,
            scroller_prefer_center: self.scroller_prefer_center,
            scroller_prefer_overspread: self.scroller_prefer_overspread,
            canvas_pan: (0.0, 0.0),
        }
    }
}

// ── Output formatter: ArrangeResult → stable text snapshot ─────────────────

/// Format a layout result as a deterministic ASCII grid:
///
/// ```text
/// 0  x=8     y=8     w=952    h=1064
/// 1  x=968   y=8     w=944    h=1064
/// ```
///
/// Sorted by index so re-orderings inside the algorithm don't show
/// up as snapshot churn (the visible output is rect-by-window, not
/// vec-position-by-window). Width-padded so column drift is
/// obvious in a diff.
fn format_arranged(arranged: &[(usize, Rect)]) -> String {
    let mut sorted: Vec<_> = arranged.iter().collect();
    sorted.sort_by_key(|(idx, _)| *idx);
    let mut lines = Vec::new();
    for (idx, rect) in sorted {
        // Trailing column gets no width-padding so each line trims
        // clean — important for `assert_snapshot!` raw-string
        // matching, which is otherwise sensitive to invisible spaces.
        let line = format!(
            "{:>2}  x={:<6} y={:<6} w={:<6} h={}",
            idx, rect.x, rect.y, rect.width, rect.height
        );
        lines.push(line);
    }
    lines.join("\n")
}

// ── Tests ──────────────────────────────────────────────────────────────────
//
// Snapshots use `assert_snapshot!(value, @"…")` with the value
// inline. First run: `@""` and run `cargo insta review` to accept
// the produced text. Subsequent runs gate on identity.
//
// Test naming: <layout>_<scenario> so cargo's listing groups by
// algorithm.

use insta::assert_snapshot;

// ── tile (master+stack) ─────────────────────────────────────────────────────

#[test]
fn tile_single_window() {
    let f = Fixture::with_windows(HD_1080P, 1);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&tile(&ctx)));
}

#[test]
fn tile_two_windows_default_mfact() {
    let f = Fixture::with_windows(HD_1080P, 2);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&tile(&ctx)));
}

#[test]
fn tile_three_windows() {
    let f = Fixture::with_windows(HD_1080P, 3);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&tile(&ctx)));
}

#[test]
fn tile_two_windows_mfact_50() {
    let f = Fixture::with_windows(HD_1080P, 2);
    let ctx = f.ctx().mfact(0.5).build();
    assert_snapshot!(format_arranged(&tile(&ctx)));
}

#[test]
fn tile_no_gap() {
    let f = Fixture::with_windows(HD_1080P, 2).no_gap();
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&tile(&ctx)));
}

// ── monocle ────────────────────────────────────────────────────────────────

#[test]
fn monocle_single_window() {
    let f = Fixture::with_windows(HD_1080P, 1);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&monocle(&ctx)));
}

#[test]
fn monocle_three_windows_all_same_rect() {
    // Monocle should give every window the full work area — only
    // one is visible at a time but all share geometry.
    let f = Fixture::with_windows(HD_1080P, 3);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&monocle(&ctx)));
}

// ── grid ───────────────────────────────────────────────────────────────────

#[test]
fn grid_four_windows_2x2() {
    let f = Fixture::with_windows(HD_1080P, 4);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&grid(&ctx)));
}

#[test]
fn grid_three_windows_2_then_1() {
    let f = Fixture::with_windows(HD_1080P, 3);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&grid(&ctx)));
}

// ── deck (master + stack-of-tabs) ──────────────────────────────────────────

#[test]
fn deck_two_windows() {
    let f = Fixture::with_windows(HD_1080P, 2);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&deck(&ctx)));
}

// ── center_tile ────────────────────────────────────────────────────────────

#[test]
fn center_tile_three_windows() {
    let f = Fixture::with_windows(HD_1080P, 3);
    let ctx = f.ctx().build();
    let arranged = center_tile(&ctx);
    // Master should be horizontally centered; stack splits left / right.
    assert_snapshot!(format_arranged(&arranged));
}

// ── scroller (PaperWM-style) ───────────────────────────────────────────────

#[test]
fn scroller_three_windows_focus_first() {
    let f = Fixture::with_windows(HD_1080P, 3);
    let ctx = f.ctx().focused(0).build();
    assert_snapshot!(format_arranged(&scroller(&ctx)));
}

#[test]
fn scroller_three_windows_focus_centered() {
    let f = Fixture::with_windows(HD_1080P, 3);
    let ctx = f.ctx().focused(1).scroller_focus_center(true).build();
    let arranged = scroller(&ctx);
    // Focused (idx 1) should be horizontally centered. Verify the
    // mid-point of its rect lands within ±2 px of the work-area
    // mid-point — exact pixel position depends on rounding.
    let focused_rect = arranged.iter().find(|(i, _)| *i == 1).unwrap().1;
    let focused_mid = focused_rect.x + focused_rect.width / 2;
    let target_mid = ctx.work_area.x + ctx.work_area.width / 2;
    assert!(
        (focused_mid - target_mid).abs() <= 2,
        "scroller focus-centered: focused mid {focused_mid} vs target {target_mid}"
    );
    // And the snapshot for the full layout — locks the geometry.
    assert_snapshot!(format_arranged(&arranged));
}

#[test]
fn scroller_focus_center_off_anchors_to_left() {
    let f = Fixture::with_windows(HD_1080P, 3);
    let ctx = f.ctx().focused(2).scroller_focus_center(false).build();
    assert_snapshot!(format_arranged(&scroller(&ctx)));
}

// ── vertical_scroller ──────────────────────────────────────────────────────

#[test]
fn vertical_scroller_three_windows_focus_centered() {
    let f = Fixture::with_windows(PORTRAIT, 3);
    let ctx = f.ctx().focused(1).scroller_focus_center(true).build();
    let arranged = vertical_scroller(&ctx);
    let focused_rect = arranged.iter().find(|(i, _)| *i == 1).unwrap().1;
    let focused_mid = focused_rect.y + focused_rect.height / 2;
    let target_mid = ctx.work_area.y + ctx.work_area.height / 2;
    assert!(
        (focused_mid - target_mid).abs() <= 2,
        "vertical scroller focus-centered: mid {focused_mid} vs target {target_mid}"
    );
}

// ── vertical_tile / vertical_grid / vertical_deck ──────────────────────────

#[test]
fn vertical_tile_two_windows() {
    let f = Fixture::with_windows(PORTRAIT, 2);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&vertical_tile(&ctx)));
}

#[test]
fn vertical_grid_four_windows() {
    let f = Fixture::with_windows(PORTRAIT, 4);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&vertical_grid(&ctx)));
}

#[test]
fn vertical_deck_three_windows() {
    let f = Fixture::with_windows(PORTRAIT, 3);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&vertical_deck(&ctx)));
}

// ── right_tile ─────────────────────────────────────────────────────────────

#[test]
fn right_tile_two_windows() {
    let f = Fixture::with_windows(HD_1080P, 2);
    let ctx = f.ctx().build();
    let arranged = right_tile(&ctx);
    // Master (idx 0) should be on the RIGHT; stack on the LEFT.
    let master = arranged.iter().find(|(i, _)| *i == 0).unwrap().1;
    let stack0 = arranged.iter().find(|(i, _)| *i == 1).unwrap().1;
    assert!(master.x > stack0.x, "right_tile: master should be right of stack");
    assert_snapshot!(format_arranged(&arranged));
}

// ── tgmix (tile + grid hybrid) ─────────────────────────────────────────────

#[test]
fn tgmix_five_windows() {
    let f = Fixture::with_windows(HD_1080P, 5);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&tgmix(&ctx)));
}

// ── dwindle (recursive split) ──────────────────────────────────────────────

#[test]
fn dwindle_four_windows() {
    let f = Fixture::with_windows(HD_1080P, 4);
    let ctx = f.ctx().build();
    assert_snapshot!(format_arranged(&dwindle(&ctx)));
}

// ── canvas (free-form, panless) ────────────────────────────────────────────

#[test]
fn canvas_does_not_arrange() {
    // canvas is a no-op layout — clients keep their canvas_geom.
    // The function returns an empty result; arrange() short-circuits
    // and the live render path consults `client.canvas_geom` direct.
    let f = Fixture::with_windows(HD_1080P, 3);
    let ctx = f.ctx().build();
    let arranged = canvas(&ctx);
    assert!(arranged.is_empty(), "canvas should return empty (no auto-arrange)");
}

// ── Cross-cutting: every layout stays inside work area for SDR cases ──────
//
// Property test: across `tile`, `monocle`, `grid`, `deck`,
// `center_tile`, `right_tile`, `vertical_tile`, `vertical_grid`,
// `vertical_deck`, `tgmix`, `dwindle` — every output rect should
// overlap the work area for any reasonable window count. (Scroller
// variants intentionally exceed work_area for off-screen clients;
// they're excluded.)

#[test]
fn non_scroller_layouts_stay_inside_work_area() {
    use LayoutId::*;
    let layouts = [
        Tile,
        Monocle,
        Grid,
        Deck,
        CenterTile,
        RightTile,
        VerticalTile,
        VerticalGrid,
        VerticalDeck,
        TgMix,
        Dwindle,
    ];
    for &layout in &layouts {
        for n in 1..=6 {
            let f = Fixture::with_windows(HD_1080P, n);
            let ctx = f.ctx().build();
            let arranged = arrange(layout, &ctx);
            for (idx, rect) in &arranged {
                // Allow rect to touch the work-area edge (right/bottom
                // can land at work_area + width/height); just check
                // it doesn't go negative or escape further than the
                // gap config permits.
                assert!(
                    rect.x >= 0 && rect.y >= 0,
                    "{layout:?} n={n} idx={idx}: rect at ({}, {}) escaped the work area top-left",
                    rect.x,
                    rect.y,
                );
                assert!(
                    rect.width > 0 && rect.height > 0,
                    "{layout:?} n={n} idx={idx}: zero-or-negative size {}x{}",
                    rect.width,
                    rect.height,
                );
            }
        }
    }
}

// ── Property: `arrange()` dispatcher returns same as direct call ──────────

#[test]
fn arrange_dispatcher_matches_direct_call() {
    let f = Fixture::with_windows(HD_1080P, 4);
    let ctx = f.ctx().build();
    assert_eq!(arrange(LayoutId::Tile, &ctx), tile(&ctx));
    assert_eq!(arrange(LayoutId::Monocle, &ctx), monocle(&ctx));
    assert_eq!(arrange(LayoutId::Grid, &ctx), grid(&ctx));
    assert_eq!(arrange(LayoutId::Deck, &ctx), deck(&ctx));
    assert_eq!(arrange(LayoutId::CenterTile, &ctx), center_tile(&ctx));
    assert_eq!(arrange(LayoutId::Scroller, &ctx), scroller(&ctx));
    assert_eq!(arrange(LayoutId::Dwindle, &ctx), dwindle(&ctx));
}

// ── W1.2: extended property tests across the full layout catalogue ─────────
//
// Property tests for the full 14-layout catalogue × {1, 2, 3, 5} window
// counts × focus shift. Each `LayoutId` variant should satisfy the
// invariants below. Canvas is the panless free-form layout — it
// returns an empty vec by design and is excluded from cardinality /
// rect-validity properties (the live render path consults
// `client.canvas_geom` directly).

/// Every `LayoutId` variant except Canvas (no-op by design).
const ALL_LAYOUTS_EXCEPT_CANVAS: &[LayoutId] = &[
    LayoutId::Tile,
    LayoutId::Scroller,
    LayoutId::Grid,
    LayoutId::Monocle,
    LayoutId::Deck,
    LayoutId::CenterTile,
    LayoutId::RightTile,
    LayoutId::VerticalScroller,
    LayoutId::VerticalTile,
    LayoutId::VerticalGrid,
    LayoutId::VerticalDeck,
    LayoutId::TgMix,
    LayoutId::Dwindle,
    LayoutId::Overview,
];

/// Layouts whose master/stack rects should not overlap one another.
/// Excludes monocle/deck (intentional overlap), scroller variants
/// (off-screen by design), canvas (empty), overview (== monocle).
const TILE_CLASS_LAYOUTS: &[LayoutId] = &[
    LayoutId::Tile,
    LayoutId::RightTile,
    LayoutId::Grid,
    LayoutId::CenterTile,
    LayoutId::VerticalTile,
    LayoutId::VerticalGrid,
    LayoutId::TgMix,
    LayoutId::Dwindle,
];

#[test]
fn arrange_dispatcher_matches_direct_call_all_layouts() {
    // Property test: the `arrange()` dispatcher must agree with the
    // direct function call for *every* `LayoutId` variant. The narrow
    // version of this test (above) only spot-checks 7; this one
    // covers all 15 (Canvas + Overview included).
    let f = Fixture::with_windows(HD_1080P, 4);
    let ctx = f.ctx().build();
    for &layout in ALL_LAYOUTS_EXCEPT_CANVAS {
        let direct: ArrangeResult = match layout {
            LayoutId::Tile => tile(&ctx),
            LayoutId::Scroller => scroller(&ctx),
            LayoutId::Grid => grid(&ctx),
            LayoutId::Monocle => monocle(&ctx),
            LayoutId::Deck => deck(&ctx),
            LayoutId::CenterTile => center_tile(&ctx),
            LayoutId::RightTile => right_tile(&ctx),
            LayoutId::VerticalScroller => vertical_scroller(&ctx),
            LayoutId::VerticalTile => vertical_tile(&ctx),
            LayoutId::VerticalGrid => vertical_grid(&ctx),
            LayoutId::VerticalDeck => vertical_deck(&ctx),
            LayoutId::TgMix => tgmix(&ctx),
            LayoutId::Dwindle => dwindle(&ctx),
            LayoutId::Overview => monocle(&ctx),
            LayoutId::Canvas => unreachable!("Canvas is filtered out of the test loop earlier"),
        };
        assert_eq!(
            arrange(layout, &ctx),
            direct,
            "arrange({layout:?}) diverged from direct call",
        );
    }
    // Canvas is a no-op separately.
    assert!(arrange(LayoutId::Canvas, &ctx).is_empty());
}

#[test]
fn cardinality_matches_input_for_non_canvas_layouts() {
    // Every non-canvas layout returns exactly one rect per input
    // client. Empty input → empty output. Caught a real regression
    // when an early `dwindle` impl dropped the last leaf for n>=8.
    for &layout in ALL_LAYOUTS_EXCEPT_CANVAS {
        for n in [0, 1, 2, 3, 5, 8] {
            let f = Fixture::with_windows(HD_1080P, n);
            let ctx = f.ctx().build();
            let arranged = arrange(layout, &ctx);
            assert_eq!(
                arranged.len(),
                n,
                "{layout:?} n={n}: returned {} rects, expected {n}",
                arranged.len(),
            );
        }
    }
}

#[test]
fn no_degenerate_rects_across_full_catalogue() {
    // Every output rect must have positive width and height. A
    // zero-or-negative size means a layout silently dropped a client
    // off-screen — invisible bug at runtime, easy to catch here.
    // Scroller variants intentionally exceed work_area off-screen,
    // so we don't constrain x/y here — just the size invariant.
    for &layout in ALL_LAYOUTS_EXCEPT_CANVAS {
        for n in 1..=6 {
            let f = Fixture::with_windows(HD_1080P, n);
            let ctx = f.ctx().build();
            for (idx, rect) in arrange(layout, &ctx) {
                assert!(
                    rect.width > 0 && rect.height > 0,
                    "{layout:?} n={n} idx={idx}: degenerate rect {}x{}",
                    rect.width,
                    rect.height,
                );
            }
        }
    }
}

#[test]
fn monocle_returns_identical_rect_for_every_client() {
    // Property: monocle is a single-window-visible layout — every
    // client gets the SAME rect. Adding gaps / changing nmaster
    // shouldn't fan-out the geometry.
    for n in 1..=6 {
        let f = Fixture::with_windows(HD_1080P, n);
        let ctx = f.ctx().build();
        let arranged = monocle(&ctx);
        if let Some((_, first)) = arranged.first().copied() {
            for (idx, rect) in &arranged {
                assert_eq!(
                    *rect, first,
                    "monocle n={n} idx={idx}: rect diverged from monocle invariant",
                );
            }
        }
    }
}

#[test]
fn deck_stack_clients_share_one_rect() {
    // Property: deck's non-master clients all get the same stack
    // rect (that's the "tab stack" semantic). Master rects can
    // differ when nmaster > 1, so we slice off the masters and
    // verify the rest collapse to one rect.
    for nmaster in 1..=2 {
        for n in (nmaster + 1)..=5 {
            let f = Fixture::with_windows(HD_1080P, n);
            let ctx = f.ctx().nmaster(nmaster as u32).build();
            let arranged = deck(&ctx);
            let stack: Vec<_> = arranged
                .iter()
                .skip(nmaster)
                .map(|(_, r)| *r)
                .collect();
            let first = stack[0];
            for (i, rect) in stack.iter().enumerate() {
                assert_eq!(
                    *rect,
                    first,
                    "deck n={n} nmaster={nmaster} stack[{i}] diverged from shared stack rect",
                );
            }
        }
    }
}

#[test]
fn tile_class_layouts_have_pairwise_disjoint_rects() {
    // Property: in tiling layouts (no master/stack overlap by
    // design), no two output rects share interior pixels. Allow a
    // single-pixel touch for rounding seams. Excludes monocle / deck
    // (overlap is the point) and scroller variants (off-screen by
    // design).
    for &layout in TILE_CLASS_LAYOUTS {
        for n in 2..=5 {
            let f = Fixture::with_windows(HD_1080P, n);
            let ctx = f.ctx().build();
            let arranged = arrange(layout, &ctx);
            for i in 0..arranged.len() {
                for j in (i + 1)..arranged.len() {
                    let a = arranged[i].1;
                    let b = arranged[j].1;
                    let overlap_w = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
                    let overlap_h = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
                    let overlaps = overlap_w > 0 && overlap_h > 0;
                    assert!(
                        !overlaps,
                        "{layout:?} n={n}: rects {i}={a:?} and {j}={b:?} overlap by {overlap_w}x{overlap_h}",
                    );
                }
            }
        }
    }
}

#[test]
fn focus_position_does_not_shift_non_scroller_layouts() {
    // Property: focus tracking is a scroller-only concern. Tile,
    // grid, deck, monocle, etc. should produce IDENTICAL geometry
    // regardless of `focused_tiled_pos` — moving focus among already-
    // mapped clients should never trigger a re-arrange jitter.
    let focus_invariant_layouts = [
        LayoutId::Tile,
        LayoutId::RightTile,
        LayoutId::Grid,
        LayoutId::Monocle,
        LayoutId::Deck,
        LayoutId::CenterTile,
        LayoutId::VerticalTile,
        LayoutId::VerticalGrid,
        LayoutId::VerticalDeck,
        LayoutId::TgMix,
        LayoutId::Dwindle,
        LayoutId::Overview,
    ];
    for &layout in &focus_invariant_layouts {
        let f = Fixture::with_windows(HD_1080P, 4);
        let baseline = arrange(layout, &f.ctx().build());
        for focus in 0..4 {
            let with_focus = arrange(layout, &f.ctx().focused(focus).build());
            assert_eq!(
                baseline, with_focus,
                "{layout:?}: focus={focus} changed geometry — expected focus-invariant",
            );
        }
    }
}

#[test]
fn overview_aliases_monocle() {
    // Overview is dispatched through the overview UI elsewhere; the
    // arrange dispatcher routes it to monocle so the underlying
    // tag still has a sensible base layout when overview ends.
    for n in [1, 3, 5] {
        let f = Fixture::with_windows(HD_1080P, n);
        let ctx = f.ctx().build();
        assert_eq!(
            arrange(LayoutId::Overview, &ctx),
            monocle(&ctx),
            "Overview should route to monocle (n={n})",
        );
    }
}

#[test]
fn empty_input_yields_empty_output_for_every_layout() {
    // Edge case: zero-window tag. Every layout (canvas included)
    // must return an empty vec — no panics, no synthetic rects.
    let f = Fixture::with_windows(HD_1080P, 0);
    let ctx = f.ctx().build();
    for &layout in ALL_LAYOUTS_EXCEPT_CANVAS {
        assert!(
            arrange(layout, &ctx).is_empty(),
            "{layout:?}: empty input should produce empty output",
        );
    }
    assert!(arrange(LayoutId::Canvas, &ctx).is_empty());
}

#[test]
fn right_tile_master_strictly_right_of_stack() {
    // Property: right_tile is the mirror of tile — master is the
    // RIGHTMOST rect, stack columns are to its left. This held in
    // the snapshot test for n=2; extend across n in 2..=5.
    for n in 2..=5 {
        let f = Fixture::with_windows(HD_1080P, n);
        let ctx = f.ctx().build();
        let arranged = right_tile(&ctx);
        let master = arranged.iter().find(|(i, _)| *i == 0).unwrap().1;
        for (idx, rect) in &arranged {
            if *idx == 0 {
                continue;
            }
            assert!(
                master.x >= rect.x + rect.width - 1, // allow 1 px gap-rounding
                "right_tile n={n} stack idx={idx}: master.x={} not strictly right of stack rect right-edge {}",
                master.x,
                rect.x + rect.width,
            );
        }
    }
}

#[test]
fn vertical_tile_master_top_half_for_portrait_fixture() {
    // Property: `vertical_tile` arranges top-master / bottom-stack on
    // a portrait monitor. Master should lie in the top half.
    for n in 2..=5 {
        let f = Fixture::with_windows(PORTRAIT, n);
        let ctx = f.ctx().build();
        let arranged = vertical_tile(&ctx);
        let master = arranged.iter().find(|(i, _)| *i == 0).unwrap().1;
        let half_h = ctx.work_area.y + ctx.work_area.height / 2;
        assert!(
            master.y + master.height / 2 <= half_h + 1,
            "vertical_tile n={n}: master center y={} should sit in top half (≤ {half_h})",
            master.y + master.height / 2,
        );
    }
}

#[test]
fn scroller_total_width_grows_with_window_count() {
    // Property: scroller is unbounded — adding clients widens the
    // sum-of-widths past the work area. (This is the off-screen
    // semantic that excludes scroller from `non_scroller_layouts_*`.)
    let work_w = HD_1080P.0;
    let mut last_total: i32 = 0;
    for n in 2..=6 {
        let f = Fixture::with_windows(HD_1080P, n);
        let ctx = f.ctx().focused(0).build();
        let arranged = scroller(&ctx);
        let total: i32 = arranged.iter().map(|(_, r)| r.width).sum();
        assert!(
            total >= last_total,
            "scroller n={n}: total width {total} shrank from previous {last_total}",
        );
        if n >= 3 {
            assert!(
                total >= work_w,
                "scroller n={n}: total width {total} should exceed work-area width {work_w}",
            );
        }
        last_total = total;
    }
}

#[test]
fn gap_zero_makes_layouts_use_full_work_area() {
    // Property: with all gaps zero, the bounding box of a tile-class
    // layout should cover the *entire* work area (within rounding).
    // Catches "gappoh hardcoded somewhere" regressions.
    for &layout in TILE_CLASS_LAYOUTS {
        let f = Fixture::with_windows(HD_1080P, 4).no_gap();
        let ctx = f.ctx().build();
        let arranged = arrange(layout, &ctx);
        if arranged.is_empty() {
            continue;
        }
        let min_x = arranged.iter().map(|(_, r)| r.x).min().unwrap();
        let min_y = arranged.iter().map(|(_, r)| r.y).min().unwrap();
        let max_x = arranged.iter().map(|(_, r)| r.x + r.width).max().unwrap();
        let max_y = arranged.iter().map(|(_, r)| r.y + r.height).max().unwrap();
        let wa = ctx.work_area;
        // Allow ±2 px slop for rounding seams between rects.
        assert!(
            (min_x - wa.x).abs() <= 2 && (min_y - wa.y).abs() <= 2,
            "{layout:?}: top-left ({min_x},{min_y}) drifted from work-area ({},{})",
            wa.x,
            wa.y,
        );
        assert!(
            ((wa.x + wa.width) - max_x).abs() <= 2
                && ((wa.y + wa.height) - max_y).abs() <= 2,
            "{layout:?}: bottom-right ({max_x},{max_y}) drifted from work-area ({},{})",
            wa.x + wa.width,
            wa.y + wa.height,
        );
    }
}

#[test]
fn scroller_focus_centering_holds_for_every_focused_index() {
    // Stronger version of the existing single-index scroller centering
    // test: with `scroller_focus_center = true`, ANY focused index
    // should sit centered within ±2 px of work-area mid.
    let n = 5;
    let f = Fixture::with_windows(HD_1080P, n);
    for focus in 0..n {
        let ctx = f.ctx().focused(focus).scroller_focus_center(true).build();
        let arranged = scroller(&ctx);
        let focused_rect = arranged.iter().find(|(i, _)| *i == focus).unwrap().1;
        let focused_mid = focused_rect.x + focused_rect.width / 2;
        let target_mid = ctx.work_area.x + ctx.work_area.width / 2;
        assert!(
            (focused_mid - target_mid).abs() <= 2,
            "scroller focus={focus}: mid {focused_mid} vs target {target_mid}",
        );
    }
}
