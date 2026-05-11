//! In-compositor region selector — W2.1 from the catch-and-surpass-niri plan.
//!
//! When the user binds `screenshot-region-ui` to a key, instead of
//! spawning `slurp` as an external client (the previous shell-helper
//! flow), the dispatch action lights up an [`ActiveRegionSelector`]
//! on `MargoState`. While active:
//!
//!   * Render path overlays four solid-color edges showing the
//!     selection rectangle on top of every other element.
//!   * Input path intercepts pointer button + motion + keyboard
//!     events: drag to size, release / Enter to commit, Escape to
//!     cancel.
//!   * Commit path spawns `mscreenshot rec` with `MARGO_REGION_GEOM`
//!     set, so the binary skips its own `slurp` invocation and
//!     captures the user-chosen rect directly.
//!
//! ## Why this scope (not "do everything in-compositor")
//!
//! The previous Phase 3 attempt (`3cf9198` revert) tried to handle
//! capture, frozen-frame editing, encoding, clipboard, and editor
//! launch all in-process. Each phase surfaced a new problem
//! (HiDPI math, frame readback colour order, P-toggle repaints,
//! clipboard MIME aliases, focus-aware cursor visibility). The
//! verdict: the in-compositor side is only worth shipping for the
//! parts shell helpers can't do well — and that's just the
//! *selection UI*. `grim` already does capture perfectly.
//! `wl-copy` already does clipboard perfectly. `swappy` already
//! does editing perfectly. They lose to slurp only on the UI-feel
//! front, where it doubles as a separate window fighting focus.
//!
//! So this module does ONE thing well: present a nice selection
//! rect inside margo's render loop, hand the resulting geometry
//! to `mscreenshot`, and get out of the way.

use smithay::backend::renderer::element::{
    solid::{SolidColorBuffer, SolidColorRenderElement},
    Kind,
};
use smithay::utils::{Physical, Point, Scale};

use crate::layout::Rect as LayoutRect;

/// Selection-confirm modes — mirror `mscreenshot`'s region
/// subcommands so users can bind whichever delivery flavour they
/// want behind the same selection UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorMode {
    /// `mscreenshot rec` — region → editor → save + clipboard.
    Rec,
    /// `mscreenshot area` — region → save (no editor, no clip).
    Area,
    /// `mscreenshot ri` — region → editor → save (no clip).
    Ri,
    /// `mscreenshot rc` — region → clipboard only.
    Rc,
    /// `mscreenshot rf` — region → save to disk only.
    Rf,
}

impl SelectorMode {
    /// `mscreenshot` subcommand string for this mode.
    pub fn subcommand(self) -> &'static str {
        match self {
            SelectorMode::Rec => "rec",
            SelectorMode::Area => "area",
            SelectorMode::Ri => "ri",
            SelectorMode::Rc => "rc",
            SelectorMode::Rf => "rf",
        }
    }

    /// Parse a mode name from the dispatch action arg. Falls back
    /// to `Rec` for unrecognized strings — matches the bare
    /// `screenshot-region-ui` action's existing default.
    pub fn parse(s: Option<&str>) -> Self {
        match s.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("rec") | None | Some("") => SelectorMode::Rec,
            Some("area") => SelectorMode::Area,
            Some("ri") => SelectorMode::Ri,
            Some("rc") => SelectorMode::Rc,
            Some("rf") => SelectorMode::Rf,
            _ => SelectorMode::Rec,
        }
    }
}

/// Outline thickness in logical pixels. 2 px is enough to be
/// visible on HiDPI without occluding too much of the captured
/// content underneath.
pub const OUTLINE_PX: i32 = 2;

/// RGBA outline color in 0.0..=1.0 — bright magenta so it stands
/// out against any wallpaper. Could be a config knob later.
pub const OUTLINE_COLOR: [f32; 4] = [0.80, 0.42, 0.97, 1.0];

/// Translucent black tint laid over the entire output while the
/// selector is active. Tells the user "you're in screenshot
/// mode" at a glance + draws attention to the bright outline
/// rect cut out on top. ~22% alpha — dim enough to be obvious,
/// light enough to keep the captured content readable while
/// they aim.
pub const DIM_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.22];

/// Live selection state. Stored as `Option<Self>` on MargoState so
/// `is_some()` becomes the universal "is the selector active?"
/// test.
///
/// The four `outline_*` buffers are pre-allocated 1×1 solid color
/// surfaces. We update their size each frame so smithay's
/// CommitCounter only ticks when the rect actually changes — no
/// per-frame allocation churn.
pub struct ActiveRegionSelector {
    /// The rectangle's first corner — set on pointer button down or
    /// keyboard activation. Logical compositor coords (global).
    pub anchor: (f64, f64),
    /// The rectangle's opposite corner — tracks the cursor while
    /// dragging.
    pub current: (f64, f64),
    /// `true` between button down and button up. Distinguishes the
    /// "user is sizing the rect" phase from "user is pondering"
    /// (after drag end, before Enter / Escape).
    pub dragging: bool,
    /// What to do on confirm.
    pub mode: SelectorMode,

    // Persistent SolidColorBuffer per outline edge. Keep them on
    // the selector itself so their internal Ids remain stable
    // across frames — that's what damage tracking keys off.
    outline_top: SolidColorBuffer,
    outline_bottom: SolidColorBuffer,
    outline_left: SolidColorBuffer,
    outline_right: SolidColorBuffer,
    /// Full-output translucent black tint. Sits above the live
    /// scene but below the outline edges. Resized per-output
    /// each frame; one shared buffer suffices across outputs
    /// because solid-color buffers don't store size in GPU
    /// memory — the size lives on the buffer struct only.
    dim_overlay: SolidColorBuffer,
}

impl std::fmt::Debug for ActiveRegionSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveRegionSelector")
            .field("anchor", &self.anchor)
            .field("current", &self.current)
            .field("dragging", &self.dragging)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl ActiveRegionSelector {
    /// Open the selector at the cursor's current position. The
    /// selector starts un-dragged and zero-area; the first pointer
    /// button-down sets `anchor` to the click point and starts
    /// `dragging`.
    pub fn at(cursor_logical: (f64, f64), mode: SelectorMode) -> Self {
        let make_outline = || SolidColorBuffer::new((1, 1), OUTLINE_COLOR);
        Self {
            anchor: cursor_logical,
            current: cursor_logical,
            dragging: false,
            mode,
            outline_top: make_outline(),
            outline_bottom: make_outline(),
            outline_left: make_outline(),
            outline_right: make_outline(),
            dim_overlay: SolidColorBuffer::new((1, 1), DIM_COLOR),
        }
    }

    /// User clicked: pin the anchor at the click point and start
    /// tracking.
    pub fn begin_drag(&mut self, cursor: (f64, f64)) {
        self.anchor = cursor;
        self.current = cursor;
        self.dragging = true;
    }

    /// Cursor moved while the button is held: extend the rect.
    pub fn update_drag(&mut self, cursor: (f64, f64)) {
        if self.dragging {
            self.current = cursor;
        }
    }

    /// User released: stop tracking. The rect stays — Enter
    /// confirms it, Escape clears the selector, another button
    /// down starts a fresh drag.
    pub fn end_drag(&mut self) {
        self.dragging = false;
    }

    /// Logical-coords selection rect (always positive width/height,
    /// regardless of which corner the user dragged from). Returns
    /// `None` when the rect is degenerate (zero area) — caller
    /// treats this as "user hasn't selected anything yet, ignore
    /// confirm".
    pub fn selection_rect(&self) -> Option<LayoutRect> {
        let x0 = self.anchor.0.min(self.current.0);
        let y0 = self.anchor.1.min(self.current.1);
        let x1 = self.anchor.0.max(self.current.0);
        let y1 = self.anchor.1.max(self.current.1);
        let w = (x1 - x0).round() as i32;
        let h = (y1 - y0).round() as i32;
        if w <= 0 || h <= 0 {
            return None;
        }
        Some(LayoutRect::new(x0.round() as i32, y0.round() as i32, w, h))
    }

    /// Format the selection as the `MARGO_REGION_GEOM` env value
    /// `mscreenshot` looks for — `"X,Y WxH"`, the same shape
    /// `grim -g` accepts. Returns `None` for degenerate selections.
    pub fn geom_string(&self) -> Option<String> {
        let r = self.selection_rect()?;
        Some(format!("{},{} {}x{}", r.x, r.y, r.width, r.height))
    }

    /// Build the overlay render elements for one output:
    ///   * Always: a translucent black tint covering the full
    ///     output ("you're in screenshot mode" cue).
    ///   * If the user has dragged a non-degenerate rect: four
    ///     bright-magenta outline edges around the rect.
    ///
    /// **z-order convention**: vec index 0 = highest z. The
    /// caller (udev::render_output) inserts these BELOW the
    /// cursor element so the cursor stays visible on top of
    /// everything, but ABOVE the live scene so the dim + edges
    /// are visible. Within the vec, edges come first (above the
    /// dim) so the bright outline isn't washed out by its own
    /// tint.
    ///
    /// `output_origin_logical` is the output's top-left in global
    /// logical coords (matches `Monitor::monitor_area.x/y`).
    /// `output_size_logical` is `(width, height)` in logical
    /// pixels — drives the dim's full-output cover.
    /// `output_scale` is the fractional scale.
    pub fn render_elements(
        &mut self,
        output_origin_logical: (i32, i32),
        output_size_logical: (i32, i32),
        output_scale: f64,
    ) -> Vec<SolidColorRenderElement> {
        let scale: Scale<f64> = Scale::from(output_scale);
        let to_phys = |x: i32, y: i32| -> Point<i32, Physical> {
            let px = (x as f64 * output_scale).round() as i32;
            let py = (y as f64 * output_scale).round() as i32;
            Point::from((px, py))
        };

        // Always-on full-output dim. Resize buffer to match the
        // current output (cheap — solid-color buffers store size
        // on the struct, not in GPU memory; `update` only bumps
        // the CommitCounter when something actually changed so
        // damage tracking stays tight).
        let (ow, oh) = output_size_logical;
        self.dim_overlay
            .update((ow.max(1), oh.max(1)), DIM_COLOR);
        let dim_elem = SolidColorRenderElement::from_buffer(
            &self.dim_overlay,
            to_phys(0, 0),
            scale,
            1.0,
            Kind::Unspecified,
        );

        let mut out = Vec::new();

        // Outline edges (only when there's a non-degenerate rect).
        if let Some(rect) = self.selection_rect() {
            let (ox, oy) = output_origin_logical;
            let lx = rect.x - ox;
            let ly = rect.y - oy;
            let lw = rect.width;
            let lh = rect.height;
            let t = OUTLINE_PX;
            let safe = |v: i32| -> i32 { v.max(1) };
            self.outline_top.update((safe(lw), safe(t)), OUTLINE_COLOR);
            self.outline_bottom.update((safe(lw), safe(t)), OUTLINE_COLOR);
            self.outline_left
                .update((safe(t), safe(lh - 2 * t).max(1)), OUTLINE_COLOR);
            self.outline_right
                .update((safe(t), safe(lh - 2 * t).max(1)), OUTLINE_COLOR);

            // Edges first → above dim within this vec.
            out.push(SolidColorRenderElement::from_buffer(
                &self.outline_top,
                to_phys(lx, ly),
                scale,
                1.0,
                Kind::Unspecified,
            ));
            out.push(SolidColorRenderElement::from_buffer(
                &self.outline_bottom,
                to_phys(lx, ly + lh - t),
                scale,
                1.0,
                Kind::Unspecified,
            ));
            out.push(SolidColorRenderElement::from_buffer(
                &self.outline_left,
                to_phys(lx, ly + t),
                scale,
                1.0,
                Kind::Unspecified,
            ));
            out.push(SolidColorRenderElement::from_buffer(
                &self.outline_right,
                to_phys(lx + lw - t, ly + t),
                scale,
                1.0,
                Kind::Unspecified,
            ));
        }

        // Dim last → below edges within this vec; both still
        // above the live scene because the caller pushes the
        // whole vec in front of `elements`.
        out.push(dim_elem);
        out
    }
}

#[cfg(test)]
mod tests {
    //! T6 — geometry tests for the screenshot region selector.
    //!
    //! `selection_rect` normalises (anchor, current) into a
    //! positive-w/h LayoutRect regardless of which corner the user
    //! dragged from. The maths is symmetric across the four
    //! quadrants; any sign-flip regression (forgetting one `.min` /
    //! `.max` swap) lands here as a wrong-quadrant rect.

    use super::*;

    fn sel_at(x: f64, y: f64) -> ActiveRegionSelector {
        ActiveRegionSelector::at((x, y), SelectorMode::Rec)
    }

    fn drag(from: (f64, f64), to: (f64, f64)) -> ActiveRegionSelector {
        let mut s = sel_at(from.0, from.1);
        s.begin_drag(from);
        s.update_drag(to);
        s
    }

    // ── normalisation across drag directions ────────────────────────────────

    #[test]
    fn drag_top_left_to_bottom_right_produces_positive_rect() {
        let s = drag((100.0, 50.0), (300.0, 200.0));
        let r = s.selection_rect().expect("non-degenerate");
        assert_eq!(r.x, 100);
        assert_eq!(r.y, 50);
        assert_eq!(r.width, 200);
        assert_eq!(r.height, 150);
    }

    #[test]
    fn drag_bottom_right_to_top_left_normalises_origin() {
        // Inverse drag — anchor at bottom-right, drag up-left to
        // top-left. selection_rect should still report top-left as
        // (x0, y0) and positive w/h.
        let s = drag((300.0, 200.0), (100.0, 50.0));
        let r = s.selection_rect().expect("non-degenerate");
        assert_eq!(r.x, 100);
        assert_eq!(r.y, 50);
        assert_eq!(r.width, 200);
        assert_eq!(r.height, 150);
    }

    #[test]
    fn drag_top_right_to_bottom_left_normalises_origin() {
        let s = drag((400.0, 0.0), (50.0, 250.0));
        let r = s.selection_rect().expect("non-degenerate");
        assert_eq!(r.x, 50);
        assert_eq!(r.y, 0);
        assert_eq!(r.width, 350);
        assert_eq!(r.height, 250);
    }

    #[test]
    fn drag_bottom_left_to_top_right_normalises_origin() {
        let s = drag((50.0, 250.0), (400.0, 0.0));
        let r = s.selection_rect().expect("non-degenerate");
        assert_eq!(r.x, 50);
        assert_eq!(r.y, 0);
        assert_eq!(r.width, 350);
        assert_eq!(r.height, 250);
    }

    // ── degeneracy ──────────────────────────────────────────────────────────

    #[test]
    fn zero_area_selection_returns_none() {
        // No drag at all: anchor == current, w = 0, h = 0 → None.
        let s = sel_at(100.0, 100.0);
        assert!(s.selection_rect().is_none());
    }

    #[test]
    fn single_pixel_drag_is_below_threshold() {
        // Sub-pixel drag — round(x1-x0) < 1 → degenerate.
        let s = drag((100.0, 100.0), (100.2, 100.3));
        assert!(s.selection_rect().is_none());
    }

    #[test]
    fn vertical_line_drag_has_zero_width_and_is_rejected() {
        // Horizontal motion zero, vertical 50 px → w=0, rejected.
        let s = drag((100.0, 100.0), (100.0, 150.0));
        assert!(s.selection_rect().is_none());
    }

    #[test]
    fn horizontal_line_drag_has_zero_height_and_is_rejected() {
        let s = drag((100.0, 100.0), (200.0, 100.0));
        assert!(s.selection_rect().is_none());
    }

    // ── geom string formatting ──────────────────────────────────────────────

    #[test]
    fn geom_string_matches_grim_format() {
        let s = drag((100.0, 50.0), (300.0, 200.0));
        let g = s.geom_string().expect("formatted");
        assert_eq!(g, "100,50 200x150");
    }

    #[test]
    fn geom_string_none_for_degenerate_selection() {
        let s = sel_at(50.0, 50.0);
        assert!(s.geom_string().is_none());
    }

    // ── drag lifecycle ──────────────────────────────────────────────────────

    #[test]
    fn end_drag_preserves_selection_rect() {
        // After button-up: rect should still be readable for
        // post-drag confirm.
        let mut s = drag((100.0, 100.0), (200.0, 200.0));
        s.end_drag();
        let r = s.selection_rect().expect("non-degenerate after end_drag");
        assert_eq!(r.x, 100);
        assert_eq!(r.width, 100);
        assert!(!s.dragging);
    }

    #[test]
    fn update_drag_outside_drag_is_a_noop() {
        // No `begin_drag` was called — `dragging` stays false and
        // update_drag must not move `current`.
        let mut s = sel_at(100.0, 100.0);
        s.update_drag((300.0, 300.0));
        assert!(s.selection_rect().is_none(), "no drag started, no rect");
        assert_eq!(s.current, (100.0, 100.0));
    }

    #[test]
    fn begin_drag_resets_anchor_to_click_point() {
        // Selector was opened at one position, but the user
        // *clicked* at a different point — anchor must snap to the
        // click. Stops rectangles starting at "wherever the cursor
        // happened to be when screenshot mode armed".
        let mut s = sel_at(10.0, 10.0);
        s.begin_drag((200.0, 150.0));
        s.update_drag((400.0, 250.0));
        let r = s.selection_rect().expect("non-degenerate");
        assert_eq!(r.x, 200);
        assert_eq!(r.y, 150);
        assert_eq!(r.width, 200);
        assert_eq!(r.height, 100);
    }

    // ── floating-point edge cases ───────────────────────────────────────────

    #[test]
    fn round_to_nearest_pixel_not_truncated() {
        // 100.4 → 100, 100.5 → 101 (round half-to-even on Rust).
        // The selector rounds at the end, so a 0.5-pixel anchor +
        // 100.4 current produces width = round(100.4 - 0.5) = 100.
        let s = drag((0.5, 0.5), (100.4, 50.4));
        let r = s.selection_rect().expect("non-degenerate");
        // x0 = round(0.5) = 0 (banker's rounding to even)
        // y0 = round(0.5) = 0
        // w  = round(100.4 - 0.5) = 100
        // h  = round(50.4 - 0.5) = 50
        assert!(r.x <= 1, "x in [0, 1] got {}", r.x);
        assert!(r.y <= 1, "y in [0, 1] got {}", r.y);
        assert!((99..=101).contains(&r.width));
        assert!((49..=51).contains(&r.height));
    }
}
