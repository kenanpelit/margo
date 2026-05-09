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
        let make_buf = || SolidColorBuffer::new((1, 1), OUTLINE_COLOR);
        Self {
            anchor: cursor_logical,
            current: cursor_logical,
            dragging: false,
            mode,
            outline_top: make_buf(),
            outline_bottom: make_buf(),
            outline_left: make_buf(),
            outline_right: make_buf(),
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

    /// Build the overlay render elements for one output. Returns
    /// up to four [`SolidColorRenderElement`] instances representing
    /// the top / bottom / left / right edges of the selection rect.
    /// Empty when the selector hasn't been dragged yet (degenerate
    /// rect) OR when the rect doesn't intersect this output (the
    /// user can drag across multiple monitors; each monitor draws
    /// only the segments that fall in its bounds).
    ///
    /// `output_origin_logical` is the output's top-left in global
    /// logical coords (matches `Monitor::monitor_area.x/y`).
    /// `output_scale` is the fractional scale.
    pub fn render_elements(
        &mut self,
        output_origin_logical: (i32, i32),
        output_scale: f64,
    ) -> Vec<SolidColorRenderElement> {
        let Some(rect) = self.selection_rect() else {
            return Vec::new();
        };
        // Translate to output-local logical coords.
        let (ox, oy) = output_origin_logical;
        let lx = rect.x - ox;
        let ly = rect.y - oy;
        let lw = rect.width;
        let lh = rect.height;
        let t = OUTLINE_PX;

        // Update each buffer's logical size — the
        // SolidColorRenderElement converts to physical via the
        // scale we pass.
        let safe = |v: i32| -> i32 { v.max(1) };
        self.outline_top.update((safe(lw), safe(t)), OUTLINE_COLOR);
        self.outline_bottom.update((safe(lw), safe(t)), OUTLINE_COLOR);
        self.outline_left
            .update((safe(t), safe(lh - 2 * t).max(1)), OUTLINE_COLOR);
        self.outline_right
            .update((safe(t), safe(lh - 2 * t).max(1)), OUTLINE_COLOR);

        let scale: Scale<f64> = Scale::from(output_scale);
        let to_phys = |x: i32, y: i32| -> Point<i32, Physical> {
            let px = (x as f64 * output_scale).round() as i32;
            let py = (y as f64 * output_scale).round() as i32;
            Point::from((px, py))
        };
        vec![
            SolidColorRenderElement::from_buffer(
                &self.outline_top,
                to_phys(lx, ly),
                scale,
                1.0,
                Kind::Unspecified,
            ),
            SolidColorRenderElement::from_buffer(
                &self.outline_bottom,
                to_phys(lx, ly + lh - t),
                scale,
                1.0,
                Kind::Unspecified,
            ),
            SolidColorRenderElement::from_buffer(
                &self.outline_left,
                to_phys(lx, ly + t),
                scale,
                1.0,
                Kind::Unspecified,
            ),
            SolidColorRenderElement::from_buffer(
                &self.outline_right,
                to_phys(lx + lw - t, ly + t),
                scale,
                1.0,
                Kind::Unspecified,
            ),
        ]
    }

}
