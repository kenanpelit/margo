use crate::{ArrangeCtx, ArrangeResult, Rect};

// ── Tile (master-stack, horizontal split) ────────────────────────────────────

pub fn tile(ctx: &ArrangeCtx) -> ArrangeResult {
    let n = ctx.tiled.len();
    if n == 0 {
        return vec![];
    }

    let wa = &ctx.work_area;
    let nm = ctx.nmaster as usize;
    let g = ctx.gaps;

    let oh = g.gappoh;
    let ov = g.gappov;
    // `ih` (gappih) is the *horizontal* inner gap — between the master
    // column and the stack column. `iv` (gappiv) is the *vertical* inner
    // gap — between windows stacked within a column. The old code used
    // `ih` for both, so an asymmetric gappih≠gappiv tiled wrong.
    let ih = g.gappih;
    let iv = g.gappiv;

    let mut result = Vec::with_capacity(n);

    let mfact = ctx.mfact;
    let master_count = nm.min(n);
    let stack_count = n.saturating_sub(master_count);

    let total_w = wa.width - 2 * oh;
    let total_h = wa.height - 2 * ov;

    let master_w = if stack_count > 0 {
        (total_w as f32 * mfact) as i32
    } else {
        total_w
    };
    let stack_w = total_w - master_w - if stack_count > 0 { ih } else { 0 };

    for (i, &idx) in ctx.tiled.iter().enumerate() {
        let rect = if i < master_count {
            let h = (total_h - (master_count - 1) as i32 * iv) / master_count as i32;
            let y = wa.y + ov + i as i32 * (h + iv);
            Rect::new(wa.x + oh, y, master_w, h)
        } else {
            let si = i - master_count;
            let h = (total_h - (stack_count - 1) as i32 * iv) / stack_count as i32;
            let y = wa.y + ov + si as i32 * (h + iv);
            Rect::new(wa.x + oh + master_w + ih, y, stack_w, h)
        };
        result.push((idx, rect));
    }
    result
}

// ── Right tile (stack left, master right) ────────────────────────────────────

pub fn right_tile(ctx: &ArrangeCtx) -> ArrangeResult {
    let n = ctx.tiled.len();
    if n == 0 {
        return vec![];
    }

    let wa = &ctx.work_area;
    let nm = ctx.nmaster as usize;
    let g = ctx.gaps;

    let oh = g.gappoh;
    let ov = g.gappov;
    // `ih` (gappih) is the *horizontal* inner gap between the stack and
    // master columns; `iv` (gappiv) is the *vertical* inner gap between
    // windows stacked within a column. This mirrors the fix made in
    // `tile()` — the old code used `ih` for the vertical stacking too, so
    // an asymmetric gappih≠gappiv tiled wrong (and gappiv had no effect).
    let ih = g.gappih;
    let iv = g.gappiv;

    let master_count = nm.min(n);
    let stack_count = n.saturating_sub(master_count);

    let total_w = wa.width - 2 * oh;
    let total_h = wa.height - 2 * ov;

    let master_w = if stack_count > 0 {
        (total_w as f32 * ctx.mfact) as i32
    } else {
        total_w
    };
    let stack_w = total_w - master_w - if stack_count > 0 { ih } else { 0 };

    let mut result = Vec::with_capacity(n);
    for (i, &idx) in ctx.tiled.iter().enumerate() {
        let rect = if i < master_count {
            let h = (total_h - (master_count - 1) as i32 * iv) / master_count as i32;
            let y = wa.y + ov + i as i32 * (h + iv);
            // No stack column → no inner gap to clear (caught by the
            // proptest containment invariant: a lone master drifted
            // `gappih` px past the work-area edge).
            let x = wa.x + oh + stack_w + if stack_count > 0 { ih } else { 0 };
            Rect::new(x, y, master_w, h)
        } else {
            let si = i - master_count;
            let h = (total_h - (stack_count - 1) as i32 * iv) / stack_count as i32;
            let y = wa.y + ov + si as i32 * (h + iv);
            Rect::new(wa.x + oh, y, stack_w, h)
        };
        result.push((idx, rect));
    }
    result
}

// ── Monocle (all windows maximised to work area) ─────────────────────────────

pub fn monocle(ctx: &ArrangeCtx) -> ArrangeResult {
    let wa = &ctx.work_area;
    let g = ctx.gaps;
    let rect = Rect::new(
        wa.x + g.gappoh,
        wa.y + g.gappov,
        wa.width - 2 * g.gappoh,
        wa.height - 2 * g.gappov,
    );
    ctx.tiled.iter().map(|&idx| (idx, rect)).collect()
}

// ── Grid ─────────────────────────────────────────────────────────────────────

pub fn grid(ctx: &ArrangeCtx) -> ArrangeResult {
    let n = ctx.tiled.len();
    if n == 0 {
        return vec![];
    }

    let wa = &ctx.work_area;
    let g = ctx.gaps;
    let oh = g.gappoh;
    let ov = g.gappov;
    let ih = g.gappih;
    let iv = g.gappiv;

    let total_w = wa.width - 2 * oh;
    let total_h = wa.height - 2 * ov;

    if n == 1 {
        let cw = (total_w as f32 * 0.9) as i32;
        let ch = (total_h as f32 * 0.9) as i32;
        return vec![(
            ctx.tiled[0],
            Rect::new(
                wa.x + oh + (total_w - cw) / 2,
                wa.y + ov + (total_h - ch) / 2,
                cw,
                ch,
            ),
        )];
    }

    // cols such that cols*cols >= n
    let mut cols = 1usize;
    while cols * cols < n {
        cols += 1;
    }
    let rows = if cols > 1 && (cols - 1) * cols >= n {
        cols - 1
    } else {
        cols
    };

    let cw = (total_w - (cols - 1) as i32 * ih) / cols as i32;
    let ch = (total_h - (rows - 1) as i32 * iv) / rows as i32;
    let overcols = n % cols;
    let dx = if overcols > 0 {
        (total_w - overcols as i32 * cw - (overcols - 1) as i32 * ih) / 2
    } else {
        0
    };

    let mut result = Vec::with_capacity(n);
    for (i, &idx) in ctx.tiled.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let extra_x = if overcols > 0 && i >= n - overcols {
            dx
        } else {
            0
        };
        let rect = Rect::new(
            wa.x + oh + col as i32 * (cw + ih) + extra_x,
            wa.y + ov + row as i32 * (ch + iv),
            cw,
            ch,
        );
        result.push((idx, rect));
    }
    result
}

// ── Deck (master + all-stack stacked) ────────────────────────────────────────

pub fn deck(ctx: &ArrangeCtx) -> ArrangeResult {
    let n = ctx.tiled.len();
    if n == 0 {
        return vec![];
    }

    let wa = &ctx.work_area;
    let g = ctx.gaps;
    let oh = g.gappoh;
    let ov = g.gappov;
    let ih = g.gappih;

    let nm = (ctx.nmaster as usize).min(n);
    let stack_count = n.saturating_sub(nm);

    let total_w = wa.width - 2 * oh;
    let total_h = wa.height - 2 * ov;

    let mw = if stack_count > 0 {
        (total_w as f32 * ctx.mfact) as i32
    } else {
        total_w
    };
    let sw = total_w - mw - if stack_count > 0 { ih } else { 0 };

    let stack_rect = Rect::new(wa.x + oh + mw + ih, wa.y + ov, sw, total_h);

    let mut result = Vec::with_capacity(n);
    let mut my = 0;
    for (i, &idx) in ctx.tiled.iter().enumerate() {
        if i < nm {
            let h = (total_h - my) / (nm - i) as i32;
            result.push((idx, Rect::new(wa.x + oh, wa.y + ov + my, mw, h)));
            my += h;
        } else {
            result.push((idx, stack_rect));
        }
    }
    result
}

// ── Center tile ───────────────────────────────────────────────────────────────

pub fn center_tile(ctx: &ArrangeCtx) -> ArrangeResult {
    let n = ctx.tiled.len();
    if n == 0 {
        return vec![];
    }

    let wa = &ctx.work_area;
    let g = ctx.gaps;
    let oh = g.gappoh;
    let ov = g.gappov;
    let ih = g.gappih;

    let nm = (ctx.nmaster as usize).min(n);
    let stack_count = n.saturating_sub(nm);

    let total_w = wa.width - 2 * oh;
    let total_h = wa.height - 2 * ov;

    let master_w = if stack_count > 0 {
        (total_w as f32 * ctx.mfact) as i32
    } else {
        total_w
    };

    let side_w = if stack_count >= 2 {
        (total_w - master_w - 2 * ih) / 2
    } else if stack_count == 1 {
        total_w - master_w - ih
    } else {
        0
    };

    let left_count = stack_count / 2;
    let right_count = stack_count - left_count;

    let mut result = Vec::with_capacity(n);

    // master column (center)
    let master_x = wa.x + oh + if stack_count >= 2 { side_w + ih } else { 0 };
    let mut my = 0;
    for i in 0..nm {
        let idx = ctx.tiled[i];
        let h = (total_h - my) / (nm - i) as i32;
        result.push((idx, Rect::new(master_x, wa.y + ov + my, master_w, h)));
        my += h;
    }

    // left stack
    let mut ly = 0;
    for i in 0..left_count {
        let idx = ctx.tiled[nm + i];
        let h = (total_h - ly) / (left_count - i) as i32;
        result.push((idx, Rect::new(wa.x + oh, wa.y + ov + ly, side_w, h)));
        ly += h;
    }

    // right stack
    let rx = wa.x
        + oh
        + if stack_count >= 2 {
            side_w + ih + master_w + ih
        } else {
            master_w + ih
        };
    let mut ry = 0;
    for i in 0..right_count {
        let idx = ctx.tiled[nm + left_count + i];
        let h = (total_h - ry) / (right_count - i) as i32;
        result.push((idx, Rect::new(rx, wa.y + ov + ry, side_w, h)));
        ry += h;
    }

    result
}

// ── Scroller (horizontal scrolling layout) ───────────────────────────────────

pub fn scroller(ctx: &ArrangeCtx) -> ArrangeResult {
    let n = ctx.tiled.len();
    if n == 0 {
        return vec![];
    }

    let wa = &ctx.work_area;
    let g = ctx.gaps;
    let oh = g.gappoh;
    let ov = g.gappov;
    let ih = g.gappih;

    let total_h = (wa.height - 2 * ov).max(1);
    let default_prop = ctx.default_scroller_proportion;
    let side_margin = ctx.scroller_structs.max(0);
    let max_client_w = (wa.width - 2 * side_margin - ih).max(1);

    let widths: Vec<i32> = (0..n)
        .map(|i| {
            let prop = ctx
                .scroller_proportions
                .get(i)
                .copied()
                .unwrap_or(default_prop)
                .clamp(0.1, 1.0);
            ((max_client_w as f32) * prop).round().max(1.0) as i32
        })
        .collect();

    let mut x = wa.x + oh;
    let mut raw_x = Vec::with_capacity(n);
    for width in &widths {
        raw_x.push(x);
        x += *width + ih;
    }

    let focus_pos = ctx.focused_tiled_pos.filter(|&pos| pos < n).unwrap_or(0);
    let focus_w = widths[focus_pos];
    let focus_raw_x = raw_x[focus_pos];
    let visible_left = wa.x + side_margin;
    let visible_right = wa.x + wa.width - side_margin;
    let center_focused =
        n == 1 || ctx.scroller_focus_center || (ctx.scroller_prefer_center && n > 1);

    let desired_focus_x = if center_focused {
        wa.x + (wa.width - focus_w) / 2
    } else if ctx.scroller_prefer_overspread && focus_pos == 0 && n > 1 {
        visible_left
    } else if ctx.scroller_prefer_overspread && focus_pos + 1 == n && n > 1 {
        visible_right - focus_w
    } else if focus_raw_x < visible_left {
        visible_left
    } else if focus_raw_x + focus_w > visible_right {
        visible_right - focus_w
    } else {
        focus_raw_x
    };
    let shift = desired_focus_x - focus_raw_x;

    let mut result = Vec::with_capacity(n);

    for (i, &idx) in ctx.tiled.iter().enumerate() {
        result.push((
            idx,
            Rect::new(raw_x[i] + shift, wa.y + ov, widths[i], total_h),
        ));
    }
    result
}

// ── TgMix (tile master, grid stack) ──────────────────────────────────────────

pub fn tgmix(ctx: &ArrangeCtx) -> ArrangeResult {
    let n = ctx.tiled.len();
    if n == 0 {
        return vec![];
    }

    let nm = (ctx.nmaster as usize).min(n);
    if nm == n {
        return tile(ctx);
    }

    let wa = &ctx.work_area;
    let g = ctx.gaps;
    let oh = g.gappoh;
    let master_w = ((wa.width - 2 * oh) as f32 * ctx.mfact) as i32;

    let master_wa = Rect::new(wa.x, wa.y, master_w + 2 * oh, wa.height);
    let stack_wa = Rect::new(
        wa.x + oh + master_w + g.gappih,
        wa.y,
        wa.width - oh - master_w - g.gappih,
        wa.height,
    );

    let master_props: Vec<f32> =
        ctx.scroller_proportions[..ctx.scroller_proportions.len().min(nm)].to_vec();
    let stack_props: Vec<f32> = if ctx.scroller_proportions.len() > nm {
        ctx.scroller_proportions[nm..].to_vec()
    } else {
        Vec::new()
    };
    let master_focus = ctx.focused_tiled_pos.filter(|&pos| pos < nm);
    let stack_focus = ctx
        .focused_tiled_pos
        .and_then(|pos| pos.checked_sub(nm))
        .filter(|&pos| pos < ctx.tiled.len().saturating_sub(nm));

    let master_ctx = ArrangeCtx {
        work_area: master_wa,
        tiled: &ctx.tiled[..nm],
        nmaster: ctx.nmaster,
        mfact: ctx.mfact,
        gaps: ctx.gaps,
        scroller_proportions: &master_props,
        default_scroller_proportion: ctx.default_scroller_proportion,
        focused_tiled_pos: master_focus,
        scroller_structs: ctx.scroller_structs,
        scroller_focus_center: ctx.scroller_focus_center,
        scroller_prefer_center: ctx.scroller_prefer_center,
        scroller_prefer_overspread: ctx.scroller_prefer_overspread,
        canvas_pan: ctx.canvas_pan,
    };
    let stack_ctx = ArrangeCtx {
        work_area: stack_wa,
        tiled: &ctx.tiled[nm..],
        nmaster: 0,
        mfact: ctx.mfact,
        gaps: ctx.gaps,
        scroller_proportions: &stack_props,
        default_scroller_proportion: ctx.default_scroller_proportion,
        focused_tiled_pos: stack_focus,
        scroller_structs: ctx.scroller_structs,
        scroller_focus_center: ctx.scroller_focus_center,
        scroller_prefer_center: ctx.scroller_prefer_center,
        scroller_prefer_overspread: ctx.scroller_prefer_overspread,
        canvas_pan: ctx.canvas_pan,
    };

    let mut result = tile(&master_ctx);
    result.extend(grid(&stack_ctx));
    result
}

// ── Canvas (infinite canvas — positions set externally) ──────────────────────

/// For canvas layout, this just returns each client in its current canvas_geom.
/// The actual pan/zoom transforms are applied at render time by the compositor.
pub fn canvas(_ctx: &ArrangeCtx) -> ArrangeResult {
    // Canvas layout does not reposition clients through the normal arrange path;
    // each client retains its canvas_geom position set by pan/zoom operations.
    vec![]
}

// ── Dwindle ───────────────────────────────────────────────────────────────────

pub fn dwindle(ctx: &ArrangeCtx) -> ArrangeResult {
    let n = ctx.tiled.len();
    if n == 0 {
        return vec![];
    }

    let wa = &ctx.work_area;
    let g = ctx.gaps;
    let oh = g.gappoh;
    let ov = g.gappov;
    let ih = g.gappih;
    let iv = g.gappiv;

    let mut rect = Rect::new(wa.x + oh, wa.y + ov, wa.width - 2 * oh, wa.height - 2 * ov);
    let mut result = Vec::with_capacity(n);

    for (i, &idx) in ctx.tiled.iter().enumerate() {
        if i == n - 1 {
            result.push((idx, rect));
            break;
        }
        let split_h = i % 2 == 0;
        if split_h {
            let half = (rect.width - ih) / 2;
            result.push((idx, Rect::new(rect.x, rect.y, half, rect.height)));
            rect = Rect::new(
                rect.x + half + ih,
                rect.y,
                rect.width - half - ih,
                rect.height,
            );
        } else {
            let half = (rect.height - iv) / 2;
            result.push((idx, Rect::new(rect.x, rect.y, rect.width, half)));
            rect = Rect::new(
                rect.x,
                rect.y + half + iv,
                rect.width,
                rect.height - half - iv,
            );
        }
    }
    result
}

// ── Arrange dispatcher ────────────────────────────────────────────────────────

use crate::LayoutId;

pub fn arrange(layout: LayoutId, ctx: &ArrangeCtx) -> ArrangeResult {
    match layout {
        LayoutId::Tile => tile(ctx),
        LayoutId::RightTile => right_tile(ctx),
        LayoutId::Monocle => monocle(ctx),
        LayoutId::Grid => grid(ctx),
        LayoutId::Deck => deck(ctx),
        LayoutId::CenterTile => center_tile(ctx),
        LayoutId::Scroller => scroller(ctx),
        LayoutId::TgMix => tgmix(ctx),
        LayoutId::Canvas => canvas(ctx),
        LayoutId::Dwindle => dwindle(ctx),
        LayoutId::Overview => monocle(ctx), // overview handled elsewhere
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GapConfig;

    #[test]
    fn scroller_centers_the_focused_client() {
        let gaps = GapConfig {
            gappih: 8,
            gappiv: 8,
            gappoh: 8,
            gappov: 8,
        };
        let tiled = [10, 11, 12];
        let proportions = [0.8, 0.8, 0.8];
        let ctx = ArrangeCtx {
            work_area: Rect::new(0, 0, 1000, 600),
            tiled: &tiled,
            nmaster: 1,
            mfact: 0.55,
            gaps: &gaps,
            scroller_proportions: &proportions,
            default_scroller_proportion: 0.8,
            focused_tiled_pos: Some(2),
            scroller_structs: 24,
            scroller_focus_center: true,
            scroller_prefer_center: true,
            scroller_prefer_overspread: false,
            canvas_pan: (0.0, 0.0),
        };

        let arranged = scroller(&ctx);
        let focused = arranged.iter().find(|(idx, _)| *idx == 12).unwrap().1;
        assert!(focused.x >= 0);
        assert!(focused.x + focused.width <= 1000);
        assert!(((focused.x + focused.width / 2) - 500).abs() <= 1);
    }

    /// Asymmetric inner gaps: horizontal (`gappih`) separates the master
    /// column from the stack; vertical (`gappiv`) separates the windows
    /// stacked within the stack column. They must not be conflated.
    const ASYM: GapConfig = GapConfig {
        gappih: 10,
        gappiv: 20,
        gappoh: 0,
        gappov: 0,
    };

    fn ctx<'a>(tiled: &'a [usize], gaps: &'a GapConfig, nmaster: u32) -> ArrangeCtx<'a> {
        ArrangeCtx {
            work_area: Rect::new(0, 0, 1000, 600),
            tiled,
            nmaster,
            mfact: 0.5,
            gaps,
            scroller_proportions: &[],
            default_scroller_proportion: 0.8,
            focused_tiled_pos: None,
            scroller_structs: 0,
            scroller_focus_center: false,
            scroller_prefer_center: false,
            scroller_prefer_overspread: false,
            canvas_pan: (0.0, 0.0),
        }
    }

    #[test]
    fn tile_uses_horizontal_gap_between_columns_and_vertical_within_a_column() {
        // 1 master + 2 stack.
        let tiled = [0usize, 1, 2];
        let r = tile(&ctx(&tiled, &ASYM, 1));
        let master = r[0].1;
        let stack0 = r[1].1;
        let stack1 = r[2].1;
        // Master↔stack separation is horizontal → gappih.
        assert_eq!(stack0.x - (master.x + master.width), ASYM.gappih);
        // Within the stack column, the two windows are separated
        // vertically → gappiv.
        assert_eq!(stack1.y - (stack0.y + stack0.height), ASYM.gappiv);
    }

    #[test]
    fn grid_keeps_column_and_row_gaps_distinct() {
        // 4 windows → 2×2 grid.
        let tiled = [0usize, 1, 2, 3];
        let r = grid(&ctx(&tiled, &ASYM, 1));
        // Indices 0,1 share a row (cols=2): horizontal gap = gappih.
        assert_eq!(r[1].1.x - (r[0].1.x + r[0].1.width), ASYM.gappih);
        // Indices 0,2 share a column: vertical gap = gappiv.
        assert_eq!(r[2].1.y - (r[0].1.y + r[0].1.height), ASYM.gappiv);
    }

    #[test]
    fn right_tile_uses_horizontal_gap_between_columns_and_vertical_within_a_column() {
        // Regression: right_tile used gappih for the vertical stacking too
        // (and never read gappiv), so an asymmetric gappih≠gappiv tiled wrong.
        let tiled = [0usize, 1, 2]; // 1 master (right) + 2 stack (left).
        let r = right_tile(&ctx(&tiled, &ASYM, 1));
        let master = r[0].1;
        let stack0 = r[1].1;
        let stack1 = r[2].1;
        // Stack column ↔ master column separation is horizontal → gappih.
        assert_eq!(master.x - (stack0.x + stack0.width), ASYM.gappih);
        // Two stack windows stacked vertically → gappiv.
        assert_eq!(stack1.y - (stack0.y + stack0.height), ASYM.gappiv);
    }

}
