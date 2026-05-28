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

use super::MargoState;

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

        // Seed the selection from the focused monitor's first active tag
        // so keyboard nav starts where the user already is.
        let mon_idx = self.focused_monitor();
        let selected_tag = self
            .monitors
            .get(mon_idx)
            .map(|mon| mon.current_tagset().trailing_zeros() as usize)
            .unwrap_or(0);

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
}
