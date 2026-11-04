//! Where the border ring and the drop shadow actually put pixels.
//!
//! Both are drawn by a pixel shader over a rect that covers the whole window
//! (plus border / shadow padding). Most of that rect comes out fully
//! transparent: the border is a few pixels wide and the shadow is hidden
//! under the window. Reporting the whole rect as damage and shading every
//! damaged pixel of it re-composites the window interior whenever the border
//! colour animates and shades the full window once more on every damaging
//! client frame. These helpers give the disjoint rects that can hold visible
//! pixels, so `damage_since` and `draw` stay on the band.

use smithay::utils::{Physical, Rectangle, Size};

type Rect = Rectangle<i32, Physical>;

/// Edge and corner thickness (physical px) of a rounded border ring of
/// `border` px width and `radius` px outer corner radius: the straight edges
/// need the border plus the 1 px anti-aliasing, the corners also the curve of
/// the inner hole, which reaches `radius` px in from the outer edge.
pub fn border_band(border: f32, radius: f32) -> (i32, i32) {
    let border = border.max(0.0);
    let edge = border.ceil() as i32 + 1;
    let corner = border.max(radius).ceil() as i32 + 1;
    (edge, corner)
}

/// The ring of thickness `edge` along the sides and `corner` x `corner`
/// squares at the corners of a `size` rect at the origin, as disjoint rects.
/// Falls back to the whole rect when the corners would meet.
pub fn ring_rects(size: Size<i32, Physical>, edge: i32, corner: i32) -> Vec<Rect> {
    let (w, h) = (size.w, size.h);
    let corner = corner.max(edge);
    if w <= 2 * corner || h <= 2 * corner {
        return vec![Rectangle::from_size(size)];
    }
    let mid_w = w - 2 * corner;
    let mid_h = h - 2 * corner;
    vec![
        // Corners.
        rect(0, 0, corner, corner),
        rect(w - corner, 0, corner, corner),
        rect(0, h - corner, corner, corner),
        rect(w - corner, h - corner, corner, corner),
        // Straight edges between them.
        rect(corner, 0, mid_w, edge),
        rect(corner, h - edge, mid_w, edge),
        rect(0, corner, edge, mid_h),
        rect(w - edge, corner, edge, mid_h),
    ]
}

/// The part of a `size` shadow rect (at the origin) that the window does not
/// cover. `window` is the window rect relative to the shadow rect and
/// `radius` its corner radius. The window's 1 px anti-aliased outline and its
/// rounded corners stay in, so the shadow still shows through them.
pub fn shadow_rects(size: Size<i32, Physical>, window: Rect, radius: f32) -> Vec<Rect> {
    let c = (radius.max(0.0).ceil() as i32).max(1);
    let (x, y, w, h) = (window.loc.x, window.loc.y, window.size.w, window.size.h);
    // The window's opaque interior: inset by the 1 px outline, a cross that
    // leaves out the corner squares.
    let interior = [
        rect(x + c, y + 1, w - 2 * c, h - 2),
        rect(x + 1, y + c, w - 2, h - 2 * c),
    ]
    .into_iter()
    .filter(|r| r.size.w > 0 && r.size.h > 0);
    Rectangle::subtract_rects_many([Rectangle::from_size(size)], interior)
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rectangle::new((x, y).into(), (w, h).into())
}

/// `damage` restricted to `area`. Both lists are disjoint, so the result is
/// too (a draw never blends the same pixel twice).
pub fn clip_damage(damage: &[Rect], area: &[Rect]) -> Vec<Rect> {
    damage
        .iter()
        .flat_map(|d| area.iter().filter_map(move |a| d.intersection(*a)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::utils::Point;

    fn covers(rects: &[Rect], x: i32, y: i32) -> bool {
        rects
            .iter()
            .any(|r| r.contains::<Point<i32, Physical>>((x, y).into()))
    }

    fn assert_disjoint(rects: &[Rect]) {
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!a.overlaps(*b), "{a:?} overlaps {b:?}");
            }
        }
    }

    fn area(rects: &[Rect]) -> i32 {
        rects.iter().map(|r| r.size.w * r.size.h).sum()
    }

    #[test]
    fn the_border_band_covers_the_width_the_curve_and_the_anti_aliasing() {
        assert_eq!(border_band(2.0, 12.0), (3, 13));
        // Fractional scale: round up, never cut a partially lit pixel.
        assert_eq!(border_band(2.5, 15.25), (4, 17));
        // Square corners: the corner is as thick as the edge.
        assert_eq!(border_band(2.0, 0.0), (3, 3));
        // A border wider than the radius sets both.
        assert_eq!(border_band(8.0, 4.0), (9, 9));
    }

    #[test]
    fn the_ring_leaves_the_interior_out() {
        let rects = ring_rects((100, 80).into(), 3, 13);
        assert_disjoint(&rects);
        assert!(!covers(&rects, 50, 40), "window interior");
        assert!(!covers(&rects, 20, 20));
        for (x, y) in [
            (0, 40),
            (2, 40),
            (99, 40),
            (50, 0),
            (50, 79),
            (5, 5),
            (12, 12),
            (90, 70),
        ] {
            assert!(covers(&rects, x, y), "{x},{y} is on the ring");
        }
        assert!(!covers(&rects, 3, 40), "past the straight edge");
        assert_eq!(
            area(&rects),
            4 * 13 * 13 + 2 * (100 - 26) * 3 + 2 * (80 - 26) * 3
        );
    }

    #[test]
    fn a_ring_whose_corners_meet_is_the_whole_rect() {
        assert_eq!(ring_rects((20, 60).into(), 3, 13), vec![rect(0, 0, 20, 60)]);
        assert_eq!(ring_rects((60, 26).into(), 3, 13), vec![rect(0, 0, 60, 26)]);
    }

    #[test]
    fn the_shadow_skips_what_the_window_covers() {
        // 140x120 shadow, window 100x80 at (20, 20), radius 10.
        let window = rect(20, 20, 100, 80);
        let rects = shadow_rects((140, 120).into(), window, 10.0);
        assert_disjoint(&rects);
        assert!(!covers(&rects, 70, 60), "under the window");
        assert!(!covers(&rects, 21, 60), "inside the window's straight edge");
        assert!(covers(&rects, 20, 60), "the window's anti-aliased outline");
        assert!(covers(&rects, 22, 22), "under the rounded corner");
        assert!(covers(&rects, 5, 5), "outside the window");
        assert!(covers(&rects, 130, 110));
        // Everything minus the interior cross.
        let cross = 98 * 78 - 4 * 9 * 9;
        assert_eq!(area(&rects), 140 * 120 - cross);
    }

    #[test]
    fn a_shadow_offset_past_the_window_is_drawn_whole_where_uncovered() {
        // The window only overlaps the shadow's top-left corner.
        let rects = shadow_rects((100, 100).into(), rect(-80, -80, 100, 100), 0.0);
        assert_disjoint(&rects);
        assert!(!covers(&rects, 5, 5));
        assert!(covers(&rects, 19, 5), "the window's outline");
        assert!(covers(&rects, 50, 50));
        assert_eq!(area(&rects), 100 * 100 - 19 * 19);
    }

    #[test]
    fn damage_is_cut_to_the_band() {
        let ring = ring_rects((100, 80).into(), 3, 13);
        let cut = clip_damage(&[rect(0, 0, 100, 80)], &ring);
        assert_eq!(area(&cut), area(&ring));
        assert!(clip_damage(&[rect(30, 30, 10, 10)], &ring).is_empty());
        let cut = clip_damage(&[rect(0, 30, 10, 10)], &ring);
        assert_eq!(cut, vec![rect(0, 30, 3, 10)]);
    }
}
