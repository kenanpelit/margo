//! Property-based fuzzing of the arrange algebra (niri-style).
//!
//! Randomised inputs across the whole catalogue, checking the
//! invariants no example-based test can pin down exhaustively:
//!
//! 1. cardinality — one rect per tiled client, always (Canvas excepted:
//!    it arranges via the render-time `canvas_geom` path and returns
//!    nothing by design);
//! 2. no degenerate rects — every emitted rect has positive size;
//! 3. containment — non-scroller layouts stay inside the work area;
//! 4. no overlap — strict tile-class layouts never stack two clients
//!    on the same pixel;
//! 5. monocle/overview — every client gets the identical rect;
//! 6. determinism — same ctx, same output.
//!
//! Input ranges are constrained to sane daily-driver values (≥800×600
//! work area, gaps ≤24, nmaster ≤3) so the properties assert real
//! guarantees instead of chasing pathological-config arithmetic; the
//! compositor's config validator keeps live values in these ranges.
//!
//! Case count: 256 by default (fast enough for `just check`); crank
//! with `PROPTEST_CASES=100000 cargo test -p margo-layouts` for a
//! deep soak (the niri CI pattern).

use margo_layouts::{ArrangeCtx, GapConfig, LayoutId, Rect, arrange};
use proptest::prelude::*;

/// Every layout the dispatcher knows about except Canvas (no-op).
const ALL_EXCEPT_CANVAS: &[LayoutId] = &[
    LayoutId::Tile,
    LayoutId::Scroller,
    LayoutId::Grid,
    LayoutId::Monocle,
    LayoutId::Deck,
    LayoutId::CenterTile,
    LayoutId::RightTile,
    LayoutId::TgMix,
    LayoutId::Dwindle,
    LayoutId::Overview,
];

/// Layouts that must keep every client inside the work area
/// (scrollers overspill by design, Canvas returns nothing).
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

/// Layouts where two clients must never share a pixel
/// (monocle/deck/overview stack clients by design).
const NON_OVERLAPPING: &[LayoutId] = &[
    LayoutId::Tile,
    LayoutId::RightTile,
    LayoutId::Grid,
    LayoutId::CenterTile,
    LayoutId::TgMix,
    LayoutId::Dwindle,
];

#[derive(Debug, Clone)]
struct Params {
    n: usize,
    nmaster: u32,
    mfact: f32,
    gaps: GapConfig,
    wa: Rect,
    focused: Option<usize>,
    proportions: Vec<f32>,
    focus_center: bool,
    prefer_center: bool,
    prefer_overspread: bool,
    structs: i32,
}

fn params() -> impl Strategy<Value = Params> {
    (
        0usize..=8,
        1u32..=3,
        0.10f32..=0.90,
        (0i32..=24, 0i32..=24, 0i32..=24, 0i32..=24),
        (0i32..=3000, 0i32..=2000, 800i32..=5120, 600i32..=2880),
        (
            proptest::collection::vec(0.1f32..=1.0, 8),
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
            0i32..=48,
            proptest::option::of(0usize..8),
        ),
    )
        .prop_map(
            |(
                n,
                nmaster,
                mfact,
                (ih, iv, oh, ov),
                (x, y, w, h),
                (proportions, focus_center, prefer_center, prefer_overspread, structs, focused),
            )| {
                Params {
                    n,
                    nmaster,
                    mfact,
                    gaps: GapConfig {
                        gappih: ih,
                        gappiv: iv,
                        gappoh: oh,
                        gappov: ov,
                    },
                    wa: Rect::new(x, y, w, h),
                    focused: focused.filter(|f| *f < n.max(1)).filter(|_| n > 0),
                    proportions,
                    focus_center,
                    prefer_center,
                    prefer_overspread,
                    structs,
                }
            },
        )
}

fn ctx_of<'a>(p: &'a Params, tiled: &'a [usize]) -> ArrangeCtx<'a> {
    ArrangeCtx {
        work_area: p.wa,
        tiled,
        nmaster: p.nmaster,
        mfact: p.mfact,
        gaps: &p.gaps,
        scroller_proportions: &p.proportions[..p.n],
        default_scroller_proportion: 0.8,
        focused_tiled_pos: p.focused,
        scroller_structs: p.structs,
        scroller_focus_center: p.focus_center,
        scroller_prefer_center: p.prefer_center,
        scroller_prefer_overspread: p.prefer_overspread,
        canvas_pan: (0.0, 0.0),
    }
}

fn overlap_area(a: &Rect, b: &Rect) -> i64 {
    let ox = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let oy = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    if ox > 0 && oy > 0 {
        ox as i64 * oy as i64
    } else {
        0
    }
}

proptest! {
    #[test]
    fn cardinality_and_positive_sizes(p in params()) {
        let tiled: Vec<usize> = (0..p.n).collect();
        for &layout in ALL_EXCEPT_CANVAS {
            let ctx = ctx_of(&p, &tiled);
            let arranged = arrange(layout, &ctx);
            prop_assert_eq!(
                arranged.len(), p.n,
                "{:?}: {} rects for {} clients ({:?})", layout, arranged.len(), p.n, p
            );
            for (idx, rect) in &arranged {
                prop_assert!(
                    rect.width > 0 && rect.height > 0,
                    "{:?} idx={} degenerate rect {}x{} ({:?})",
                    layout, idx, rect.width, rect.height, p
                );
            }
        }
        // Canvas is a render-time layout: always empty here.
        let ctx = ctx_of(&p, &tiled);
        prop_assert!(arrange(LayoutId::Canvas, &ctx).is_empty());
    }

    #[test]
    fn contained_layouts_stay_inside_work_area(p in params()) {
        let tiled: Vec<usize> = (0..p.n).collect();
        for &layout in CONTAINED {
            let ctx = ctx_of(&p, &tiled);
            for (idx, rect) in arrange(layout, &ctx) {
                prop_assert!(
                    rect.x >= p.wa.x
                        && rect.y >= p.wa.y
                        && rect.x + rect.width <= p.wa.x + p.wa.width
                        && rect.y + rect.height <= p.wa.y + p.wa.height,
                    "{:?} idx={} rect ({},{} {}x{}) escaped work area ({},{} {}x{}) ({:?})",
                    layout, idx, rect.x, rect.y, rect.width, rect.height,
                    p.wa.x, p.wa.y, p.wa.width, p.wa.height, p
                );
            }
        }
    }

    #[test]
    fn tile_class_layouts_never_overlap(p in params()) {
        let tiled: Vec<usize> = (0..p.n).collect();
        for &layout in NON_OVERLAPPING {
            let ctx = ctx_of(&p, &tiled);
            let arranged = arrange(layout, &ctx);
            for i in 0..arranged.len() {
                for j in (i + 1)..arranged.len() {
                    let area = overlap_area(&arranged[i].1, &arranged[j].1);
                    prop_assert!(
                        area == 0,
                        "{:?}: clients {} and {} overlap by {}px² ({:?})",
                        layout, arranged[i].0, arranged[j].0, area, p
                    );
                }
            }
        }
    }

    #[test]
    fn monocle_and_overview_give_every_client_the_same_rect(p in params()) {
        let tiled: Vec<usize> = (0..p.n).collect();
        for layout in [LayoutId::Monocle, LayoutId::Overview] {
            let ctx = ctx_of(&p, &tiled);
            let arranged = arrange(layout, &ctx);
            if let Some((_, first)) = arranged.first().copied() {
                for (idx, rect) in &arranged {
                    prop_assert_eq!(
                        *rect, first,
                        "{:?} idx={} diverged from the single-window rect", layout, idx
                    );
                }
            }
        }
    }

    #[test]
    fn arrange_is_deterministic(p in params()) {
        let tiled: Vec<usize> = (0..p.n).collect();
        for &layout in ALL_EXCEPT_CANVAS {
            let ctx = ctx_of(&p, &tiled);
            let a = arrange(layout, &ctx);
            let b = arrange(layout, &ctx);
            prop_assert_eq!(a, b, "{:?} is not deterministic", layout);
        }
    }
}
