//! Tiling-arrangement methods on `MargoState`.
//!
//! Extracted from `state.rs` (state.rs split): the layout-arrangement cluster
//! — `arrange_monitor` (the per-monitor tiling driver, the single largest
//! method), its `arrange_all`/`arrange_monitors` fan-out wrappers, the
//! overview pre-arrange pass, `configure_window_size`, and `enforce_z_order`.
//! Pure `MargoState` glue, no new types.

use super::*;

/// Every column-mate's *effective* min-width floor: each member's own
/// declared minimum, boosted to the largest declared minimum anywhere
/// in its [`stack_columns`] group.
///
/// A real vertical column (`center_tile`/`tile`/`right_tile`'s side
/// stack: same `x`, same width, stacked top to bottom) must stay one
/// consistent width — but `apply_min_width_floors` only ever redistributes
/// within a [`stack_rows`] group (members genuinely side by side sharing a
/// width budget), and a column's own members have `y` ranges that don't
/// overlap at all (separated by the layout's own inner gap, or, in
/// `center_tile`'s stack loops specifically, not separated by any gap at
/// all) — so `stack_rows` never groups them together, and a member with
/// its own oversized `min_width` (an Electron app like Discord commonly
/// declares one) grew alone, leaving its column-mates at their old,
/// narrower width: the column stopped being one consistent width, opening
/// a gap next to whatever sits beside it.
///
/// Boosting every column-mate's floor *before* `apply_min_width_floors`
/// runs means its normal per-row redistribution does the rest on its
/// own — including shrinking a column-mate's own *row*-mate to make
/// room, something a separate sync pass run before or after that
/// redistribution couldn't do without either being skipped by it (too
/// early: the no-op guard sees a floor `apply_min_width_floors` didn't
/// know was already met) or leaving a row it never got to revisit stuck
/// with the new floor unhandled (too late).
///
/// Skipped for `Deck` (its stack is *deliberately* one shared slot — only
/// the top member is ever shown, and each keeps its own independent
/// floor, never synced to a sibling it's never simultaneously visible
/// with — see `apply_min_width_floors`'s coincident-rect case) and
/// `Monocle` (every window shares the exact same rect on purpose), for
/// the same reason `resolve_residual_overlaps` skips them.
fn effective_min_widths(
    geometries: &[(usize, crate::layout::Rect)],
    clients: &[MargoClient],
    layout: crate::layout::LayoutId,
    column_groups: &std::collections::BTreeMap<usize, Vec<usize>>,
) -> Vec<i32> {
    let mut floors: Vec<i32> = geometries
        .iter()
        .map(|&(ci, _)| clients[ci].min_width.max(0))
        .collect();
    use crate::layout::LayoutId;
    if matches!(layout, LayoutId::Deck | LayoutId::Monocle) {
        return floors;
    }
    for members in column_groups.values() {
        if members.len() < 2 {
            continue;
        }
        let max_floor = members.iter().map(|&i| floors[i]).max().unwrap_or(0);
        for &i in members {
            floors[i] = max_floor;
        }
    }
    floors
}

/// Grow each client's rect to its own `min_height` (from a window rule
/// or its own xdg_toplevel request), but never past what its column
/// can actually hold.
///
/// `layout::arrange` hands every tile-family layout's master/stack
/// column a set of rects that share a horizontal territory and sum,
/// with the gaps between them, to exactly the column's span. A single
/// client's own declared minimum height can be bigger than its fair
/// share of that column (an Electron app like Discord commonly
/// declares one) — growing just that client, as the old code did,
/// left its neighbours untouched and let the grown client (and every
/// sibling below it) spill past the column's own span: off the bottom
/// of the screen in `tile`, or into a neighbouring column's territory
/// in `center_tile`/`right_tile`, since nothing shrank to make room.
///
/// Groups `geometries` into vertical-stack columns (see
/// [`stack_columns`]) and hands each group with more than one member
/// to [`layout::repack_1d`], which grows the min-height member(s)
/// exactly to their floor and shrinks the rest to compensate, keeping
/// the group's total unchanged. A group of one has no sibling to
/// shrink; it just takes its floor directly, same as the old
/// per-client-only clamp did.
fn apply_min_height_floors(
    geometries: &mut [(usize, crate::layout::Rect)],
    clients: &[MargoClient],
    column_groups: &std::collections::BTreeMap<usize, Vec<usize>>,
) {
    for members in column_groups.values() {
        if members.len() == 1 {
            let gi = members[0];
            let (client_idx, rect) = &mut geometries[gi];
            let min_h = clients[*client_idx].min_height;
            if min_h > rect.height {
                rect.height = min_h;
            }
            continue;
        }

        let mut ordered = members.clone();
        ordered.sort_by_key(|&gi| geometries[gi].1.y);

        // `deck`'s stack members deliberately share one identical rect
        // (a tabbed "deck of cards" — only one is ever shown at a
        // time), so every member in this group can land at the exact
        // same `y`.
        let distinct_y = ordered
            .iter()
            .map(|&gi| geometries[gi].1.y)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if distinct_y <= 1 {
            // Same `y` alone isn't proof of `deck`'s literal same-rect
            // stack — `stack_columns` groups by "same width and *any*
            // x-overlap", and a real row (grid/tgmix/center_tile: same
            // y, same height, laid out side by side) can register a
            // sliver of x-overlap right at a shared gap boundary, or
            // via the odd-row centring `stack_columns`'s own doc
            // comment describes. Only when `x` is *also* shared (a
            // true single-point stack, or a group of one) is there
            // nothing to keep in sync — each takes its own floor
            // independently. Otherwise this is a real row that
            // `stack_columns` mis-caught: every member must stay the
            // same height, so every member grows to match whichever
            // one's floor is biggest, instead of just that one.
            let distinct_x = ordered
                .iter()
                .map(|&gi| geometries[gi].1.x)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            if distinct_x <= 1 {
                for &gi in &ordered {
                    let (client_idx, rect) = &mut geometries[gi];
                    let min_h = clients[*client_idx].min_height;
                    if min_h > rect.height {
                        rect.height = min_h;
                    }
                }
            } else {
                let max_floor = ordered
                    .iter()
                    .map(|&gi| clients[geometries[gi].0].min_height.max(0))
                    .max()
                    .unwrap_or(0);
                for &gi in &ordered {
                    let rect = &mut geometries[gi].1;
                    if max_floor > rect.height {
                        rect.height = max_floor;
                    }
                }
            }
            continue;
        }

        let sizes: Vec<i32> = ordered.iter().map(|&gi| geometries[gi].1.height).collect();
        let floors: Vec<i32> = ordered
            .iter()
            .map(|&gi| clients[geometries[gi].0].min_height.max(0))
            .collect();

        // Nobody in this group actually needs to grow: leave every member
        // exactly where `layout::arrange` put it. `stack_columns` groups
        // rects by "same width and x-ranges overlap", which is a good
        // enough proxy for "real column" most of the time but isn't
        // precise — a centred odd cell in an incomplete grid row can
        // straddle two real columns and get transitively unioned into
        // one group with both of them (see
        // `grid_with_an_incomplete_last_row_does_not_collapse_into_a_diagonal_cascade`).
        // Re-flowing positions below is only safe (and only intended) when
        // there's an actual floor to make room for; without one, this
        // guard is what keeps a mis-grouped column from being flattened
        // into a stack that was never really there.
        if floors.iter().zip(&sizes).all(|(f, s)| f <= s) {
            continue;
        }

        let gaps: Vec<i32> = ordered
            .windows(2)
            .map(|w| {
                let prev = geometries[w[0]].1;
                let next = geometries[w[1]].1;
                (next.y - (prev.y + prev.height)).max(0)
            })
            .collect();
        let Some(&last_gi) = ordered.last() else {
            // `ordered` mirrors `members`, and we're past the
            // `members.len() == 1` early-continue above, so this is
            // unreachable — but a group that somehow came in empty is
            // simply nothing to repack, not a crash.
            continue;
        };
        let top = geometries[ordered[0]].1.y;
        let bottom = {
            let last = geometries[last_gi].1;
            last.y + last.height
        };
        let span = bottom - top;

        let repacked = layout::repack_1d(&sizes, &floors, &gaps, span);

        let mut y = top;
        for (n, &gi) in ordered.iter().enumerate() {
            let h = repacked[n];
            geometries[gi].1.y = y;
            geometries[gi].1.height = h;
            y += h + gaps.get(n).copied().unwrap_or(0);
        }
    }
}

/// Width's twin of [`apply_min_height_floors`]: grows each client's
/// rect to its own `min_width`, redistributing within its row (see
/// [`stack_rows`]) instead of spilling past whatever's next to it.
///
/// The same shape that motivated the height version shows up on this
/// axis too: `grid` (reached directly, or through `tgmix`'s stack
/// half) lays cells out in genuine rows, and a cell whose client
/// declares a real minimum width (Discord's chat UI needs real
/// horizontal room, not just vertical) grew past its column boundary
/// in place, riding straight over its row-mate instead of shrinking it
/// to make room.
fn apply_min_width_floors(
    geometries: &mut [(usize, crate::layout::Rect)],
    effective_min_width: &[i32],
    row_groups: &std::collections::BTreeMap<usize, Vec<usize>>,
) {
    for members in row_groups.values() {
        if members.len() == 1 {
            let gi = members[0];
            let min_w = effective_min_width[gi];
            let rect = &mut geometries[gi].1;
            if min_w > rect.width {
                rect.width = min_w;
            }
            continue;
        }

        let mut ordered = members.clone();
        ordered.sort_by_key(|&gi| geometries[gi].1.x);

        // Mirrors `deck`'s coincident-rect case in
        // `apply_min_height_floors`: members that all share the same
        // `x` aren't dividing a row between them, so there's nothing
        // to redistribute.
        let distinct_x = ordered
            .iter()
            .map(|&gi| geometries[gi].1.x)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if distinct_x <= 1 {
            // Same `x` alone isn't proof they're `deck`'s literal
            // same-rect stack — `stack_rows` groups by "same height and
            // *any* y-overlap", and two members of a real vertical
            // column (center_tile/tile/right_tile's side stack: same x,
            // same width, stacked with a real gap) can register a
            // sliver of y-overlap right at that gap's boundary. Only
            // when `y` is *also* shared by every member (deck's actual
            // coincident rect, or a group of one) is there truly
            // nothing to keep in sync — each takes its own floor
            // independently, as before. Otherwise this is a real
            // column that `stack_rows` mis-caught: every member must
            // stay the same width, so instead of growing just the one
            // whose floor is biggest, every member grows to match it —
            // the same shape a genuinely mixed floor produced within a
            // real column when the members' `y` truly differ.
            let distinct_y = ordered
                .iter()
                .map(|&gi| geometries[gi].1.y)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            if distinct_y <= 1 {
                for &gi in &ordered {
                    let min_w = effective_min_width[gi];
                    let rect = &mut geometries[gi].1;
                    if min_w > rect.width {
                        rect.width = min_w;
                    }
                }
            } else {
                let max_floor = ordered
                    .iter()
                    .map(|&gi| effective_min_width[gi])
                    .max()
                    .unwrap_or(0);
                for &gi in &ordered {
                    let rect = &mut geometries[gi].1;
                    if max_floor > rect.width {
                        rect.width = max_floor;
                    }
                }
            }
            continue;
        }

        let sizes: Vec<i32> = ordered.iter().map(|&gi| geometries[gi].1.width).collect();
        let floors: Vec<i32> = ordered.iter().map(|&gi| effective_min_width[gi]).collect();

        // Width's twin of the height guard above: nobody in this group
        // needs to grow, so leave every member exactly where
        // `layout::arrange` put it. `stack_rows` groups rects by "same
        // height and y-ranges overlap" — with `deck`'s default nmaster=1,
        // the single master column is exactly as tall as the coincident
        // stack rect beside it, so that heuristic alone would merge the
        // master into the stack's group. Re-flowing positions below is
        // only safe (and only intended) when there's an actual floor to
        // make room for; without one, this guard is what stops that
        // false merge from splitting `deck` into a row of columns (see
        // `deck_stack_stays_one_coincident_rect_not_a_row_of_columns`).
        if floors.iter().zip(&sizes).all(|(f, s)| f <= s) {
            continue;
        }

        let gaps: Vec<i32> = ordered
            .windows(2)
            .map(|w| {
                let prev = geometries[w[0]].1;
                let next = geometries[w[1]].1;
                (next.x - (prev.x + prev.width)).max(0)
            })
            .collect();
        let Some(&last_gi) = ordered.last() else {
            continue;
        };
        let left = geometries[ordered[0]].1.x;
        let right = {
            let last = geometries[last_gi].1;
            last.x + last.width
        };
        let span = right - left;

        let repacked = layout::repack_1d(&sizes, &floors, &gaps, span);

        let mut x = left;
        for (n, &gi) in ordered.iter().enumerate() {
            let w = repacked[n];
            geometries[gi].1.x = x;
            geometries[gi].1.width = w;
            x += w + gaps.get(n).copied().unwrap_or(0);
        }
    }
}

/// Last-resort safety net after [`apply_min_height_floors`]: nudges any
/// rect that still spills past `work_area`'s edges back on-screen by
/// repositioning it (never by shrinking — a min-size floor stays
/// honoured), for every layout whose own contract already promises
/// every client fits inside the work area.
///
/// Column-based redistribution fixes the common shape (a simple
/// master/stack column, or a grid column reached through `tgmix`) with
/// no overlap at all. But `dwindle`'s spiral splits the work area on
/// an *alternating* axis each step, so a stack member's "column mate"
/// for a height overflow can be a sibling it shares *width* with, not
/// height — outside what column-grouping can redistribute into. Rather
/// than leave that case hanging off the bottom of the screen (content
/// no key or click can ever reach), reposition it fully on-screen; in
/// the rare case that still means overlapping one immediate neighbour,
/// that is strictly better than being partly invisible and dead to
/// input.
///
/// Skips `Scroller`, whose columns intentionally extend past the work
/// area (horizontal panning is the point) — every other layout this
/// runs on already guarantees full containment by design (see
/// `contained_layouts_keep_every_rect_inside_the_work_area` in
/// `margo-layouts`), so clamping here restores that guarantee rather
/// than fighting it.
///
/// A member of a real multi-member [`stack_columns`] / [`stack_rows`]
/// group is repositioned as part of that whole group, sliding every
/// member by the same offset, rather than independently — clamping a
/// single grown member back on-screen on its own can push it past
/// where its own column-mate already sits (dwindle's spiral hands a
/// deep leaf a floor bigger than its entire column can hold; nudging
/// just that leaf up to fit the work area rides straight over the
/// sibling directly above it, which was already correctly positioned).
/// Sliding the whole group preserves every member's relative order and
/// gap — the group may still hang off the work area's edge afterwards
/// if its combined size genuinely doesn't fit, the same honest,
/// accepted overflow a lone oversized member already had before this
/// existed; it just never trades that overflow for a new overlap with
/// a sibling that was already fine. Singleton groups fall through to
/// the plain independent clamp below, unchanged.
fn clamp_to_work_area(
    geometries: &mut [(usize, crate::layout::Rect)],
    work_area: crate::layout::Rect,
    layout: crate::layout::LayoutId,
    column_groups: &std::collections::BTreeMap<usize, Vec<usize>>,
    row_groups: &std::collections::BTreeMap<usize, Vec<usize>>,
) {
    if layout == crate::layout::LayoutId::Scroller {
        return;
    }

    let mut in_column_group: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for members in column_groups.values() {
        if members.len() < 2 {
            continue;
        }
        // `members.len() >= 2` was just checked above, so this group is
        // never empty — `.min()`/`.max()` would only return `None` on an
        // empty iterator, which can't happen here, but `unwrap_or` keeps
        // this a graceful fallback rather than a panic-prone call.
        let top = members
            .iter()
            .map(|&i| geometries[i].1.y)
            .min()
            .unwrap_or(work_area.y);
        let bottom = members
            .iter()
            .map(|&i| geometries[i].1.y + geometries[i].1.height)
            .max()
            .unwrap_or(work_area.y + work_area.height);
        let wa_top = work_area.y;
        let wa_bottom = work_area.y + work_area.height;
        let mut shift = if bottom > wa_bottom {
            wa_bottom - bottom
        } else {
            0
        };
        if top + shift < wa_top {
            shift += wa_top - (top + shift);
        }
        if shift != 0 {
            for &i in members {
                geometries[i].1.y += shift;
            }
        }
        in_column_group.extend(members.iter().copied());
    }

    let mut in_row_group: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for members in row_groups.values() {
        if members.len() < 2 {
            continue;
        }
        // Same graceful fallback as the column-group branch above.
        let left = members
            .iter()
            .map(|&i| geometries[i].1.x)
            .min()
            .unwrap_or(work_area.x);
        let right = members
            .iter()
            .map(|&i| geometries[i].1.x + geometries[i].1.width)
            .max()
            .unwrap_or(work_area.x + work_area.width);
        let wa_left = work_area.x;
        let wa_right = work_area.x + work_area.width;
        let mut shift = if right > wa_right {
            wa_right - right
        } else {
            0
        };
        if left + shift < wa_left {
            shift += wa_left - (left + shift);
        }
        if shift != 0 {
            for &i in members {
                geometries[i].1.x += shift;
            }
        }
        in_row_group.extend(members.iter().copied());
    }

    for (i, (_, rect)) in geometries.iter_mut().enumerate() {
        if !in_row_group.contains(&i) {
            if rect.width < work_area.width {
                rect.x = rect
                    .x
                    .clamp(work_area.x, work_area.x + work_area.width - rect.width);
            } else {
                rect.x = work_area.x;
            }
        }
        if !in_column_group.contains(&i) {
            if rect.height < work_area.height {
                rect.y = rect
                    .y
                    .clamp(work_area.y, work_area.y + work_area.height - rect.height);
            } else {
                rect.y = work_area.y;
            }
        }
    }
}

/// Final safety net after [`clamp_to_work_area`]: a min-size floor grew a
/// client's rect exactly the way [`apply_min_height_floors`] /
/// [`apply_min_width_floors`] intend, but the group it landed in
/// ([`stack_columns`] / [`stack_rows`]) only recognises rects that share a
/// dimension outright. `dwindle`'s alternating-axis spiral routinely
/// produces direct siblings — one leaf, and the recursively-split branch
/// beside it — whose *combined* span matches the leaf's but whose
/// individual height (or width) doesn't, so a floor grew one of them
/// straight into its true sibling's territory without either ever being
/// grouped for redistribution, and nothing was off-screen for
/// `clamp_to_work_area` to catch.
///
/// Sweeps every pair still overlapping after everything above and shrinks
/// whichever one has room to give — on whichever axis needs the smaller
/// correction, from the shared edge inward — without ever shrinking a
/// rect past its own client's declared minimum. When neither side has
/// enough room, the residual overlap is left in place: the same accepted
/// last resort `clamp_to_work_area`'s own doc comment describes, and here
/// too neither client's floor can honestly give any further.
///
/// Skipped for layouts whose members are *meant* to occupy the same
/// space: `Deck`'s stack (a tabbed "deck of cards", only the top one
/// shown) and `Monocle` (every window maximised to the same rect), and
/// for `Scroller`, whose columns intentionally grow past their
/// neighbours (see `clamp_to_work_area`).
fn resolve_residual_overlaps(
    geometries: &mut [(usize, crate::layout::Rect)],
    clients: &[MargoClient],
    layout: crate::layout::LayoutId,
    column_groups: &std::collections::BTreeMap<usize, Vec<usize>>,
    row_groups: &std::collections::BTreeMap<usize, Vec<usize>>,
) {
    use crate::layout::LayoutId;
    if matches!(
        layout,
        LayoutId::Scroller | LayoutId::Deck | LayoutId::Monocle
    ) {
        return;
    }
    let n = geometries.len();
    // A pair `apply_min_height_floors`/`apply_min_width_floors` already
    // grouped and correctly redistributed (real column/row siblings,
    // like a dwindle leaf and its direct column-mate) must never be
    // re-touched here — this pass only owns relationships neither of
    // those recognises, like a dwindle "uncle" and the nephews one
    // level down. Touching an already-settled column/row pair a second
    // time, blind to the fact it's already balanced, is what corrupted
    // it: shrinking one member here to fix an unrelated overlap
    // elsewhere left its real column-mate's own redistribution
    // inconsistent. Membership comes from the same natural-geometry
    // `column_groups`/`row_groups` every other pass uses — fixed once,
    // not recomputed from whatever this function has already reshaped.
    let mut column_of = vec![usize::MAX; n];
    for (gid, members) in column_groups.values().enumerate() {
        for &m in members {
            column_of[m] = gid;
        }
    }
    let mut row_of = vec![usize::MAX; n];
    for (gid, members) in row_groups.values().enumerate() {
        for &m in members {
            row_of[m] = gid;
        }
    }
    // A handful of passes lets a chain of overlaps (A pushes into B,
    // B's shrink then reveals it still overlaps C) settle instead of
    // stopping after resolving only the first link.
    for _ in 0..3 {
        let mut any_resolved = false;
        for i in 0..n {
            for j in (i + 1)..n {
                if column_of[i] == column_of[j] || row_of[i] == row_of[j] {
                    continue;
                }
                let a = geometries[i].1;
                let b = geometries[j].1;
                let ox = a.x.max(b.x);
                let ox_end = (a.x + a.width).min(b.x + b.width);
                let oy = a.y.max(b.y);
                let oy_end = (a.y + a.height).min(b.y + b.height);
                let overlap_w = ox_end - ox;
                let overlap_h = oy_end - oy;
                if overlap_w <= 0 || overlap_h <= 0 {
                    continue;
                }
                any_resolved = true;
                // Which axis actually separates this pair? A straddle —
                // neither rect's extent is fully swallowed by the
                // overlap — is the signature of two things meant to sit
                // side by side on that axis, one having grown into the
                // other. Full containment on an axis (the overlap covers
                // one rect's *entire* extent there) means that axis was
                // never the dividing line — a dwindle "uncle" naturally
                // spans its nephews' whole combined height, so height
                // nests without ever being the axis they're actually
                // split on. Prefer resolving on the straddling axis;
                // only fall back to "whichever overlap is smaller" when
                // both axes straddle (or both fully nest) and there's no
                // such signal to go on.
                let x_nested = overlap_w >= a.width.min(b.width);
                let y_nested = overlap_h >= a.height.min(b.height);
                // X being nested means X was never the dividing line (the
                // overlap swallows one rect's whole width there), so the
                // pair must be separated on the *other* axis instead —
                // resolve on Y. Symmetrically, Y nested means resolve on
                // X. Only fall back to "whichever overlap is smaller"
                // when both axes agree (both straddle or both nest) and
                // there's no such signal to go on.
                let resolve_on_y = if x_nested != y_nested {
                    x_nested
                } else {
                    overlap_w > overlap_h
                };
                if !resolve_on_y {
                    let (left, right) = if a.x <= b.x { (i, j) } else { (j, i) };
                    // The real penetration depth on this axis — how far
                    // left's right edge extends past right's left edge —
                    // not `overlap_w`. They agree in the ordinary partial
                    // straddle, but when right is nested entirely inside
                    // left's own extent (the dwindle "uncle" shape),
                    // `overlap_w` collapses to right's own width, which
                    // undershoots badly: shrinking left by only that much
                    // barely dents a gap left's edge is still nowhere
                    // near closing, so the "overlap" barely shrinks pass
                    // after pass instead of resolving.
                    let depth = (geometries[left].1.x + geometries[left].1.width
                        - geometries[right].1.x)
                        .max(0);
                    let left_min = clients[geometries[left].0].min_width.max(0);
                    let right_min = clients[geometries[right].0].min_width.max(0);
                    let left_margin = (geometries[left].1.width - left_min).max(0);
                    let mut remaining = depth;
                    let shrink_left = remaining.min(left_margin);
                    geometries[left].1.width -= shrink_left;
                    remaining -= shrink_left;
                    if remaining > 0 {
                        let right_margin = (geometries[right].1.width - right_min).max(0);
                        let shrink_right = remaining.min(right_margin);
                        geometries[right].1.x += shrink_right;
                        geometries[right].1.width -= shrink_right;
                    }
                } else {
                    let (top, bottom) = if a.y <= b.y { (i, j) } else { (j, i) };
                    // See the X-axis branch's comment: the true
                    // penetration depth, not `overlap_h`, which
                    // undershoots the same way when bottom nests
                    // entirely inside top's own extent.
                    let depth = (geometries[top].1.y + geometries[top].1.height
                        - geometries[bottom].1.y)
                        .max(0);
                    let top_min = clients[geometries[top].0].min_height.max(0);
                    let bottom_min = clients[geometries[bottom].0].min_height.max(0);
                    let top_margin = (geometries[top].1.height - top_min).max(0);
                    let mut remaining = depth;
                    let shrink_top = remaining.min(top_margin);
                    geometries[top].1.height -= shrink_top;
                    remaining -= shrink_top;
                    if remaining > 0 {
                        let bottom_margin = (geometries[bottom].1.height - bottom_min).max(0);
                        let shrink_bottom = remaining.min(bottom_margin);
                        geometries[bottom].1.y += shrink_bottom;
                        geometries[bottom].1.height -= shrink_bottom;
                    }
                }
            }
        }
        if !any_resolved {
            break;
        }
    }
}

/// Partitions `geometries` into the vertical-stack columns a tile-family
/// layout produced them in: rects belong to the same column when they
/// share a width and their x-ranges overlap. Returns each column's
/// member indices (into `geometries`), keyed by an arbitrary but stable
/// group id.
///
/// An exact `x` match would miss a real column: `grid` centres the one
/// odd cell of an incomplete last row across the whole work area
/// (`tgmix`'s stack half hands its clients straight to `grid`), which
/// nudges that cell a few pixels sideways from the column above it —
/// same width, still clearly the same column visually, but no longer
/// the same `x`. Overlap is transitive (grouped via union-find) so a
/// column of 3+ rects still merges correctly even if only consecutive
/// rows overlap pairwise.
///
/// Same width and *any* x-overlap alone isn't enough: `center_tile`'s
/// left and right stacks routinely land at the exact same height by
/// simple 50/50 arithmetic (splitting a monitor's height two equal
/// ways twice gives the same numbers both times), which would satisfy
/// [`stack_rows`]'s own "same height, y-overlaps" test despite an
/// entire master column sitting between them — two completely
/// unrelated stacks, not a row. A real column's members are adjacent —
/// separated by nothing wider than a normal inner gap — so this also
/// requires the y-gap between the two rects (0 when their y-ranges
/// already overlap) to be no more than the larger of their two
/// heights, ruling out two rects that only coincidentally share a
/// width with an unrelated block of on-screen space between them.
fn stack_columns(
    geometries: &[(usize, crate::layout::Rect)],
) -> std::collections::BTreeMap<usize, Vec<usize>> {
    let n = geometries.len();
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }

    for i in 0..n {
        for j in (i + 1)..n {
            let a = geometries[i].1;
            let b = geometries[j].1;
            let same_width = a.width == b.width;
            let x_overlaps = a.x < b.x + b.width && b.x < a.x + a.width;
            let y_gap = if a.y + a.height <= b.y {
                b.y - (a.y + a.height)
            } else if b.y + b.height <= a.y {
                a.y - (b.y + b.height)
            } else {
                0
            };
            let adjacent = y_gap <= a.height.max(b.height);
            if same_width && x_overlaps && adjacent {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }

    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    groups
}

/// [`stack_columns`]'s transpose: partitions `geometries` into the
/// horizontal rows a layout produced them in — rects belong to the same
/// row when they share a height, their y-ranges overlap, and (the same
/// adjacency requirement `stack_columns` needs — see its doc comment)
/// the x-gap between them is no more than the larger of their two
/// widths, so two rects that only coincidentally share a height with an
/// unrelated block of on-screen space between them — `center_tile`'s
/// left and right stacks, mirrored to the same heights by construction
/// — are never treated as one row.
fn stack_rows(
    geometries: &[(usize, crate::layout::Rect)],
) -> std::collections::BTreeMap<usize, Vec<usize>> {
    let n = geometries.len();
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }

    for i in 0..n {
        for j in (i + 1)..n {
            let a = geometries[i].1;
            let b = geometries[j].1;
            let same_height = a.height == b.height;
            let y_overlaps = a.y < b.y + b.height && b.y < a.y + a.height;
            let x_gap = if a.x + a.width <= b.x {
                b.x - (a.x + a.width)
            } else if b.x + b.width <= a.x {
                a.x - (b.x + b.width)
            } else {
                0
            };
            let adjacent = x_gap <= a.width.max(b.width);
            if same_height && y_overlaps && adjacent {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }

    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    groups
}

impl MargoState {
    pub fn arrange_all(&mut self) {
        for mon_idx in 0..self.monitors.len() {
            self.arrange_monitor(mon_idx);
        }
        self.mark_state_dirty();
        self.publish_a11y_window_list();
    }

    /// Arrange just the listed monitors. Used by `open_overview` and
    /// `close_overview` so a multi-monitor setup doesn't pay the cost
    /// of re-laying out outputs that didn't flip overview state. Skips
    /// out-of-range indices defensively — the caller is the same
    /// process that built the list, but `monitors` can shrink under us
    /// during multi-output hot-unplug and we don't want to panic mid-
    /// arrange.
    pub fn arrange_monitors(&mut self, indices: &[usize]) {
        for &idx in indices {
            if idx < self.monitors.len() {
                self.arrange_monitor(idx);
            }
        }
        self.mark_state_dirty();
        self.publish_a11y_window_list();
    }

    /// The tiled-strip position that focus-following layouts (scroller,
    /// vertical scroller, deck) should centre on.
    ///
    /// Normally it is the focused window's own slot. But when focus sits on
    /// a surface that is *not* in the tiled strip — a floating keyring /
    /// polkit dialog, a launcher layer surface, a scratchpad — the naive
    /// lookup returns `None`, and every consumer falls back to slot 0. That
    /// made scroller snap the whole strip to the *first* window the instant a
    /// transient floating dialog grabbed focus, sliding the layout out from
    /// under the window the user was working in — which is exactly why a
    /// keyring prompt that we centre over window 2 ended up sitting over
    /// window 1. Instead, hold the strip on the most-recently-focused window
    /// that is *still tiled*, read from the per-monitor MRU `focus_history`
    /// (most-recent first). Falls back to `None` (→ slot 0) only when nothing
    /// in the history is tiled, e.g. a fresh tag.
    pub(crate) fn focused_tiled_pos(
        &self,
        mon_idx: usize,
        tiled: &[usize],
        focused_idx: Option<usize>,
    ) -> Option<usize> {
        if let Some(pos) = focused_idx.and_then(|fi| tiled.iter().position(|&idx| idx == fi)) {
            return Some(pos);
        }
        let mon = self.monitors.get(mon_idx)?;
        mon.focus_history
            .iter()
            .find_map(|&id| tiled.iter().position(|&idx| self.clients[idx].id == id))
    }

    /// Auto-float / re-tile clients for the `Floating` layout. Runs at
    /// the top of every `arrange_monitor` pass (issue #1).
    ///
    /// On a tag whose layout is `Floating`, every governed tiled
    /// client is switched to floating (`floated_by_layout` marks it as
    /// ours) and given a cascaded `float_geom`; the existing
    /// float-apply pass later in `arrange_monitor` then writes that to
    /// `geom`. On any other layout, clients we previously auto-floated
    /// are returned to the tiled set. Clients the user floated by hand
    /// (`is_floating && !floated_by_layout`) are never touched.
    /// Idempotent: a second pass with no state change is a no-op.
    fn reconcile_floating_layout(&mut self, mon_idx: usize) {
        let Some(mon) = self.monitors.get(mon_idx) else {
            return;
        };
        if mon.is_overview {
            return;
        }
        let curtag = mon.pertag.curtag;
        let is_floating_layout =
            mon.pertag.ltidxs.get(curtag).copied() == Some(crate::layout::LayoutId::Floating);
        let tagset = mon.current_tagset();
        let work_area = mon.work_area;

        // Clients on this monitor, visible on the current tagset, that
        // the floating layout governs. Excludes fullscreen (the
        // fullscreen override wins above `is_floating`), scratchpad /
        // overlay (own visibility model), and not-yet-mapped / dying /
        // hidden-group-member clients.
        let governed: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.monitor == mon_idx
                    && !c.is_initial_map_pending
                    && c.is_visible_on(mon_idx, tagset)
                    && c.fullscreen_mode == FullscreenMode::Off
                    && !c.is_in_scratchpad
                    && !c.is_named_scratchpad
                    && !c.is_overlay
                    && !c.is_minimized
                    && !c.is_killing
                    && !c.is_hidden_group_member()
            })
            .map(|(i, _)| i)
            .collect();

        if is_floating_layout {
            // A tabbed group is a tiling construct — dissolve any on
            // this tag before floating its members.
            let gids: std::collections::BTreeSet<u32> = governed
                .iter()
                .filter_map(|&i| self.clients[i].group_id)
                .collect();
            for gid in gids {
                self.dissolve_group(gid);
            }

            // Cascade slots already consumed by floating clients on
            // this tag (recomputed each pass → stable / idempotent).
            let mut cascade_index = governed
                .iter()
                .filter(|&&i| self.clients[i].is_floating && self.clients[i].float_geom.width != 0)
                .count();

            for &i in &governed {
                let needs_geom = self.clients[i].float_geom.width == 0;
                // Claim this client if it isn't floating yet (a tiled
                // client entering Floating), or if it *is* floating but
                // under Mosaic's ownership (a tag that just switched
                // Mosaic -> Floating) — either way it becomes ours. A
                // client already ours, or one the user hand-floated
                // (`is_floating && !floated_by_layout`), is left alone.
                let owned_by_other = self.clients[i].floated_by_layout
                    && self.clients[i].auto_float_owner != Some(crate::layout::LayoutId::Floating);
                if !self.clients[i].is_floating || owned_by_other {
                    self.clients[i].is_floating = true;
                    self.clients[i].floated_by_layout = true;
                    self.clients[i].auto_float_owner = Some(crate::layout::LayoutId::Floating);
                }
                if needs_geom {
                    let c = &self.clients[i];
                    let preferred = if c.geom.width > 0 && c.geom.height > 0 {
                        Some((c.geom.width, c.geom.height))
                    } else {
                        None
                    };
                    let seed = crate::layout::place_floating_cascade(
                        work_area,
                        preferred,
                        (c.min_width, c.min_height),
                        (c.max_width, c.max_height),
                        cascade_index,
                    );
                    self.clients[i].float_geom = seed;
                    cascade_index += 1;
                }
                self.clients[i].geom = self.clients[i].float_geom;
            }
        } else {
            for &i in &governed {
                if self.clients[i].floated_by_layout
                    && self.clients[i].auto_float_owner == Some(crate::layout::LayoutId::Floating)
                {
                    self.clients[i].is_floating = false;
                    self.clients[i].floated_by_layout = false;
                    self.clients[i].auto_float_owner = None;
                    // `float_geom` is left intact: switching back to
                    // Floating restores each window to its last spot.
                }
            }
        }
    }

    /// The lowest-numbered tag (1..=`MAX_TAGS`) on `mon_idx` with no client
    /// tagged onto it — a global/sticky client (`tags == u32::MAX`)
    /// occupies every tag, so it rules all of them out. `exclude` (the
    /// tag being evicted *from*) is skipped even if it would otherwise
    /// qualify; there's no point "moving" a window to the tag it's
    /// already on. `None` when every tag already has something.
    fn find_empty_tag(&self, mon_idx: usize, exclude: usize) -> Option<usize> {
        (1..=crate::layout::MAX_TAGS).find(|&tag| {
            if tag == exclude {
                return false;
            }
            let bit = 1u32 << (tag - 1);
            !self
                .clients
                .iter()
                .any(|c| c.monitor == mon_idx && (c.tags & bit) != 0)
        })
    }

    /// Auto-float / re-pack clients for the `Mosaic` layout — GNOME's
    /// content-aware self-arranging desktop
    /// (<https://blogs.gnome.org/tbernard/2023/07/26/rethinking-window-management/>).
    /// Mirrors `reconcile_floating_layout`'s shape closely (both hand
    /// governed clients to `is_floating`/`float_geom`, both leave
    /// user-hand-floated windows alone, both idempotent no-ops on a
    /// change-free pass) but hands off to a different placement algorithm
    /// ([`crate::layout::mosaic_arrange`]) and, crucially, *re-packs every
    /// governed client together on every pass* rather than only seeding
    /// brand-new ones — that's what makes existing windows move aside /
    /// shrink when a new one opens, and re-expand when one closes.
    ///
    /// `auto_float_owner` (not just `floated_by_layout`) gates every read
    /// and write here so this never fights `reconcile_floating_layout` over
    /// a client when a tag switches between the two layouts.
    fn reconcile_mosaic_layout(&mut self, mon_idx: usize) {
        let Some(mon) = self.monitors.get(mon_idx) else {
            return;
        };
        if mon.is_overview {
            return;
        }
        let curtag = mon.pertag.curtag;
        let is_mosaic_layout =
            mon.pertag.ltidxs.get(curtag).copied() == Some(crate::layout::LayoutId::Mosaic);
        let tagset = mon.current_tagset();
        let work_area = mon.work_area;
        let gaps = crate::layout::GapConfig {
            gappih: if self.enable_gaps { mon.gappih } else { 0 },
            gappiv: if self.enable_gaps { mon.gappiv } else { 0 },
            gappoh: if self.enable_gaps { mon.gappoh } else { 0 },
            gappov: if self.enable_gaps { mon.gappov } else { 0 },
        };

        // Same governed-set contract as `reconcile_floating_layout`.
        let governed: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.monitor == mon_idx
                    && !c.is_initial_map_pending
                    && c.is_visible_on(mon_idx, tagset)
                    && c.fullscreen_mode == FullscreenMode::Off
                    && !c.is_in_scratchpad
                    && !c.is_named_scratchpad
                    && !c.is_overlay
                    && !c.is_minimized
                    && !c.is_killing
                    && !c.is_hidden_group_member()
            })
            .map(|(i, _)| i)
            .collect();

        if is_mosaic_layout {
            // A tabbed group is a tiling construct — dissolve any on this
            // tag before mosaic-floating its members (same reasoning as
            // `reconcile_floating_layout`).
            let gids: std::collections::BTreeSet<u32> = governed
                .iter()
                .filter_map(|&i| self.clients[i].group_id)
                .collect();
            for gid in gids {
                self.dissolve_group(gid);
            }

            for &i in &governed {
                // Claim this client if it isn't floating yet, or if it's
                // floating under Floating's ownership (a tag that just
                // switched Floating -> Mosaic) — see the matching comment
                // in `reconcile_floating_layout`.
                let owned_by_other = self.clients[i].floated_by_layout
                    && self.clients[i].auto_float_owner != Some(crate::layout::LayoutId::Mosaic);
                if !self.clients[i].is_floating || owned_by_other {
                    self.clients[i].is_floating = true;
                    self.clients[i].floated_by_layout = true;
                    self.clients[i].auto_float_owner = Some(crate::layout::LayoutId::Mosaic);
                }
            }

            // Pack only the clients Mosaic itself owns. A window the user
            // hand-floated before this tag ever became Mosaic
            // (`is_floating && !floated_by_layout`) keeps its own spot,
            // same as `reconcile_floating_layout`. A client mid interactive
            // move/resize is *also* excluded — otherwise this pass would
            // immediately snap it back to its packed slot on every motion
            // tick, fighting the user's own drag (see `interactive_grab`'s
            // doc comment).
            let mut packable: Vec<crate::layout::MosaicClient> = governed
                .iter()
                .copied()
                .filter(|&i| {
                    self.clients[i].floated_by_layout
                        && self.clients[i].auto_float_owner == Some(crate::layout::LayoutId::Mosaic)
                        && !self.clients[i].interactive_grab
                        && !self.clients[i].is_mosaic_stacked
                })
                .map(|i| {
                    let c = &self.clients[i];
                    crate::layout::MosaicClient {
                        index: i,
                        id: c.id,
                        // Deliberately never sourced from this client's own
                        // `geom` — that's whatever layout happened to be
                        // active on this tag a moment ago (Tile's lopsided
                        // master/stack split, Scroller's near-full-width
                        // columns, ...), so it made Mosaic's placement
                        // depend on tiling history instead of being its own
                        // consistent standard. `(0, 0)` always takes
                        // `pack_rows`'s fallback fraction of the *current*
                        // work area instead, clamped by this client's own
                        // (layout-independent) min/max size hints — same
                        // input, same output, no matter what tag/layout
                        // this client was on a second ago.
                        ideal: (0, 0),
                        min: (c.min_width, c.min_height),
                        max: (c.max_width, c.max_height),
                    }
                })
                .collect();

            // Overflow handling: even shrinking every client to its own
            // minimum still doesn't fit — GNOME's "windows that don't fit
            // move to a new workspace" (see `mosaic_overflow_client`'s doc
            // comment). Two strategies, chosen by `mosaic_overflow_stack`:
            // move the offender to an empty tag (default — margo's fixed
            // tag set stands in for infinite dynamic workspaces), or keep
            // it on this tag shrunk to a small corner "peek"
            // (PaperWM-style always-reachable stack). Either way it drops
            // out of `packable` for *this* pass, so `mosaic_arrange` below
            // never sees the client that just left normal packing.
            if self.config.mosaic_auto_overflow_tag
                && let Some(evict_id) =
                    crate::layout::mosaic_overflow_client(work_area, &gaps, &packable)
                && let Some(evict_idx) = self.clients.iter().position(|c| c.id == evict_id)
            {
                if self.config.mosaic_overflow_stack {
                    self.clients[evict_idx].is_mosaic_stacked = true;
                    packable.retain(|c| self.clients[c.index].id != evict_id);
                    self.mark_state_dirty();
                } else if let Some(dest_tag) = self.find_empty_tag(mon_idx, curtag) {
                    let dest_bit = 1u32 << (dest_tag - 1);
                    self.animate_tag_departure(evict_idx);
                    self.clients[evict_idx].old_tags = self.clients[evict_idx].tags;
                    self.clients[evict_idx].is_tag_switching = true;
                    self.clients[evict_idx].animation.running = false;
                    self.clients[evict_idx].tags = dest_bit;
                    packable.retain(|c| self.clients[c.index].id != evict_id);
                    if !self.clients[evict_idx].is_visible_on(mon_idx, tagset) {
                        self.focus_first_visible_or_clear(mon_idx);
                    }
                    self.mark_state_dirty();
                }
            }

            // Position every currently-stacked client (existing ones from
            // a prior pass plus any just marked above) along the work
            // area's bottom-right corner, ascending by id so the row
            // stays stable and gap-free as clients stack/un-stack (ids
            // are monotonic creation order, so this never reshuffles
            // relative to itself the way re-deriving order from `governed`
            // each pass could).
            let mut stacked: Vec<usize> = governed
                .iter()
                .copied()
                .filter(|&i| self.clients[i].is_mosaic_stacked)
                .collect();
            stacked.sort_by_key(|&i| self.clients[i].id);
            for (stack_index, &i) in stacked.iter().enumerate() {
                let peek =
                    crate::layout::mosaic_stack_peek_rect(work_area, &gaps, stack_index as i32);
                self.clients[i].float_geom = peek;
                self.clients[i].geom = peek;
            }

            let placed = crate::layout::mosaic_arrange(work_area, &gaps, &packable);
            for (i, rect) in placed {
                self.clients[i].float_geom = rect;
                self.clients[i].geom = rect;
            }
        } else {
            for &i in &governed {
                if self.clients[i].floated_by_layout
                    && self.clients[i].auto_float_owner == Some(crate::layout::LayoutId::Mosaic)
                {
                    self.clients[i].is_floating = false;
                    self.clients[i].floated_by_layout = false;
                    self.clients[i].auto_float_owner = None;
                    self.clients[i].is_mosaic_stacked = false;
                }
            }
        }
    }

    pub fn arrange_monitor(&mut self, mon_idx: usize) {
        let _span = tracy_client::span!("arrange_monitor");
        if mon_idx >= self.monitors.len() {
            return;
        }
        // Soft-disabled monitor: don't lay out — clients have already
        // been migrated off, and laying out against a panel that isn't
        // being rendered just produces stale geometry.
        if !self.monitors[mon_idx].enabled {
            return;
        }

        // Preserve the old output footprint before any map/unmap/move below.
        // A floating window (or a window moving between monitors) can overlap
        // more than its owner output; both the old and new footprints must be
        // repainted or the neighbouring output keeps stale pixels.
        let mut repaint_outputs = vec![self.monitors[mon_idx].output.clone()];
        for client in self
            .clients
            .iter()
            .filter(|client| client.monitor == mon_idx)
        {
            for output in self.space.outputs_for_element(&client.window) {
                if !repaint_outputs.contains(&output) {
                    repaint_outputs.push(output);
                }
            }
        }

        // Adaptive layout: when `Config::auto_layout` is on AND the
        // user hasn't explicitly picked a layout for the current tag
        // (`pertag.user_picked_layout[curtag]` sticky bit), pick a
        // layout based on the visible-client count and the monitor's
        // aspect ratio. Sets `pertag.ltidxs[curtag]` *before* we read
        // it for `layout` below, so a single arrange pass picks up
        // the new value naturally.
        if self.config.auto_layout && !self.monitors[mon_idx].is_overview {
            self.maybe_apply_adaptive_layout(mon_idx);
        }

        // Floating layout: auto-float / re-tile the current tag's
        // clients before `tiled` is built below (issue #1).
        self.reconcile_floating_layout(mon_idx);
        // Mosaic layout: same idea, content-aware packing instead of a
        // fixed grid — see `reconcile_mosaic_layout`.
        self.reconcile_mosaic_layout(mon_idx);

        let mon = &self.monitors[mon_idx];
        let is_overview = mon.is_overview;
        // Overview path: a single Grid arrangement over the
        // (already-zoomed) work area, holding every tag's clients
        // simultaneously. Mango/Hypr-style geometric continuity —
        // each window keeps a deterministic spot in the thumbnail,
        // and the keyboard-first MRU navigation
        // (`overview_focus_next/prev`) cycles through them with
        // focus + border tracking the selection.
        let mut layout = if is_overview {
            crate::layout::LayoutId::Grid
        } else {
            mon.current_layout()
        };
        let tagset = if is_overview {
            !0
        } else {
            mon.current_tagset()
        };
        let nmaster = mon.current_nmaster();
        let mfact = mon.current_mfact();
        let monitor_area = mon.monitor_area;
        // Apply `overview_zoom` to the work area so the overview Grid
        // arranges every visible window inside a *centered* sub-rect
        // smaller than the full work area — niri's "zoom 0.5" feeling
        // without a true scene-tree transform. Centering keeps the
        // overview rect inside the layer-shell exclusion zone, so the
        // bar and other top/overlay layers stay anchored to the panel
        // edges (niri pattern: top + overlay layers stay at 1.0,
        // background + bottom would zoom in lock-step — margo doesn't
        // depend on the latter today, so we only zoom the workspace
        // surface).
        let work_area = if is_overview {
            let zoom = self.config.overview_zoom.clamp(0.1, 1.0) as f64;
            let wa = mon.work_area;
            let new_w = ((wa.width as f64) * zoom).round() as i32;
            let new_h = ((wa.height as f64) * zoom).round() as i32;
            let dx = (wa.width - new_w) / 2;
            let dy = (wa.height - new_h) / 2;
            crate::layout::Rect {
                x: wa.x + dx,
                y: wa.y + dy,
                width: new_w.max(1),
                height: new_h.max(1),
            }
        } else {
            mon.work_area
        };
        let mut gaps = if is_overview {
            let inner = self.config.overview_gap_inner.max(0);
            let outer = self.config.overview_gap_outer.max(0);
            layout::GapConfig {
                gappih: inner,
                gappiv: inner,
                gappoh: outer,
                gappov: outer,
            }
        } else {
            layout::GapConfig {
                gappih: if self.enable_gaps { mon.gappih } else { 0 },
                gappiv: if self.enable_gaps { mon.gappiv } else { 0 },
                gappoh: if self.enable_gaps { mon.gappoh } else { 0 },
                gappov: if self.enable_gaps { mon.gappov } else { 0 },
            }
        };
        let visible_in_pass = |c: &MargoClient| {
            // Skip clients that haven't gone through their deferred
            // initial map yet — they exist in `self.clients` but
            // haven't been placed in `space` and don't have rules
            // applied. Including them in arrange would map them at
            // the layout's default position, which is exactly the
            // pre-rule flicker we deferred to avoid.
            !c.is_initial_map_pending
                && c.is_visible_on(mon_idx, tagset)
                && (!is_overview || (!c.is_minimized && !c.is_killing && !c.is_in_scratchpad))
        };

        let tiled: Vec<usize> = if is_overview {
            // In overview, the visual cell order should match what
            // alt+Tab walks — so the user can read the grid as
            // "left = most-recently-touched, right = older" (or tag
            // 1-9 / mixed, depending on `overview_cycle_order`).
            // Re-uses the same ordering the cycle path computes.
            self.overview_visible_clients_for_monitor(mon_idx)
        } else {
            self.clients
                .iter()
                .enumerate()
                .filter(|(_, c)| visible_in_pass(c) && c.is_tiled())
                .map(|(i, _)| i)
                .collect()
        };

        let scroller_proportions: Vec<f32> = tiled
            .iter()
            .map(|&i| self.clients[i].scroller_proportion)
            .collect();
        let focused_tiled_pos = self.focused_tiled_pos(mon_idx, &tiled, self.focused_client_idx());

        if !is_overview
            && layout != crate::layout::LayoutId::Floating
            && self.config.smartgaps
            && tiled.len() <= 1
        {
            // Collapse the OUTER gaps for a lone window — but not all the way to
            // 0. Window borders are drawn OUTSET (`render_element_for_client`
            // grows the border rect by `border_width` beyond `c.geom` on every
            // side), so the content must sit far enough inside the work area for
            // the whole border ring to land *within* it. At gap 0 the content is
            // flush to the work-area edge and the outset border spills past it.
            //
            // The two axes need DIFFERENT clamps:
            //
            //  * Left/right (`gappoh`): the work area is flush with the
            //    monitor's physical edges (no side bars). A plain `borderpx`
            //    lands the border's OUTER edge exactly on the screen edge — in a
            //    full-width tile that reads as the border being clipped off the
            //    monitor. Clamp to `2 * borderpx` so the outset border keeps a
            //    `borderpx` margin inside the screen.
            //
            //  * Top/bottom (`gappov`): the work area is already inset from the
            //    monitor by the bar(s), so the border can never reach the screen
            //    edge here. Use just `borderpx` so the outset border's outer edge
            //    lands exactly on the work-area edge, flush against the bar.
            //    `2 * borderpx` would instead leave a `borderpx`-wide strip of
            //    wallpaper between the bar and the border — unnoticeable
            //    left/right (it abuts the bezel) but an obvious gap top/bottom
            //    against the dark bar.
            let bw = self.config.borderpx as i32;
            gaps.gappoh = 2 * bw;
            gaps.gappov = bw;
        }

        // `monly` (port of oniri): when a tag holds exactly one tiled window,
        // maximise it — arrange as Monocle regardless of the active layout, so
        // the lone window fills the work area even in column layouts like
        // scroller (where it would otherwise keep its column width). Pairs with
        // `smartgaps` above, which drops the outer gaps for a single window.
        if !is_overview
            && layout != crate::layout::LayoutId::Floating
            && self.config.monly
            && tiled.len() == 1
        {
            layout = crate::layout::LayoutId::Monocle;
        }

        let ctx = layout::ArrangeCtx {
            work_area,
            tiled: &tiled,
            nmaster,
            mfact,
            gaps: &gaps,
            scroller_proportions: &scroller_proportions,
            default_scroller_proportion: self.config.scroller_default_proportion,
            focused_tiled_pos,
            scroller_structs: self.config.scroller_structs,
            scroller_focus_center: self.config.scroller_focus_center,
            scroller_prefer_center: self.config.scroller_prefer_center,
            scroller_prefer_overspread: self.config.scroller_prefer_overspread,
        };

        // Overview path — mango-ext pattern (`overview(m) { grid(m); }`).
        // Above we forced `layout = Grid` and `tagset = !0` when
        // `is_overview`, and the `tiled` filter at line ~2977 admits
        // floating clients in overview too. So a single Grid arrange
        // over every visible window produces the right shape: 1 window
        // ≈ 90%×90% centred, 2 → side-by-side halves, 4 → 2×2 quarters,
        // 9 → 3×3 evenly. Cells shrink as window count grows, which is
        // the natural Mango/Hypr feel — no fixed 3×3 per-tag thumbnails.
        let mut geometries: Vec<(usize, crate::layout::Rect)> = layout::arrange(layout, &ctx);
        // Floor every layout rect to a positive size. A pathological gap
        // config (e.g. a large `gappov` on a short work area) can drive a
        // master-stack layout's computed width/height negative; a negative
        // size is a protocol error at xdg configure (and corrupts
        // border/hit-test math), while the window-rule clamp below only
        // runs for clients that declare min/max. This is the single
        // choke-point every one of the 11 layouts flows through.
        //
        // Apply per-client size constraints from window rules / the
        // client's own xdg_toplevel min/max request. The layout algorithm
        // is constraint-agnostic; we clamp post-hoc. max_width/max_height
        // only ever shrink a single client in place — shrinking never
        // creates room another client needs, so there's nothing to
        // redistribute. min_width/min_height are handled separately,
        // right below: growing one client to satisfy its own minimum
        // must not silently grow its whole row/column past the span the
        // layout gave it as a whole.
        for (client_idx, rect) in &mut geometries {
            rect.width = rect.width.max(1);
            rect.height = rect.height.max(1);
            let c = &self.clients[*client_idx];
            if c.max_width > 0 || c.max_height > 0 {
                clamp_size(
                    &mut rect.width,
                    &mut rect.height,
                    0,
                    0,
                    c.max_width,
                    c.max_height,
                );
            }
        }
        // Column/row membership is a structural fact about the layout
        // algorithm's own pristine output, computed once here and
        // threaded through every pass below (sync, both floor passes,
        // the work-area clamp, and the final overlap resolver). Letting
        // each pass instead recompute `stack_columns`/`stack_rows` from
        // whatever the *previous* pass had already changed made them
        // disagree about who's even in the same column/row — a member
        // one pass grew broke the exact-match another pass needed to
        // recognise its group at all.
        let column_groups = stack_columns(&geometries);
        let row_groups = stack_rows(&geometries);
        apply_min_height_floors(&mut geometries, &self.clients, &column_groups);
        // Scroller's columns are *meant* to grow past their neighbours —
        // a wider member reflows the strip via panning, exactly what the
        // pre-existing width clamp (now folded into `apply_min_width_floors`
        // for every other layout) always allowed. Redistributing width
        // there would fight that design, shrinking every other column to
        // keep a wide one on-screen instead of letting the strip pan.
        if layout != crate::layout::LayoutId::Scroller {
            let effective_min_width =
                effective_min_widths(&geometries, &self.clients, layout, &column_groups);
            apply_min_width_floors(&mut geometries, &effective_min_width, &row_groups);
        } else {
            for (client_idx, rect) in &mut geometries {
                let min_w = self.clients[*client_idx].min_width;
                if min_w > rect.width {
                    rect.width = min_w;
                }
            }
        }
        clamp_to_work_area(
            &mut geometries,
            work_area,
            layout,
            &column_groups,
            &row_groups,
        );
        resolve_residual_overlaps(
            &mut geometries,
            &self.clients,
            layout,
            &column_groups,
            &row_groups,
        );

        let now = crate::utils::now_ms();
        // gid → active group member's TARGET slot rect, filled during the
        // loop below and consumed by the hidden-member pre-size pass after
        // it (kills the tab-switch wallpaper flash).
        let mut group_slots: std::collections::HashMap<u32, crate::layout::Rect> =
            std::collections::HashMap::new();
        for (client_idx, mut rect) in geometries {
            // Tabbed group: reserve the tab strip's height at the TOP of the
            // tile and shrink the window content to match, so the strip sits
            // INSIDE the window's allocation (a title-bar band above the
            // content) instead of floating in the gap above it — where it slid
            // under the top bar and ate the outer gap. chip_rects draws the
            // strip at `geom.y - bar_h`, i.e. exactly this reserved band, now
            // within the work area. The shrunk rect is recorded so hidden
            // siblings match it (seamless cycling).
            if self.clients[client_idx].group_active {
                if let Some(gid) = self.clients[client_idx].group_id {
                    let bar_h = self.config.group_bar_height as i32;
                    if bar_h > 0 && rect.height > bar_h {
                        rect.y += bar_h;
                        rect.height -= bar_h;
                    }
                    group_slots.insert(gid, rect);
                }
            }
            let old = self.clients[client_idx].geom;

            // If we're already animating toward exactly this target,
            // leave the in-flight animation alone. arrange_monitor gets
            // called from many event sources (title change → window-
            // rule reapply, focus shift, output resize, scroller pan
            // recompute, …) and a long-running browser like Helium can
            // tick those off every frame while it's playing video. The
            // old behaviour was: each call saw `old != rect` (because
            // `old = c.geom` is the *interpolated* mid-flight value, not
            // the target), restarted the move animation with `initial
            // = old`, and reset `time_started = now`. Result: the
            // animation never finishes — every 16 ms it inches a few
            // pixels toward the target and then resets, producing the
            // exact 1-pixel-per-frame oscillation we kept seeing in the
            // arrange traces (-1794 → -1795 → -1794 → …).
            let already_animating_to_target = self.clients[client_idx].animation.running
                && self.clients[client_idx].animation.current == rect;

            let should_animate = self.config.animations
                && self.config.animation_duration_move > 0
                && !self.clients[client_idx].no_animation
                && !self.clients[client_idx].is_tag_switching
                && old.width > 0
                && old.height > 0
                && old != rect
                && !already_animating_to_target;

            // Diagnostic: every layout decision per visible client.
            // Fires per-client on every tag switch / move / focus
            // arrange — at INFO it floods the journal during normal
            // use (~30-60 lines/sec) and shows up as input latency
            // and journal contention. Trace level keeps it available
            // for `RUST_LOG=margo=trace` debugging without polluting
            // the steady-state log.
            let actual_geom = self.clients[client_idx].window.geometry().size;
            tracing::trace!(
                "arrange[{}]: client_idx={} old={}x{}+{}+{} slot={}x{}+{}+{} actual_buf={}x{} animate={} already_to_target={}",
                self.clients[client_idx].app_id.as_str(),
                client_idx,
                old.width,
                old.height,
                old.x,
                old.y,
                rect.width,
                rect.height,
                rect.x,
                rect.y,
                actual_geom.w,
                actual_geom.h,
                should_animate,
                already_animating_to_target,
            );
            if should_animate {
                // Animate the slot fully — both position AND size lerp
                // from `old` to `rect` over `animation_duration_move`.
                // Combined with the niri-style crossfade that runs in
                // parallel (snapshot rendered on top with fading
                // alpha, scaled to the *current* interpolated slot),
                // this gives the smooth resize transition the user
                // sees from niri/Hyprland's animated layouts: the
                // pre-resize content scales down while the post-
                // resize content fades up.
                //
                // Earlier we used to snap the size to the target on
                // frame 0 (initial.width = rect.width) so the buffer
                // and the slot would always match dimensions — but
                // that left the snapshot fixed at the new slot size
                // for the entire animation, which meant the snapshot
                // was rendered at a *different* size from the captured
                // content for 150 ms and the user saw a stretched/
                // squished version of the pre-resize image. The
                // crossfade infrastructure makes the size-snap
                // unnecessary: we always render BOTH layers at the
                // interpolated slot, and the buffer/slot mismatch on
                // the live layer is hidden under the snapshot until
                // alpha drops.
                let initial = old;
                // niri-style resize transition: if the slot size
                // changes (not just the position), flag a snapshot so
                // the next render captures the *current* surface tree
                // to a `GlesTexture`. While the move animation
                // interpolates the slot from old to new, the render
                // path draws that snapshot scaled to the live slot
                // instead of the live surface — the OLD content stays
                // pinned visually until the client (Electron, slow
                // ack) commits a buffer at the new size, which drops
                // the snapshot. Without this, Helium's 50–100 ms
                // ack-and-reflow window leaks the buffer-vs-slot
                // mismatch onto the screen.
                let slot_size_changed = old.width != rect.width || old.height != rect.height;
                if slot_size_changed && self.clients[client_idx].resize_snapshot.is_none() {
                    self.clients[client_idx].snapshot_pending = true;
                }
                // Spring retarget: if the previous animation was still
                // running, carry its per-channel velocity forward.
                // Without this, the integrator would re-start from rest
                // every time the layout reshuffled mid-flight and the
                // window would visibly hitch — the whole point of the
                // spring clock is that retargets stay continuous.
                // Bezier ignores this field; harmless if it's set.
                // Decide the animation's hard duration. With bezier
                // we honour the user's `animation_duration_move`; with
                // spring we let the physics tell us how long it'll
                // take to settle to within `epsilon` of the target,
                // capped between a sane floor and ceiling so a single
                // bad config value can't produce a 10-second slide.
                let use_spring = self
                    .config
                    .animation_clock_move
                    .eq_ignore_ascii_case("spring");
                let duration_ms = if use_spring {
                    let max_disp = ((rect.x - initial.x).abs())
                        .max((rect.y - initial.y).abs())
                        .max((rect.width - initial.width).abs())
                        .max((rect.height - initial.height).abs())
                        as f64;
                    if max_disp <= 0.5 {
                        // Already at target (sub-pixel). Take the
                        // bezier-style fallback so we still log a
                        // meaningful animation start, but the tick
                        // will settle on the very next frame.
                        self.config.animation_duration_move.max(1)
                    } else {
                        let spring = crate::animation::spring::Spring {
                            from: 0.0,
                            to: max_disp,
                            initial_velocity: 0.0,
                            params: crate::animation::spring::SpringParams::new(
                                self.config.animation_spring_damping_ratio,
                                self.config.animation_spring_stiffness,
                                0.5, // half-pixel epsilon
                            ),
                        };
                        let dur = spring
                            .clamped_duration()
                            .map(|d| d.as_millis() as u32)
                            // Pathological overdamped → fall back.
                            .unwrap_or(self.config.animation_duration_move.max(1));
                        // Clamp: 60 ms floor (one vblank), 1500 ms
                        // ceiling (anything longer is almost certainly
                        // a misconfiguration).
                        dur.clamp(60, 1500)
                    }
                } else {
                    // Overview transitions override the configured
                    // move duration with a snappier value (set by
                    // open_overview/close_overview); falls through to
                    // the user's animation_duration_move otherwise.
                    self.overview_transition_animation_ms
                        .unwrap_or(self.config.animation_duration_move)
                        .max(1)
                };
                self.clients[client_idx].animation = ClientAnimation {
                    should_animate: true,
                    running: true,
                    time_started: now,
                    last_tick_ms: now,
                    duration: duration_ms,
                    initial,
                    current: rect,
                    action: AnimationType::Move,
                    ..Default::default()
                };
                self.clients[client_idx].geom = initial;
            } else if already_animating_to_target {
                // Existing animation still converging on the right
                // target — leave its `time_started`, `initial`, and the
                // current interpolated `c.geom` exactly where they are.
            } else {
                self.clients[client_idx].animation.running = false;
                self.clients[client_idx].geom = rect;
            }
            self.clients[client_idx].is_tag_switching = false;
        }

        // Tabbed groups: pre-size every HIDDEN member to its active
        // sibling's slot. Only the active member is arranged above; the
        // hidden ones otherwise keep a stale size, so when
        // `changegroupactive` cycles to one it reconfigures from that
        // size — leaving a frame where the slot shows the wallpaper
        // before the client redraws (the flash the user reported).
        // Pinning their size means the swap shows their correctly-sized
        // last buffer instantly. Guarded on `geom != slot`, so once
        // settled this configures nothing until the slot actually moves.
        if !group_slots.is_empty() {
            for i in 0..self.clients.len() {
                if self.clients[i].monitor != mon_idx || !self.clients[i].is_hidden_group_member() {
                    continue;
                }
                let slot = self.clients[i]
                    .group_id
                    .and_then(|gid| group_slots.get(&gid).copied());
                if let Some(slot) = slot {
                    if self.clients[i].geom != slot {
                        self.clients[i].geom = slot;
                        self.configure_window_size(i, slot);
                    }
                }
            }
        }

        // Apply fullscreen / floating overrides outside overview. Overview
        // intentionally thumbnails every visible window in the grid.
        if !is_overview {
            for i in 0..self.clients.len() {
                let c = &self.clients[i];
                if c.monitor != mon_idx || !visible_in_pass(c) {
                    continue;
                }
                // Fullscreen geometry per mode:
                //   * Exclusive — full panel, bar will be suppressed
                //     by the render path so the window literally
                //     covers everything.
                //   * WorkArea  — `monitors[mon_idx].work_area`, i.e.
                //     the rect after layer-shell exclusion zones
                //     are subtracted; bar stays drawn on top.
                //   * Off       — fall through to the normal layout /
                //     floating geometry.
                match c.fullscreen_mode {
                    FullscreenMode::Exclusive => {
                        self.clients[i].geom = monitor_area;
                    }
                    FullscreenMode::WorkArea => {
                        self.clients[i].geom = work_area;
                    }
                    FullscreenMode::Off => {
                        if c.is_floating && c.float_geom.width > 0 {
                            self.clients[i].geom = self.clients[i].float_geom;
                        }
                    }
                }
            }
        }

        // Collect windows to show/hide (avoid borrow conflict during space ops)
        let visible: Vec<(Window, Rect, Rect)> = self
            .clients
            .iter()
            .filter(|c| visible_in_pass(c))
            .map(|c| {
                let configure_geom = if c.animation.running {
                    c.animation.current
                } else {
                    c.geom
                };
                (c.window.clone(), c.geom, configure_geom)
            })
            .collect();

        let hidden: Vec<Window> = self
            .clients
            .iter()
            .filter(|c| c.monitor == mon_idx && !visible_in_pass(c))
            .map(|c| c.window.clone())
            .collect();

        for w in hidden {
            self.space.unmap_elem(&w);
        }

        for (window, geom, configure_geom) in visible {
            self.space
                .map_element(window.clone(), (geom.x, geom.y), false);

            if let WindowSurface::Wayland(toplevel) = window.underlying_surface() {
                tracing::debug!(
                    "arrange: setting toplevel size {}x{}",
                    configure_geom.width,
                    configure_geom.height
                );
                toplevel.with_pending_state(|state| {
                    state.size = Some(Size::from((configure_geom.width, configure_geom.height)));
                });
                // Only send the configure if the initial configure has already
                // gone out. The initial configure must be sent during the first
                // commit (see CompositorHandler::commit).
                let initial_sent = with_states(toplevel.wl_surface(), |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                        .unwrap_or(false)
                });
                if initial_sent {
                    toplevel.send_pending_configure();
                }
            } else if let WindowSurface::X11(x11) = window.underlying_surface() {
                // X11 (XWayland) clients anchor their menus / popups / tooltips
                // to their OWN absolute position, so they must be told where the
                // compositor actually placed the toplevel. Wayland clients don't
                // need this (the compositor owns their position), but an X11
                // client left thinking it sits elsewhere opens popups detached
                // from the rendered window — the "menus open in the wrong place
                // under XWayland" bug. Mirror the scene rect into the X11 client
                // (guarded so we don't re-configure an already-correct window).
                if geom.width > 0 && geom.height > 0 {
                    let rect = smithay::utils::Rectangle::new(
                        (geom.x, geom.y).into(),
                        (geom.width, geom.height).into(),
                    );
                    if x11.geometry() != rect {
                        tracing::debug!(
                            "arrange: configuring x11 window to {}x{}+{}+{}",
                            geom.width,
                            geom.height,
                            geom.x,
                            geom.y
                        );
                        let _ = x11.configure(rect);
                    }
                }
            }
        }
        self.enforce_z_order();
        crate::border::refresh(self);
        // `map_element` starts with an empty output-membership map. Populate
        // it before the first render/callback cycle so a tag-return whose DRM
        // render is empty can still deliver the waiting wl_surface.frame to
        // the newly-visible window instead of stalling until unrelated damage.
        self.space.refresh();
        for client in self
            .clients
            .iter()
            .filter(|client| client.monitor == mon_idx)
        {
            for output in self.space.outputs_for_element(&client.window) {
                if !repaint_outputs.contains(&output) {
                    repaint_outputs.push(output);
                }
            }
        }
        for output in repaint_outputs {
            self.request_repaint_output(&output);
        }
        // mango 0.16.1 `tag_gather`: compact this monitor's occupied tags
        // to consecutive numbers after every arrange, closing gaps left by
        // closed/moved windows. Remapping only relabels tag numbers — the
        // set of clients visible right now is unchanged (the current
        // view's tag moves with its content), so the geometry just
        // computed above stays valid and this can't feed back into
        // another arrange.
        if self.config.tag_gather {
            self.tag_gather_apply(mon_idx);
        }
        // Refresh the IPC channels so `mctl clients`/`focused`/`status`
        // and any IPC `watch state` subscriber sees the new
        // windows the moment they're laid out. arrange_all already
        // covered both, but arrange_monitor (the path most map/unmap/
        // tag-move events take) didn't — leaving state snapshot + the bar
        // tag-counts stuck on the boot snapshot of zero.
        self.mark_state_dirty();
    }

    /// Pre-compute tiling geometry for the tags the **scroller overview**
    /// is about to show but that aren't currently on screen, so their
    /// windows render at their real tiled slots in the overview cells
    /// without the user having to visit each tag first.
    ///
    /// `arrange_monitor` only ever lays out a monitor's *current* tagset,
    /// so a window mapped onto an unvisited tag keeps whatever stale
    /// `geom` — and surface size — it had at map time. The scroller-
    /// overview render path reads `client.geom` directly and renders the
    /// window's live surface tree, so those windows showed crammed at
    /// their default position/size until the tag was selected once (which
    /// ran a real arrange + configure). This walks every off-screen tag
    /// the strip will show and assigns geom + sends a configure, with no
    /// animation, so the overview is correct from the first open. The
    /// active tag(s) are skipped — they're already laid out live and we
    /// don't want to disturb their in-flight animations.
    pub fn prearrange_overview_tags(&mut self) {
        for mon_idx in 0..self.monitors.len() {
            if !self.monitors[mon_idx].enabled {
                continue;
            }
            let current_tagset = self.monitors[mon_idx].current_tagset();
            for tag in self.scroller_overview_tags(mon_idx) {
                let bit = 1u32 << (tag - 1);
                if bit & current_tagset != 0 {
                    continue; // on screen now — already arranged live.
                }
                self.prearrange_overview_tag(mon_idx, tag);
            }
        }
    }

    /// Lay out a single off-screen `tag` on `mon_idx` (helper for
    /// [`Self::prearrange_overview_tags`]). Mirrors the non-overview
    /// branch of `arrange_monitor` — per-tag layout/nmaster/mfact from
    /// pertag, the same gap + smartgaps rules — but assigns geom directly
    /// (no move animation) and sends each window the configure for its
    /// slot so its buffer matches what the overview cell will scale down.
    fn prearrange_overview_tag(&mut self, mon_idx: usize, tag: usize) {
        let bit = 1u32 << (tag - 1);
        let mon = &self.monitors[mon_idx];
        let layout = mon
            .pertag
            .ltidxs
            .get(tag)
            .copied()
            .unwrap_or_else(|| mon.current_layout());
        let nmaster = mon.pertag.nmasters.get(tag).copied().unwrap_or(1);
        let mfact = mon.pertag.mfacts.get(tag).copied().unwrap_or(0.55);
        let work_area = mon.work_area;
        let monitor_area = mon.monitor_area;
        let mut gaps = layout::GapConfig {
            gappih: if self.enable_gaps { mon.gappih } else { 0 },
            gappiv: if self.enable_gaps { mon.gappiv } else { 0 },
            gappoh: if self.enable_gaps { mon.gappoh } else { 0 },
            gappov: if self.enable_gaps { mon.gappov } else { 0 },
        };

        // Tiled clients on this (mon, tag), in clients-vec order.
        let tiled: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.monitor == mon_idx
                    && (c.tags & bit) != 0
                    && !c.is_initial_map_pending
                    && c.is_tiled()
            })
            .map(|(i, _)| i)
            .collect();

        if self.config.smartgaps && tiled.len() <= 1 {
            // Keep room for the OUTSET border — see the matching guard in
            // `arrange_monitor` for the full reasoning. `2 * borderpx`
            // left/right (the work area is flush with the monitor edge there),
            // but only `borderpx` top/bottom so the border lands flush against
            // the bar with no wallpaper strip showing through.
            let bw = self.config.borderpx as i32;
            gaps.gappoh = 2 * bw;
            gaps.gappov = bw;
        }

        let scroller_proportions: Vec<f32> = tiled
            .iter()
            .map(|&i| self.clients[i].scroller_proportion)
            .collect();

        let ctx = layout::ArrangeCtx {
            work_area,
            tiled: &tiled,
            nmaster,
            mfact,
            gaps: &gaps,
            scroller_proportions: &scroller_proportions,
            default_scroller_proportion: self.config.scroller_default_proportion,
            // No live focus on an off-screen tag.
            focused_tiled_pos: None,
            scroller_structs: self.config.scroller_structs,
            scroller_focus_center: self.config.scroller_focus_center,
            scroller_prefer_center: self.config.scroller_prefer_center,
            scroller_prefer_overspread: self.config.scroller_prefer_overspread,
        };

        for (client_idx, mut rect) in layout::arrange(layout, &ctx) {
            let c = &self.clients[client_idx];
            if c.min_width > 0 || c.min_height > 0 || c.max_width > 0 || c.max_height > 0 {
                clamp_size(
                    &mut rect.width,
                    &mut rect.height,
                    c.min_width,
                    c.min_height,
                    c.max_width,
                    c.max_height,
                );
            }
            self.clients[client_idx].animation.running = false;
            self.clients[client_idx].geom = rect;
            self.configure_window_size(client_idx, rect);
        }

        // Floating / fullscreen clients on this tag — the overview
        // thumbnails them too, so give them their intended geometry as
        // well (tiled clients above already returned `None` here).
        for i in 0..self.clients.len() {
            let c = &self.clients[i];
            if c.monitor != mon_idx
                || (c.tags & bit) == 0
                || c.is_initial_map_pending
                || c.is_minimized
                || c.is_killing
                || c.is_in_scratchpad
            {
                continue;
            }
            let rect = match c.fullscreen_mode {
                FullscreenMode::Exclusive => Some(monitor_area),
                FullscreenMode::WorkArea => Some(work_area),
                FullscreenMode::Off if c.is_floating && c.float_geom.width > 0 => {
                    Some(c.float_geom)
                }
                FullscreenMode::Off => None,
            };
            if let Some(rect) = rect {
                self.clients[i].animation.running = false;
                self.clients[i].geom = rect;
                self.configure_window_size(i, rect);
            }
        }
    }

    /// Send `client_idx`'s toplevel a configure sizing it to `geom` (no
    /// position move — the overview places it via `client.geom`). Factored
    /// from `arrange_monitor`'s visible-window loop; only fires once the
    /// initial configure has gone out.
    fn configure_window_size(&mut self, client_idx: usize, geom: Rect) {
        let window = self.clients[client_idx].window.clone();
        if let WindowSurface::Wayland(toplevel) = window.underlying_surface() {
            toplevel.with_pending_state(|state| {
                state.size = Some(Size::from((geom.width, geom.height)));
            });
            let initial_sent = with_states(toplevel.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                    .unwrap_or(false)
            });
            if initial_sent {
                toplevel.send_pending_configure();
            }
        }
    }

    /// Smithay's `Space::map_element` always inserts the touched
    /// element at the top of the stack — there's no way to map at an
    /// explicit z. So every time `arrange_monitor` re-maps a tile-
    /// layer window during a layout change or a move animation, that
    /// tile silently leaps above any floating window (CopyQ,
    /// pavucontrol, picker dialogs) that happened to be on screen.
    ///
    /// To keep "floating sits on top of tiled" actually true, run
    /// this after every `map_element` storm. We re-`raise_element`
    /// floats first, then overlays/scratchpads, in `clients`-vec
    /// forward order — `raise_element` itself moves to top, so the
    /// last raise per band wins, which means the most-recently-
    /// created float of each band ends up at the top of its band
    /// (sane default for "newly opened picker shows on top").
    pub fn enforce_z_order(&mut self) {
        let floats: Vec<smithay::desktop::Window> = self
            .clients
            .iter()
            .filter(|c| (c.is_floating || c.is_in_scratchpad) && !c.is_overlay)
            .map(|c| c.window.clone())
            .collect();
        for w in &floats {
            self.space.raise_element(w, false);
        }
        // A client mid interactive move/resize must render above every
        // other float/tile it may currently overlap, no matter where it
        // sits in `clients` — `arrange_monitor` (and therefore this
        // function) runs on every motion tick of a drag, so without this
        // a resize that grows a window into a neighbour only shows on top
        // when its array index happens to be higher than the neighbour's,
        // which has nothing to do with what the user is actually
        // dragging. Raising it here, after the plain float band above,
        // wins the same last-raise-on-top race in the grabbed client's
        // favour.
        if let Some(w) = self
            .clients
            .iter()
            .find(|c| c.interactive_grab)
            .map(|c| c.window.clone())
        {
            self.space.raise_element(&w, false);
        }
        let overlays: Vec<smithay::desktop::Window> = self
            .clients
            .iter()
            .filter(|c| c.is_overlay)
            .map(|c| c.window.clone())
            .collect();
        for w in &overlays {
            self.space.raise_element(w, false);
        }
    }
}
