//! Microbenchmarks for the tiling-arrange algorithms.
//!
//! These run on every layout recompute (window open/close, focus, gap or
//! mfact change), so they're the compositor's hot pure-CPU path. Two
//! groups:
//!
//! * `arrange-12-windows` — a realistic loaded monitor across the
//!   representative layout families, as a regression shield.
//! * `arrange-scaling` — every tileable algorithm at 1 / 16 / 64 windows,
//!   the CPU-side half of the roadmap's "render-scene at 1/16/64 clients"
//!   scaling story (the GPU half needs a live backend and lives in
//!   `mctl perf` instead). 64 is deliberately past any sane daily count so
//!   super-linear blowups surface here, not on a user's monitor.
//!
//! Run with `cargo bench -p margo-layouts --features bench`.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use margo_layouts::{ArrangeCtx, GapConfig, LayoutId, Rect, arrange, deck, grid, scroller, tile};

fn make_ctx<'a>(tiled: &'a [usize], proportions: &'a [f32], gaps: &'a GapConfig) -> ArrangeCtx<'a> {
    ArrangeCtx {
        work_area: Rect::new(0, 0, 3840, 2160),
        tiled,
        nmaster: 1,
        mfact: 0.55,
        gaps,
        scroller_proportions: proportions,
        default_scroller_proportion: 0.5,
        focused_tiled_pos: Some(0),
        scroller_structs: 0,
        scroller_focus_center: false,
        scroller_prefer_center: false,
        scroller_prefer_overspread: false,
        canvas_pan: (0.0, 0.0),
    }
}

fn bench_arrange(c: &mut Criterion) {
    let gaps = GapConfig {
        gappih: 8,
        gappiv: 8,
        gappoh: 12,
        gappov: 12,
    };
    let tiled: Vec<usize> = (0..12).collect();
    let proportions = vec![0.5f32; tiled.len()];
    let ctx = make_ctx(&tiled, &proportions, &gaps);

    let mut group = c.benchmark_group("arrange-12-windows");
    group.bench_function("tile", |b| b.iter(|| tile(black_box(&ctx))));
    group.bench_function("grid", |b| b.iter(|| grid(black_box(&ctx))));
    group.bench_function("deck", |b| b.iter(|| deck(black_box(&ctx))));
    group.bench_function("scroller", |b| b.iter(|| scroller(black_box(&ctx))));
    group.finish();
}

fn bench_arrange_scaling(c: &mut Criterion) {
    let gaps = GapConfig {
        gappih: 8,
        gappiv: 8,
        gappoh: 12,
        gappov: 12,
    };

    let mut group = c.benchmark_group("arrange-scaling");
    for &n in &[1usize, 16, 64] {
        let tiled: Vec<usize> = (0..n).collect();
        let proportions = vec![0.5f32; n];
        let ctx = make_ctx(&tiled, &proportions, &gaps);
        for &layout in LayoutId::all_tileable() {
            group.bench_with_input(BenchmarkId::new(layout.name(), n), &ctx, |b, ctx| {
                b.iter(|| arrange(black_box(layout), black_box(ctx)))
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_arrange, bench_arrange_scaling);
criterion_main!(benches);
