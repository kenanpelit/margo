//! niri-style **scroller overview**: a zoomed-out, scrollable strip of
//! per-tag mini-desktops, each rendering its tag's real tiling layout
//! scaled down (no window resize — the scaling happens at render time
//! via `RescaleRenderElement`). Entirely separate from the classic grid
//! overview in `state/overview.rs` (`is_overview` / `overview_*`); the
//! two are mutually exclusive but share no state.
//!
//! This module owns the [`ScrollerOverview`] value and its open / close /
//! toggle lifecycle on [`MargoState`]. Rendering (P2), the zoom
//! transition (P3), and input (P4) live in their own call sites and
//! read this state.

use super::{FocusTarget, MargoState};
use crate::layout::{MAX_TAGS, Rect};

/// One tag cell in the overview strip: which tag it shows and where it
/// renders (output-logical coordinates, after centering on the
/// selection). The render path scales each tag's windows into `rect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverviewCell {
    /// 1-based pertag tag index (1..=`MAX_TAGS`).
    pub tag: usize,
    pub rect: Rect,
}

/// Lay `tags` out as a vertical strip of cells, each the `output`
/// rectangle scaled by `zoom`, separated by `gap` (post-zoom logical
/// px), horizontally centered, and offset so the cell for
/// `selected_tag` sits vertically centered in `output`. Returns one
/// [`OverviewCell`] per input tag, in order.
///
/// Pure geometry — no compositor state — so it's unit-tested directly.
pub fn overview_cells(
    output: Rect,
    tags: &[usize],
    zoom: f64,
    gap: i32,
    selected_tag: usize,
) -> Vec<OverviewCell> {
    if tags.is_empty() {
        return Vec::new();
    }
    let zoom = zoom.clamp(0.05, 1.0);
    let cell_w = ((f64::from(output.width)) * zoom).round() as i32;
    let cell_h = ((f64::from(output.height)) * zoom).round() as i32;
    let cell_x = output.x + (output.width - cell_w) / 2;
    let stride = cell_h + gap.max(0);

    // Strip position of the selected tag (fallback: first cell).
    let sel_pos = tags
        .iter()
        .position(|&t| t == selected_tag)
        .unwrap_or(0) as i32;
    // Offset so the selected cell's vertical center lands on the output
    // center: y(sel) + cell_h/2 == output.y + output.height/2.
    let offset_y = output.height / 2 - sel_pos * stride - cell_h / 2;

    tags.iter()
        .enumerate()
        .map(|(i, &tag)| OverviewCell {
            tag,
            rect: Rect::new(cell_x, output.y + offset_y + i as i32 * stride, cell_w, cell_h),
        })
        .collect()
}

/// Live state of an open scroller overview. Present (`Some`) only while
/// the overview is open or animating closed; `None` otherwise.
#[derive(Debug, Clone)]
pub struct ScrollerOverview {
    /// Animated open progress: `0.0` = closed (normal 1× view), `1.0` =
    /// fully zoomed out. The render path interpolates the zoom factor
    /// from this. P1 snaps it to `1.0` on open; P3 eases it.
    pub progress: f64,
    /// Direction the animation is heading: `true` while opening / open,
    /// `false` while closing. When closing reaches `progress == 0` the
    /// whole `Option` is cleared (P3).
    pub opening: bool,
    /// Vertical scroll offset into the tag strip, in pre-zoom logical
    /// pixels. `0.0` = first tag cell flush with the top of the strip.
    pub scroll: f64,
    /// Which tag (0-based) is highlighted for keyboard navigation and
    /// activation. Seeded from the focused monitor's active tag.
    pub selected_tag: usize,
}

impl ScrollerOverview {
    fn new(selected_tag: usize) -> Self {
        Self {
            // P1: snap straight to fully-open. P3 replaces this with an
            // eased ramp from 0.0.
            progress: 1.0,
            opening: true,
            scroll: 0.0,
            selected_tag,
        }
    }
}

impl MargoState {
    /// True while the scroller overview is open or animating.
    pub fn is_scroller_overview_open(&self) -> bool {
        self.scroller_overview.is_some()
    }

    /// Open the scroller overview. No-op if already open. Closes the
    /// classic grid overview first if it happens to be open — the two
    /// are mutually exclusive so we never composite both transforms at
    /// once.
    pub fn open_scroller_overview(&mut self) {
        if self.scroller_overview.is_some() {
            return;
        }
        if self.is_overview_open() {
            self.close_overview(None);
        }

        // Seed the selection from the focused monitor's active tag so
        // keyboard nav starts where the user already is. `pertag.curtag`
        // is the 1-based tag index (1..=9), matching our cell tags.
        let mon_idx = self.focused_monitor();
        let selected_tag = self
            .monitors
            .get(mon_idx)
            .map(|mon| mon.pertag.curtag.clamp(1, crate::layout::MAX_TAGS))
            .unwrap_or(1);

        self.scroller_overview = Some(ScrollerOverview::new(selected_tag));
        self.request_repaint();
    }

    /// Close the scroller overview. No-op if not open. P1 closes
    /// instantly; P3 animates the zoom back in before clearing.
    pub fn close_scroller_overview(&mut self) {
        if self.scroller_overview.is_none() {
            return;
        }
        self.scroller_overview = None;
        self.request_repaint();
    }

    pub fn toggle_scroller_overview(&mut self) {
        if self.is_scroller_overview_open() {
            self.close_scroller_overview();
        } else {
            self.open_scroller_overview();
        }
    }

    /// Toggle whichever overview the user picked as their preferred
    /// style (`overview_style` config). This is what the generic
    /// `toggle_overview` keybind dispatches to, so a single key opens
    /// the grid or the scroller depending on the Settings choice.
    pub fn toggle_overview_styled(&mut self) {
        match self.config.overview_style {
            margo_config::OverviewStyle::Scroller => self.toggle_scroller_overview(),
            margo_config::OverviewStyle::Grid => self.toggle_overview(),
        }
    }

    /// Tags (1-based) to show as cells for `mon_idx`: every tag that
    /// has at least one mapped, non-minimized client, always including
    /// the currently-selected tag so the strip is never empty and the
    /// selection always has a cell to highlight. Ascending order.
    pub fn scroller_overview_tags(&self, mon_idx: usize) -> Vec<usize> {
        let selected = self
            .scroller_overview
            .as_ref()
            .map(|ov| ov.selected_tag)
            .unwrap_or(1);

        (1..=MAX_TAGS)
            .filter(|&tag| {
                tag == selected
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

    /// Move the overview selection by `dir` (+1 next / −1 prev) through
    /// the shown tags on the focused monitor, wrapping around. Drives
    /// scroll-wheel and arrow-key navigation.
    pub fn scroller_overview_select(&mut self, dir: i32) {
        if self.scroller_overview.is_none() {
            return;
        }
        let mon = self.focused_monitor();
        let tags = self.scroller_overview_tags(mon);
        if tags.is_empty() {
            return;
        }
        let cur = self
            .scroller_overview
            .as_ref()
            .map(|o| o.selected_tag)
            .unwrap_or(1);
        let pos = tags.iter().position(|&t| t == cur).unwrap_or(0) as i32;
        let n = tags.len() as i32;
        let next = (pos + dir).rem_euclid(n) as usize;
        if let Some(ov) = self.scroller_overview.as_mut() {
            ov.selected_tag = tags[next];
        }
        self.request_repaint();
    }

    /// Close the overview and switch the focused monitor to the selected
    /// tag (Enter / commit). No-op switch if already on that tag.
    pub fn scroller_overview_activate(&mut self) {
        let Some(ov) = self.scroller_overview.take() else {
            return;
        };
        self.request_repaint();
        let tag = ov.selected_tag.clamp(1, MAX_TAGS);
        let bit = 1u32 << (tag - 1);
        let mon = self.focused_monitor();
        let already = self.monitors.get(mon).map(|m| m.current_tagset()) == Some(bit);
        if !already {
            self.view_tag(bit);
        }
    }

    /// Handle a left click at global-logical (`x`, `y`) while the
    /// overview is open: find the tag cell under the cursor, switch to
    /// that tag, and focus the specific window clicked (if any). A click
    /// on the bare backdrop (no cell) closes without switching.
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

        // Monitor whose area contains the cursor (fallback: focused).
        let mon = self
            .monitors
            .iter()
            .position(|m| contains(m.monitor_area, x, y))
            .unwrap_or_else(|| self.focused_monitor());
        let Some(area) = self.monitors.get(mon).map(|m| m.monitor_area) else {
            return;
        };

        let tags = self.scroller_overview_tags(mon);
        let selected = self
            .scroller_overview
            .as_ref()
            .map(|o| o.selected_tag)
            .unwrap_or(1);
        let zoom = f64::from(self.config.scroller_overview_zoom.clamp(0.1, 1.0));
        let gap = self.config.scroller_overview_gap.max(0);
        let cells = overview_cells(area, &tags, zoom, gap, selected);

        let Some(cell) = cells.into_iter().find(|c| contains(c.rect, x, y)) else {
            // Click on the bare backdrop → close without switching.
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

        self.scroller_overview = None;
        self.request_repaint();
        let already = self.monitors.get(mon).map(|m| m.current_tagset()) == Some(bit);
        if !already {
            self.view_tag(bit);
        }
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

    const OUT: Rect = Rect { x: 0, y: 0, width: 1920, height: 1080 };

    #[test]
    fn empty_tags_yields_no_cells() {
        assert!(overview_cells(OUT, &[], 0.5, 20, 1).is_empty());
    }

    #[test]
    fn cell_size_is_output_scaled_by_zoom() {
        let cells = overview_cells(OUT, &[1, 2, 3], 0.5, 20, 1);
        assert_eq!(cells.len(), 3);
        for c in &cells {
            assert_eq!(c.rect.width, 960); // 1920 * 0.5
            assert_eq!(c.rect.height, 540); // 1080 * 0.5
        }
        // Horizontally centered: (1920 - 960) / 2 = 480.
        assert_eq!(cells[0].rect.x, 480);
    }

    #[test]
    fn selected_cell_is_vertically_centered() {
        // Select the middle tag; its cell center should sit on the
        // output's vertical center (540).
        let cells = overview_cells(OUT, &[1, 2, 3], 0.5, 20, 2);
        let sel = cells.iter().find(|c| c.tag == 2).unwrap();
        let center = sel.rect.y + sel.rect.height / 2;
        assert_eq!(center, 540);
    }

    #[test]
    fn cells_are_stacked_with_gap() {
        let cells = overview_cells(OUT, &[1, 2, 3], 0.5, 20, 1);
        // Stride between successive cell tops = cell_h + gap = 540 + 20.
        assert_eq!(cells[1].rect.y - cells[0].rect.y, 560);
        assert_eq!(cells[2].rect.y - cells[1].rect.y, 560);
    }

    #[test]
    fn tags_preserved_in_order() {
        let cells = overview_cells(OUT, &[3, 5, 9], 0.4, 10, 5);
        assert_eq!(cells.iter().map(|c| c.tag).collect::<Vec<_>>(), vec![3, 5, 9]);
    }

    #[test]
    fn zoom_is_clamped() {
        // Absurd zoom clamps to <= 1.0 so a cell never exceeds the output.
        let cells = overview_cells(OUT, &[1], 5.0, 0, 1);
        assert!(cells[0].rect.width <= OUT.width);
        assert!(cells[0].rect.height <= OUT.height);
    }
}
