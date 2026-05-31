//! niri-style **scroller overview**: a zoomed-out, scrollable strip of
//! per-tag mini-desktops, each rendering its tag's real tiling layout
//! scaled down (no window resize — the scaling happens at render time
//! via `RescaleRenderElement`). Entirely separate from the classic grid
//! overview in `state/overview.rs` (`is_overview` / `overview_*`).
//!
//! Navigation is a **continuous, per-monitor scroll**: the strip pans
//! smoothly under the pointer's monitor, carries momentum after a flick,
//! rubber-bands at the ends, and springs to the nearest tag when it
//! settles. Each monitor keeps its own scroll position, and a scroll
//! targets whichever monitor the pointer is over — so two displays pan
//! independently. The physics live in [`MargoState::tick_scroller_overview`].

use super::{FocusTarget, MargoState};
use crate::layout::{MAX_TAGS, Rect};

/// One tag cell in the overview strip: which tag it shows, where it
/// renders (output-logical coords), and a `key` unique among the cells
/// in this frame (its strip index) — used to namespace the cell's render
/// elements so a tag repeated by the wrap-around loop never collides in
/// the damage tracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverviewCell {
    /// 1-based pertag tag index (1..=`MAX_TAGS`).
    pub tag: usize,
    pub rect: Rect,
    /// Per-frame unique key (strip index) for render-element namespacing.
    pub key: usize,
}

/// Lay `tags` out as a vertical strip of cells, each the `output`
/// rectangle scaled by `zoom`, separated by `gap` (post-zoom logical
/// px), horizontally centered, and offset so the fractional strip
/// position `center_pos` (0 = first cell, 1.5 = halfway between the
/// second and third, …) sits vertically centered in `output`.
///
/// When `loop_strip` is true the strip is cyclic: cells are emitted for
/// a window of indices spanning the viewport, each mapped to
/// `tags[i mod n]`, so scrolling past either end wraps seamlessly. Each
/// cell's `key` is its position in the emitted list (unique per frame).
///
/// Pure geometry — no compositor state — so it's unit-tested directly.
pub fn overview_cells(
    output: Rect,
    tags: &[usize],
    zoom: f64,
    gap: i32,
    center_pos: f64,
    loop_strip: bool,
) -> Vec<OverviewCell> {
    let n = tags.len();
    if n == 0 {
        return Vec::new();
    }
    let zoom = zoom.clamp(0.05, 1.0);
    let cell_w = (f64::from(output.width) * zoom).round() as i32;
    let cell_h = (f64::from(output.height) * zoom).round() as i32;
    let cell_x = output.x + (output.width - cell_w) / 2;
    let stride = f64::from(cell_h + gap.max(0)).max(1.0);

    // Cell at strip index `i` (i may be fractional in spirit): its top in
    // logical px so that index == center_pos lands vertically centered.
    let top = |i: f64| {
        f64::from(output.height) / 2.0 - f64::from(cell_h) / 2.0 + (i - center_pos) * stride
    };
    let make = |i: i64, key: usize| {
        let tag = tags[i.rem_euclid(n as i64) as usize];
        let y = output.y + top(i as f64).round() as i32;
        OverviewCell {
            tag,
            rect: Rect::new(cell_x, y, cell_w, cell_h),
            key,
        }
    };

    if !loop_strip {
        return (0..n).map(|i| make(i as i64, i)).collect();
    }

    // Cyclic: emit a window of indices around the centre that covers the
    // viewport (plus a margin), so wrap-around shows no seam.
    let radius = (f64::from(output.height) / stride).ceil() as i64 + 2;
    let center_i = center_pos.round() as i64;
    let lo = center_i - radius;
    let hi = center_i + radius;
    (lo..=hi).enumerate().map(|(k, i)| make(i, k)).collect()
}

/// Per-monitor continuous scroll state within an open overview.
#[derive(Debug, Clone)]
pub struct MonitorScroll {
    /// Continuous scroll position along the tag strip, in cell units
    /// (0 = first shown tag centered; fractional while panning).
    pub pos: f64,
    /// Velocity in cell-units/sec, for momentum after a flick.
    velocity: f64,
    /// `now_ms` of the last scroll delta (distinguishes "actively
    /// scrolling" from "idle", which triggers momentum + snap).
    last_scroll_ms: u32,
    /// Explicit glide target (keyboard step / end-of-momentum snap). The
    /// position springs toward it; `None` means free / momentum.
    snap_target: Option<f64>,
}

impl MonitorScroll {
    fn new(pos: f64) -> Self {
        Self {
            pos,
            velocity: 0.0,
            last_scroll_ms: 0,
            snap_target: None,
        }
    }
}

/// Live state of an open scroller overview. `Some` only while the
/// overview is open or animating closed.
#[derive(Debug, Clone)]
pub struct ScrollerOverview {
    /// Eased open progress: `0.0` = closed (normal 1× view, current tag
    /// full-screen), `1.0` = fully zoomed out to the strip.
    pub progress: f64,
    /// Animation direction: `true` while opening / open, `false` while
    /// closing (clears the `Option` once `progress` reaches 0).
    pub opening: bool,
    anim_from: f64,
    anim_started_ms: u32,
    /// `now_ms` of the previous physics tick, for the integrator's `dt`.
    last_tick_ms: u32,
    /// Per-monitor scroll state, indexed by monitor index.
    pub mon: Vec<MonitorScroll>,
}

/// Idle gap (ms) after the last scroll delta before momentum + snap kick in.
const SCROLL_IDLE_MS: u32 = 60;
/// Rubber-band spring rate (1/sec) pulling an out-of-range strip back.
const RUBBER_K: f64 = 18.0;
/// Snap spring rate (1/sec) easing the strip to the nearest tag.
const SNAP_K: f64 = 14.0;
/// Momentum friction (1/sec) — velocity decays as e^(-FRICTION·dt).
const FRICTION: f64 = 6.0;
/// Velocity (cells/sec) below which momentum stops and snapping begins.
const V_STOP: f64 = 0.6;

/// Smoothstep ease for the open/close zoom.
fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl MargoState {
    /// True while the scroller overview is open or animating.
    pub fn is_scroller_overview_open(&self) -> bool {
        self.scroller_overview.is_some()
    }

    /// Open the scroller overview. No-op if already open. Closes the
    /// classic grid overview first (the two are mutually exclusive).
    /// Seeds each monitor's scroll position on its current tag.
    pub fn open_scroller_overview(&mut self) {
        if self.scroller_overview.is_some() {
            return;
        }
        if self.is_overview_open() {
            self.close_overview(None);
        }

        let mon: Vec<MonitorScroll> = (0..self.monitors.len())
            .map(|i| {
                let tags = self.scroller_overview_tags(i);
                let cur = self
                    .monitors
                    .get(i)
                    .map(|m| m.pertag.curtag.clamp(1, MAX_TAGS))
                    .unwrap_or(1);
                let pos = tags.iter().position(|&t| t == cur).unwrap_or(0) as f64;
                MonitorScroll::new(pos)
            })
            .collect();

        let now = crate::utils::now_ms();
        self.scroller_overview = Some(ScrollerOverview {
            progress: 0.0,
            opening: true,
            anim_from: 0.0,
            anim_started_ms: now,
            last_tick_ms: now,
            mon,
        });
        self.request_repaint();
    }

    /// Close the scroller overview, committing each monitor to the tag it
    /// scrolled to (the centered cell), then zoom back in onto it. No-op
    /// if not open. The desktop is revealed on the chosen tag as the
    /// overview clears (`tick_scroller_overview` clears it at progress 0).
    pub fn close_scroller_overview(&mut self) {
        if self.scroller_overview.is_none() {
            return;
        }

        // Commit every monitor to its centered tag (no-op where unchanged).
        let focused = self.focused_monitor();
        let mut flipped: Vec<usize> = Vec::new();
        for mon_idx in 0..self.monitors.len() {
            let tags = self.scroller_overview_tags(mon_idx);
            if tags.is_empty() {
                continue;
            }
            let idx = self.scroller_centered_index(mon_idx, tags.len());
            let bit = 1u32 << (tags[idx] - 1);
            let seltags = self.monitors[mon_idx].seltags;
            if self.monitors[mon_idx].tagset[seltags] != bit {
                self.monitors[mon_idx].tagset[seltags] = bit;
                self.update_pertag_for_tagset(mon_idx, bit);
                flipped.push(mon_idx);
            }
        }
        if !flipped.is_empty() {
            self.arrange_monitors(&flipped);
            self.focus_first_visible_or_clear(focused);
        }

        // Start the zoom-back-in animation; the tick tears it down at 0.
        let now = crate::utils::now_ms();
        if let Some(ov) = self.scroller_overview.as_mut() {
            ov.opening = false;
            ov.anim_from = ov.progress;
            ov.anim_started_ms = now;
        }
        self.request_repaint();
    }

    /// Advance the open/close zoom AND the per-monitor scroll physics
    /// (momentum, rubber-band, snap-to-nearest). Returns `true` while
    /// anything is still animating, so the main loop keeps requesting
    /// frames. Clears the overview when a close settles at 0.
    pub fn tick_scroller_overview(&mut self, now_ms: u32) -> bool {
        let dur = self.config.overview_transition_ms.max(1) as f64;
        let looping = self.config.scroller_overview_loop;
        // Tag counts per monitor (for clamping), computed before the
        // mutable borrow of `scroller_overview`.
        let counts: Vec<usize> = (0..self.monitors.len())
            .map(|i| self.scroller_overview_tags(i).len())
            .collect();

        let (mut animating, teardown) = {
            let Some(ov) = self.scroller_overview.as_mut() else {
                return false;
            };

            let dt = f64::from(now_ms.wrapping_sub(ov.last_tick_ms).min(64)) / 1000.0;
            ov.last_tick_ms = now_ms;

            // Open / close zoom.
            let target = if ov.opening { 1.0 } else { 0.0 };
            let elapsed = f64::from(now_ms.wrapping_sub(ov.anim_started_ms));
            let t = (elapsed / dur).clamp(0.0, 1.0);
            ov.progress = ov.anim_from + (target - ov.anim_from) * ease(t);
            let zoom_done = t >= 1.0;
            if zoom_done {
                ov.progress = target;
            }
            let teardown = zoom_done && !ov.opening;
            let mut animating = !zoom_done;

            if !teardown {
                for (i, ms) in ov.mon.iter_mut().enumerate() {
                    let n = counts.get(i).copied().unwrap_or(0);
                    if n == 0 {
                        continue;
                    }
                    let nf = n as f64;
                    let max = nf - 1.0;
                    let idle = now_ms.wrapping_sub(ms.last_scroll_ms) > SCROLL_IDLE_MS;

                    if looping {
                        // Cyclic: no rubber-band. Glide to the snap target
                        // (then wrap into [0, n) — seamless since the strip
                        // repeats), coast on momentum otherwise, snap when slow.
                        if let Some(tgt) = ms.snap_target {
                            ms.pos += (tgt - ms.pos) * (1.0 - (-SNAP_K * dt).exp());
                            if (ms.pos - tgt).abs() < 0.004 {
                                ms.pos = tgt.rem_euclid(nf);
                                ms.snap_target = None;
                            } else {
                                animating = true;
                            }
                        } else if idle {
                            if ms.velocity.abs() > V_STOP {
                                ms.pos = (ms.pos + ms.velocity * dt).rem_euclid(nf);
                                ms.velocity *= (-FRICTION * dt).exp();
                                animating = true;
                            } else {
                                ms.velocity = 0.0;
                                let near = ms.pos.round();
                                if (ms.pos - near).abs() > 0.004 {
                                    ms.snap_target = Some(near);
                                    animating = true;
                                }
                            }
                        } else {
                            animating = true;
                        }
                        continue;
                    }

                    // Out of range → rubber-band back to the nearest bound.
                    if ms.pos < 0.0 || ms.pos > max {
                        let bound = ms.pos.clamp(0.0, max);
                        ms.pos += (bound - ms.pos) * (1.0 - (-RUBBER_K * dt).exp());
                        ms.velocity = 0.0;
                        ms.snap_target = None;
                        if (ms.pos - bound).abs() > 0.001 {
                            animating = true;
                        } else {
                            ms.pos = bound;
                        }
                        continue;
                    }

                    if let Some(tgt) = ms.snap_target {
                        // Explicit glide (keyboard step / settle).
                        ms.pos += (tgt - ms.pos) * (1.0 - (-SNAP_K * dt).exp());
                        if (ms.pos - tgt).abs() < 0.004 {
                            ms.pos = tgt;
                            ms.snap_target = None;
                        } else {
                            animating = true;
                        }
                    } else if idle {
                        // Idle after a scroll: coast on momentum, then snap.
                        if ms.velocity.abs() > V_STOP {
                            ms.pos = (ms.pos + ms.velocity * dt).clamp(0.0, max);
                            ms.velocity *= (-FRICTION * dt).exp();
                            animating = true;
                        } else {
                            ms.velocity = 0.0;
                            let near = ms.pos.round().clamp(0.0, max);
                            if (ms.pos - near).abs() > 0.004 {
                                ms.snap_target = Some(near);
                                animating = true;
                            }
                        }
                    } else {
                        // Actively scrolling — keep frames coming.
                        animating = true;
                    }
                }
            }

            (animating, teardown)
        };

        if teardown {
            self.scroller_overview = None;
            animating = false;
        }
        animating
    }

    pub fn toggle_scroller_overview(&mut self) {
        if self.is_scroller_overview_open() {
            self.close_scroller_overview();
        } else {
            self.open_scroller_overview();
        }
    }

    /// Toggle whichever overview the user picked as their preferred style
    /// (`overview_style`). What the generic `toggle_overview` keybind hits.
    pub fn toggle_overview_styled(&mut self) {
        match self.config.overview_style {
            margo_config::OverviewStyle::Scroller => self.toggle_scroller_overview(),
            margo_config::OverviewStyle::Grid => self.toggle_overview(),
        }
    }

    /// Style-aware cycle-forward (`overview_focus_next` / alt+Tab). Drives
    /// whichever overview the user selected, so only the chosen style is
    /// ever active; opens the scroller first if it's closed.
    pub fn overview_focus_next_styled(&mut self) {
        match self.config.overview_style {
            margo_config::OverviewStyle::Scroller => {
                if !self.is_scroller_overview_open() {
                    self.open_scroller_overview();
                }
                self.scroller_overview_select(1);
            }
            margo_config::OverviewStyle::Grid => self.overview_focus_next(),
        }
    }

    /// Style-aware cycle-backward (`overview_focus_prev`).
    pub fn overview_focus_prev_styled(&mut self) {
        match self.config.overview_style {
            margo_config::OverviewStyle::Scroller => {
                if !self.is_scroller_overview_open() {
                    self.open_scroller_overview();
                }
                self.scroller_overview_select(-1);
            }
            margo_config::OverviewStyle::Grid => self.overview_focus_prev(),
        }
    }

    /// Style-aware activate (`overview_activate`, e.g. alt+Tab release).
    pub fn overview_activate_styled(&mut self) {
        if self.is_scroller_overview_open() {
            self.scroller_overview_activate();
        } else if self.is_overview_open() {
            self.overview_activate();
        }
    }

    /// Tags (1-based) shown as cells for `mon_idx`: every tag with at
    /// least one mapped, non-minimized client, plus the monitor's current
    /// tag so the strip is never empty. Ascending order. Stable as you
    /// scroll (doesn't depend on the live selection).
    pub fn scroller_overview_tags(&self, mon_idx: usize) -> Vec<usize> {
        let current = self
            .monitors
            .get(mon_idx)
            .map(|m| m.pertag.curtag.clamp(1, MAX_TAGS))
            .unwrap_or(1);

        (1..=MAX_TAGS)
            .filter(|&tag| {
                tag == current
                    || self.clients.iter().any(|c| {
                        c.monitor == mon_idx
                            && (c.tags & (1 << (tag - 1))) != 0
                            && !c.is_initial_map_pending
                            && !c.is_minimized
                            && !c.is_killing
                            && !c.is_in_scratchpad
                    })
            })
            .collect()
    }

    /// 0-based tag index of the cell currently centered on `mon_idx`
    /// (the nearest tag to the continuous scroll position). Wraps when
    /// the loop option is on, else clamps to the strip ends.
    fn scroller_centered_index(&self, mon_idx: usize, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let pos = self
            .scroller_overview
            .as_ref()
            .and_then(|o| o.mon.get(mon_idx))
            .map(|m| m.pos)
            .unwrap_or(0.0);
        let r = pos.round() as i64;
        if self.config.scroller_overview_loop {
            r.rem_euclid(n as i64) as usize
        } else {
            r.clamp(0, n as i64 - 1) as usize
        }
    }

    /// Step the focused monitor's selection by `dir` (+1 next / −1 prev)
    /// with a smooth glide. Drives arrow keys and the alt+Tab cycle.
    pub fn scroller_overview_select(&mut self, dir: i32) {
        if self.scroller_overview.is_none() {
            return;
        }
        let mon = self.focused_monitor();
        let n = self.scroller_overview_tags(mon).len();
        if n == 0 {
            return;
        }
        let looping = self.config.scroller_overview_loop;
        let max = (n - 1) as f64;
        if let Some(ms) = self
            .scroller_overview
            .as_mut()
            .and_then(|ov| ov.mon.get_mut(mon))
        {
            let base = ms.snap_target.unwrap_or(ms.pos).round();
            let next = base + f64::from(dir);
            ms.snap_target = Some(if looping { next } else { next.clamp(0.0, max) });
            ms.velocity = 0.0;
        }
        self.request_repaint();
    }

    /// Feed a scroll delta (in cell units) for the monitor the pointer is
    /// over. `discrete` (mouse wheel): step cleanly to a tag with no
    /// momentum overshoot. Otherwise (touchpad finger scroll): pan the
    /// strip directly and track velocity for momentum, snapping to the
    /// nearest tag once it settles.
    pub fn scroller_overview_scroll(
        &mut self,
        mon_idx: usize,
        delta_cells: f64,
        discrete: bool,
        now_ms: u32,
    ) {
        let looping = self.config.scroller_overview_loop;
        let max = (self.scroller_overview_tags(mon_idx).len().max(1) - 1) as f64;
        if let Some(ms) = self
            .scroller_overview
            .as_mut()
            .and_then(|ov| ov.mon.get_mut(mon_idx))
        {
            if discrete {
                // Wheel: step to a whole tag, glide there, no momentum.
                // When looping, don't clamp — the tick wraps on settle.
                ms.pos += delta_cells;
                if !looping {
                    ms.pos = ms.pos.clamp(0.0, max);
                }
                ms.snap_target = Some(if looping {
                    ms.pos.round()
                } else {
                    ms.pos.round().clamp(0.0, max)
                });
                ms.velocity = 0.0;
            } else {
                let dt = f64::from(now_ms.wrapping_sub(ms.last_scroll_ms).clamp(1, 100)) / 1000.0;
                ms.pos += delta_cells;
                let inst = delta_cells / dt;
                // Smooth the velocity estimate; cap modestly so a fast
                // flick coasts a little rather than flinging across tags.
                ms.velocity = (0.6 * ms.velocity + 0.4 * inst).clamp(-6.0, 6.0);
                ms.snap_target = None;
            }
            ms.last_scroll_ms = now_ms;
        }
        self.request_repaint();
    }

    /// Activate the centered tag (Enter / alt+Tab release): same as
    /// closing — every monitor commits to the tag it scrolled to, with
    /// the zoom-back-in animation.
    pub fn scroller_overview_activate(&mut self) {
        self.close_scroller_overview();
    }

    /// Handle a left click at global-logical (`x`, `y`): find the tag cell
    /// under the cursor on its monitor, switch to that tag, and focus the
    /// specific window clicked (if any). A click on the bare backdrop
    /// closes without switching.
    pub fn scroller_overview_click(&mut self, x: f64, y: f64) {
        if self.scroller_overview.is_none() {
            return;
        }
        let contains = |r: Rect, x: f64, y: f64| {
            (x as i32) >= r.x
                && (x as i32) < r.x + r.width
                && (y as i32) >= r.y
                && (y as i32) < r.y + r.height
        };

        let mon = self
            .monitors
            .iter()
            .position(|m| contains(m.monitor_area, x, y))
            .unwrap_or_else(|| self.focused_monitor());
        let Some(area) = self.monitors.get(mon).map(|m| m.monitor_area) else {
            return;
        };

        let tags = self.scroller_overview_tags(mon);
        let pos = self
            .scroller_overview
            .as_ref()
            .and_then(|o| o.mon.get(mon))
            .map(|m| m.pos)
            .unwrap_or(0.0);
        let zoom = f64::from(self.config.scroller_overview_zoom.clamp(0.1, 1.0));
        let gap = self.config.scroller_overview_gap.max(0);
        let loop_strip = self.config.scroller_overview_loop;
        let cells = overview_cells(area, &tags, zoom, gap, pos, loop_strip);

        let Some(cell) = cells.into_iter().find(|c| contains(c.rect, x, y)) else {
            self.close_scroller_overview();
            return;
        };

        // Map the click back into output space to find the window under it.
        let cell_scale = f64::from(cell.rect.width) / f64::from(area.width.max(1));
        let out_x = f64::from(area.x) + (x - f64::from(cell.rect.x)) / cell_scale;
        let out_y = f64::from(area.y) + (y - f64::from(cell.rect.y)) / cell_scale;
        let bit = 1u32 << (cell.tag - 1);
        let clicked = self.clients.iter().position(|c| {
            c.monitor == mon
                && (c.tags & bit) != 0
                && !c.is_minimized
                && !c.is_killing
                && !c.is_in_scratchpad
                && contains(c.geom, out_x, out_y)
        });

        // Center the clicked tag on its monitor, then run the same animated
        // close as four-finger-down / Enter (`close_scroller_overview`):
        // every monitor commits to its centered tag and the strip zooms
        // back in onto it. The old path tore the overview down instantly and
        // `view_tag`'d straight there — an abrupt super+N-style switch with
        // no zoom.
        if let Some(ci) = tags.iter().position(|&t| t == cell.tag) {
            if let Some(ms) = self
                .scroller_overview
                .as_mut()
                .and_then(|o| o.mon.get_mut(mon))
            {
                ms.pos = ci as f64;
                ms.snap_target = Some(ci as f64);
                ms.velocity = 0.0;
            }
        }
        self.close_scroller_overview();

        // Prefer the specific window under the cursor over the default
        // first-visible focus `close_scroller_overview` applies.
        if let Some(idx) = clicked {
            if mon < self.monitors.len() {
                self.monitors[mon].selected = Some(idx);
            }
            let window = self.clients[idx].window.clone();
            self.focus_surface(Some(FocusTarget::Window(window)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Rect, overview_cells};

    const OUT: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn empty_tags_yields_no_cells() {
        assert!(overview_cells(OUT, &[], 0.5, 20, 0.0, false).is_empty());
    }

    #[test]
    fn cell_size_is_output_scaled_by_zoom() {
        let cells = overview_cells(OUT, &[1, 2, 3], 0.5, 20, 0.0, false);
        assert_eq!(cells.len(), 3);
        for c in &cells {
            assert_eq!(c.rect.width, 960); // 1920 * 0.5
            assert_eq!(c.rect.height, 540); // 1080 * 0.5
        }
        // Horizontally centered: (1920 - 960) / 2 = 480.
        assert_eq!(cells[0].rect.x, 480);
    }

    #[test]
    fn centered_index_is_vertically_centered() {
        // center_pos 1.0 centers the second cell (index 1) on 540.
        let cells = overview_cells(OUT, &[1, 2, 3], 0.5, 20, 1.0, false);
        let c = cells[1];
        let center = c.rect.y + c.rect.height / 2;
        assert_eq!(center, 540);
    }

    #[test]
    fn fractional_pos_sits_between_cells() {
        // center_pos 0.5 puts the midpoint between cell 0 and cell 1 on 540.
        let cells = overview_cells(OUT, &[1, 2, 3], 0.5, 20, 0.5, false);
        let mid0 = cells[0].rect.y + cells[0].rect.height / 2;
        let mid1 = cells[1].rect.y + cells[1].rect.height / 2;
        assert!(((mid0 + mid1) / 2 - 540).abs() <= 1);
    }

    #[test]
    fn cells_are_stacked_with_gap() {
        let cells = overview_cells(OUT, &[1, 2, 3], 0.5, 20, 0.0, false);
        // Stride between successive cell tops = cell_h + gap = 540 + 20.
        assert_eq!(cells[1].rect.y - cells[0].rect.y, 560);
        assert_eq!(cells[2].rect.y - cells[1].rect.y, 560);
    }

    #[test]
    fn tags_preserved_in_order() {
        let cells = overview_cells(OUT, &[3, 5, 9], 0.4, 10, 1.0, false);
        assert_eq!(
            cells.iter().map(|c| c.tag).collect::<Vec<_>>(),
            vec![3, 5, 9]
        );
    }

    #[test]
    fn zoom_is_clamped() {
        let cells = overview_cells(OUT, &[1], 5.0, 0, 0.0, false);
        assert!(cells[0].rect.width <= OUT.width);
        assert!(cells[0].rect.height <= OUT.height);
    }

    #[test]
    fn loop_strip_repeats_tags_cyclically() {
        // A 2-tag strip, looped: the emitted window covers the viewport and
        // maps indices to tags[i mod n], so both tags recur and keys are
        // unique per cell.
        let cells = overview_cells(OUT, &[1, 2], 0.5, 20, 0.0, true);
        assert!(cells.len() > 2, "looped strip fills the viewport");
        // Tags alternate 1,2,1,2,…; keys are a 0..len run.
        assert!(cells.iter().all(|c| c.tag == 1 || c.tag == 2));
        let keys: Vec<usize> = cells.iter().map(|c| c.key).collect();
        assert_eq!(keys, (0..cells.len()).collect::<Vec<_>>());
        // Each adjacent pair differs by one stride (no seam/teleport).
        for w in cells.windows(2) {
            assert_eq!(w[1].rect.y - w[0].rect.y, 560);
        }
    }
}
