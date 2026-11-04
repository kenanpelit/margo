//! Which surface paints which part of the frame.
//!
//! The frame is one output-wide shape (the bar bands, the rounded hole, the
//! menu notches). A single full-output layer surface used to paint it, and
//! GTK's Vulkan renderer keeps four output-sized swapchain buffers for every
//! surface, so that one mostly-transparent surface held 4 x 16 MiB on a
//! 2560x1440 output even while only the bars showed.
//!
//! The shape is now split over several surfaces. Each paints the same
//! output-space shape, moved to where it sits and clipped to the rectangles
//! it owns (see [`Viewport`]):
//!
//! * the **top** and **bottom strips** hold the bars. They are as tall as the
//!   bar plus an [`edge_overhang`] that covers the hole's rounded corners and
//!   the border line under the band;
//! * the **left** and **right strips** are [`side_strip_width`] wide and carry
//!   the border line down the screen edges;
//! * the **overlay** is the old full-output surface. It holds the menus and
//!   exists (is realized) only while a menu is open or closing.
//!
//! With no menu open ([`PieceMode::Idle`]) the four strips own everything the
//! frame paints and the overlay is unrealized, which frees its buffers. With
//! a menu open ([`PieceMode::Menu`]) the strips keep only the bar bands, which
//! menus never change, and the overlay owns the rest, so menu animations stay
//! inside one surface and never have to be synchronised across surfaces.
//!
//! The hand-over is ordered so the corners never blink out: on open the
//! overlay (already owning the area between the bands) paints its first frame
//! before the strips give the corners up; on close the strips take them back
//! one frame before the overlay is unmapped. For those frames both paint
//! there, which only shows (slightly darker) with a translucent frame colour.
//!
//! The owned rectangles never overlap and sit on whole logical pixels, so the
//! pieces composite to exactly the pixels the single surface painted (tested
//! below by painting both ways). That holds at integer output scales; at a
//! fractional scale a strip edge can fall between device pixels.

use crate::frame_draw_widget::{FrameStyle, Rect, Viewport};

/// Which surfaces currently own the area below / above the bar bands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PieceMode {
    /// No menu open: the strips paint the whole frame, the overlay is gone.
    Idle,
    /// A menu is open (or closing): the overlay paints everything between the
    /// bar bands, the strips only the bands.
    Menu,
}

/// The sizes the split depends on, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PieceInputs {
    pub output_width: f64,
    pub output_height: f64,
    /// Painted top band (the bar's reserved height).
    pub top_band: f64,
    /// Painted bottom band.
    pub bottom_band: f64,
    /// Height of the top strip surface (bar + overhang, at least band + overhang).
    pub top_strip_height: f64,
    /// Height of the bottom strip surface.
    pub bottom_strip_height: f64,
    /// Width of the left and right strip surfaces.
    pub side_width: f64,
}

/// One viewport per surface.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PieceViewports {
    pub overlay: Viewport,
    pub top: Viewport,
    pub bottom: Viewport,
    pub left: Viewport,
    pub right: Viewport,
}

/// How far below the top band (and above the bottom band) the frame paints
/// while no menu is open: the rounded hole corner plus the border stroke, and
/// one pixel for antialiasing.
pub(crate) fn edge_overhang(border_radius: f64, border_width: f64) -> i32 {
    border_radius.max(border_width).max(0.0).ceil() as i32 + 1
}

/// Width of the left/right strips: the border stroke plus one pixel for
/// antialiasing.
pub(crate) fn side_strip_width(border_width: f64) -> i32 {
    border_width.max(0.0).ceil() as i32 + 1
}

/// The style the strips paint with: the menus' contributions zeroed. Side
/// thickness comes only from side menus (vertical bars were removed), so it
/// is zero too. This is exactly the shape with every menu closed.
pub(crate) fn idle_style(style: &FrameStyle) -> FrameStyle {
    FrameStyle {
        left_thickness: 0.0,
        right_thickness: 0.0,
        left_top_expander_height: 0.0,
        right_top_expander_height: 0.0,
        right_bottom_expander_height: 0.0,
        left_bottom_expander_height: 0.0,
        left_expander_width: 0.0,
        right_expander_width: 0.0,
        top_revealer_size: (0.0, 0.0),
        bottom_revealer_size: (0.0, 0.0),
        top_left_revealer_size: (0.0, 0.0),
        top_right_revealer_size: (0.0, 0.0),
        bottom_left_revealer_size: (0.0, 0.0),
        bottom_right_revealer_size: (0.0, 0.0),
        ..style.clone()
    }
}

/// Where each surface sits and what it owns.
pub(crate) fn piece_viewports(inputs: &PieceInputs, mode: PieceMode) -> PieceViewports {
    let w = inputs.output_width;
    let h = inputs.output_height;
    let top_h = inputs.top_strip_height.clamp(0.0, h);
    let bottom_h = inputs.bottom_strip_height.clamp(0.0, h - top_h);
    let side_w = inputs.side_width.clamp(0.0, w / 2.0);
    let top_band = inputs.top_band.clamp(0.0, h);
    let bottom_band = inputs.bottom_band.clamp(0.0, h - top_band);
    let output = Some((w, h));
    let viewport = |origin: (f64, f64), owned: Vec<Rect>| Viewport {
        origin,
        output,
        owned: Some(owned),
    };
    let middle = (h - top_h - bottom_h).max(0.0);

    match mode {
        PieceMode::Idle => PieceViewports {
            overlay: viewport((0.0, 0.0), Vec::new()),
            top: viewport((0.0, 0.0), vec![Rect::new(0.0, 0.0, w, top_h)]),
            bottom: viewport(
                (0.0, h - inputs.bottom_strip_height),
                vec![Rect::new(0.0, h - bottom_h, w, bottom_h)],
            ),
            left: viewport((0.0, 0.0), vec![Rect::new(0.0, top_h, side_w, middle)]),
            right: viewport(
                (w - inputs.side_width, 0.0),
                vec![Rect::new(w - side_w, top_h, side_w, middle)],
            ),
        },
        PieceMode::Menu => PieceViewports {
            overlay: viewport(
                (0.0, 0.0),
                vec![Rect::new(
                    0.0,
                    top_band,
                    w,
                    (h - top_band - bottom_band).max(0.0),
                )],
            ),
            top: viewport((0.0, 0.0), vec![Rect::new(0.0, 0.0, w, top_band)]),
            bottom: viewport(
                (0.0, h - inputs.bottom_strip_height),
                vec![Rect::new(0.0, h - bottom_band, w, bottom_band)],
            ),
            left: viewport((0.0, 0.0), Vec::new()),
            right: viewport((w - inputs.side_width, 0.0), Vec::new()),
        },
    }
}

/// What to do with the overlay surface after a menu state change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverlayAction {
    /// Map (realize) the overlay, then hand it the area below the bands.
    Show,
    /// A menu reopened while the overlay was waiting to go: keep it.
    CancelHide,
    /// The last menu closed: hide the overlay once the close animation ends.
    ScheduleHide,
    /// Nothing to do.
    Keep,
}

/// The overlay lifecycle: present while any menu is revealed, gone (with its
/// buffers) once none is and the close animation had time to finish.
pub(crate) fn overlay_action(
    any_menu_revealed: bool,
    overlay_shown: bool,
    hide_pending: bool,
) -> OverlayAction {
    match (any_menu_revealed, overlay_shown, hide_pending) {
        (true, false, _) => OverlayAction::Show,
        (true, true, true) => OverlayAction::CancelHide,
        (true, true, false) => OverlayAction::Keep,
        (false, true, false) => OverlayAction::ScheduleHide,
        (false, _, _) => OverlayAction::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_draw_widget::tests::{
        assert_same_pixels, composite, owner_style, pixels, render, with_menus,
    };

    fn inputs(style: &FrameStyle, w: f64, h: f64, bar_top: f64, bar_bottom: f64) -> PieceInputs {
        let overhang = f64::from(edge_overhang(style.border_radius, style.border_width));
        PieceInputs {
            output_width: w,
            output_height: h,
            top_band: style.top_thickness,
            bottom_band: style.bottom_thickness,
            top_strip_height: bar_top.max(style.top_thickness) + overhang,
            bottom_strip_height: bar_bottom.max(style.bottom_thickness) + overhang,
            side_width: f64::from(side_strip_width(style.border_width)),
        }
    }

    /// Paint every piece with the style its surface uses and composite them.
    fn split_render(
        full_style: &FrameStyle,
        inputs: &PieceInputs,
        mode: PieceMode,
        overlay_shown: bool,
    ) -> Vec<u8> {
        let vps = piece_viewports(inputs, mode);
        let strip_style = idle_style(full_style);
        let total = (inputs.output_width, inputs.output_height);
        let (w, h) = (inputs.output_width as i32, inputs.output_height as i32);
        let mut pieces = vec![
            (
                render(
                    &strip_style,
                    total,
                    &vps.top,
                    w,
                    inputs.top_strip_height as i32,
                ),
                vps.top.origin,
            ),
            (
                render(
                    &strip_style,
                    total,
                    &vps.bottom,
                    w,
                    inputs.bottom_strip_height as i32,
                ),
                vps.bottom.origin,
            ),
            (
                render(&strip_style, total, &vps.left, inputs.side_width as i32, h),
                vps.left.origin,
            ),
            (
                render(&strip_style, total, &vps.right, inputs.side_width as i32, h),
                vps.right.origin,
            ),
        ];
        if overlay_shown {
            pieces.push((render(full_style, total, &vps.overlay, w, h), (0.0, 0.0)));
        }
        composite((w, h), &pieces)
    }

    fn styles() -> Vec<FrameStyle> {
        let mut odd = owner_style();
        odd.border_radius = 24.0;
        odd.border_width = 3.5;
        odd.top_thickness = 41.0;
        odd.bottom_thickness = 0.0;
        vec![owner_style(), odd]
    }

    #[test]
    fn idle_strips_paint_exactly_the_closed_frame() {
        for style in styles() {
            let inp = inputs(&style, 400.0, 300.0, style.top_thickness + 2.0, 0.0);
            let full = pixels(render(
                &style,
                (400.0, 300.0),
                &Viewport::default(),
                400,
                300,
            ));
            let got = split_render(&style, &inp, PieceMode::Idle, false);
            assert_same_pixels(&got, &full, 400, "idle");
        }
    }

    #[test]
    fn menu_mode_overlay_and_strips_paint_exactly_the_open_frame() {
        for style in styles() {
            let open = with_menus(style.clone());
            let inp = inputs(
                &open,
                400.0,
                300.0,
                open.top_thickness,
                open.bottom_thickness,
            );
            let full = pixels(render(
                &open,
                (400.0, 300.0),
                &Viewport::default(),
                400,
                300,
            ));
            let got = split_render(&open, &inp, PieceMode::Menu, true);
            assert_same_pixels(&got, &full, 400, "menu open");
            // And with every menu closed again, before the overlay goes.
            let closed = pixels(render(
                &style,
                (400.0, 300.0),
                &Viewport::default(),
                400,
                300,
            ));
            let got = split_render(&style, &inp, PieceMode::Menu, true);
            assert_same_pixels(&got, &closed, 400, "menu closed");
        }
    }

    fn overlaps(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn owned_rects_never_overlap() {
        let style = owner_style();
        let inp = inputs(&style, 2560.0, 1440.0, 38.0, 30.0);
        for mode in [PieceMode::Idle, PieceMode::Menu] {
            let v = piece_viewports(&inp, mode);
            let all: Vec<Rect> = [&v.overlay, &v.top, &v.bottom, &v.left, &v.right]
                .iter()
                .flat_map(|vp| vp.owned.clone().unwrap_or_default())
                .filter(|r| !r.is_empty())
                .collect();
            for (i, a) in all.iter().enumerate() {
                for b in &all[i + 1..] {
                    assert!(!overlaps(a, b), "{mode:?}: {a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn owner_strip_sizes() {
        // 12 px radius, 2 px border: 13 px below the band, 3 px side strips.
        assert_eq!(edge_overhang(12.0, 2.0), 13);
        assert_eq!(side_strip_width(2.0), 3);
        assert_eq!(edge_overhang(0.0, 0.0), 1);
        assert_eq!(side_strip_width(-1.0), 1);
        let style = owner_style();
        let v = piece_viewports(&inputs(&style, 2560.0, 1440.0, 36.0, 30.0), PieceMode::Idle);
        assert_eq!(v.top.owned, Some(vec![Rect::new(0.0, 0.0, 2560.0, 49.0)]));
        assert_eq!(v.bottom.origin, (0.0, 1440.0 - 43.0));
        assert_eq!(v.right.origin, (2557.0, 0.0));
        assert_eq!(v.overlay.owned, Some(Vec::new()));
    }

    #[test]
    fn overlay_lifecycle() {
        use OverlayAction::*;
        assert_eq!(overlay_action(true, false, false), Show);
        assert_eq!(overlay_action(true, false, true), Show);
        assert_eq!(overlay_action(true, true, true), CancelHide);
        assert_eq!(overlay_action(true, true, false), Keep);
        assert_eq!(overlay_action(false, true, false), ScheduleHide);
        assert_eq!(overlay_action(false, true, true), Keep);
        assert_eq!(overlay_action(false, false, false), Keep);
    }
}
