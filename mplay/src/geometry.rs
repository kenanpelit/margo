//! Pure window-placement + scale helpers (no I/O). Ported from
//! `margo-mpv.sh`'s `cmd_move` corner-cycle math so it can be unit-tested
//! in isolation.

/// A monitor (or window) rectangle in compositor logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Clamp `n` into `[min, max]`. If `max < min` the range collapses to
/// `min` (matches the shell helper's guard).
pub fn clamp(n: i32, min: i32, max: i32) -> i32 {
    let max = if max < min { min } else { max };
    n.clamp(min, max)
}

/// The four floating-window resting corners, cycled by `mplay snap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    /// The next corner in the snap cycle: TL → TR → BR → BL → TL.
    pub fn next(self) -> Corner {
        match self {
            Corner::TopLeft => Corner::TopRight,
            Corner::TopRight => Corner::BottomRight,
            Corner::BottomRight => Corner::BottomLeft,
            Corner::BottomLeft => Corner::TopLeft,
        }
    }

    /// The clamped top-left target position for a `w`×`h` window in `area`
    /// with the given edge margins.
    pub fn position(self, area: Rect, w: i32, h: i32, mx: i32, my: i32) -> (i32, i32) {
        let max_x = area.x + area.w - w;
        let max_y = area.y + area.h - h;
        let left = clamp(area.x + mx, area.x, max_x);
        let right = clamp(area.x + area.w - w - mx, area.x, max_x);
        let top = clamp(area.y + my, area.y, max_y);
        let bottom = clamp(area.y + area.h - h - my, area.y, max_y);
        match self {
            Corner::TopLeft => (left, top),
            Corner::TopRight => (right, top),
            Corner::BottomRight => (right, bottom),
            Corner::BottomLeft => (left, bottom),
        }
    }
}

/// Which resting corner a window at `(wx, wy)` is currently closest to
/// (Manhattan distance), so `snap` can advance to the *next* one. Ties
/// resolve TL → TR → BR → BL, matching the shell helper.
#[allow(clippy::too_many_arguments)]
pub fn nearest_corner(wx: i32, wy: i32, w: i32, h: i32, area: Rect, mx: i32, my: i32) -> Corner {
    let dist = |c: Corner| {
        let (cx, cy) = c.position(area, w, h, mx, my);
        (wx - cx).abs() + (wy - cy).abs()
    };
    let d_tl = dist(Corner::TopLeft);
    let d_tr = dist(Corner::TopRight);
    let d_br = dist(Corner::BottomRight);
    let d_bl = dist(Corner::BottomLeft);
    if d_tr <= d_tl && d_tr <= d_br && d_tr <= d_bl {
        Corner::TopRight
    } else if d_br <= d_tl && d_br <= d_tr && d_br <= d_bl {
        Corner::BottomRight
    } else if d_bl <= d_tl && d_bl <= d_tr && d_bl <= d_br {
        Corner::BottomLeft
    } else {
        Corner::TopLeft
    }
}

/// How a wallpaper video fills its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleMode {
    /// Whole frame visible, letterboxed (keep aspect, no pan).
    Fit,
    /// Cover the output, cropping overflow (keep aspect, full panscan).
    Fill,
    /// Distort to exactly fill (ignore aspect).
    Stretch,
}

impl ScaleMode {
    pub fn parse(s: &str) -> Option<ScaleMode> {
        match s {
            "fit" => Some(ScaleMode::Fit),
            "fill" => Some(ScaleMode::Fill),
            "stretch" => Some(ScaleMode::Stretch),
            _ => None,
        }
    }

    /// libmpv option string pairs that realise this scale mode.
    pub fn mpv_opts(self) -> &'static [(&'static str, &'static str)] {
        match self {
            ScaleMode::Fit => &[("keepaspect", "yes"), ("panscan", "0.0")],
            ScaleMode::Fill => &[("keepaspect", "yes"), ("panscan", "1.0")],
            ScaleMode::Stretch => &[("keepaspect", "no"), ("panscan", "0.0")],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        w: 1000,
        h: 1000,
    };

    #[test]
    fn clamp_bounds() {
        assert_eq!(clamp(5, 0, 10), 5);
        assert_eq!(clamp(-3, 0, 10), 0);
        assert_eq!(clamp(99, 0, 10), 10);
        assert_eq!(clamp(5, 8, 2), 8); // max<min → collapses to min
    }

    #[test]
    fn corner_cycle_order() {
        assert_eq!(Corner::TopLeft.next(), Corner::TopRight);
        assert_eq!(Corner::TopRight.next(), Corner::BottomRight);
        assert_eq!(Corner::BottomRight.next(), Corner::BottomLeft);
        assert_eq!(Corner::BottomLeft.next(), Corner::TopLeft);
    }

    #[test]
    fn corner_positions_are_clamped_with_margins() {
        // 200x100 window, margins 32/96.
        assert_eq!(Corner::TopLeft.position(AREA, 200, 100, 32, 96), (32, 96));
        assert_eq!(
            Corner::TopRight.position(AREA, 200, 100, 32, 96),
            (1000 - 200 - 32, 96)
        );
        assert_eq!(
            Corner::BottomRight.position(AREA, 200, 100, 32, 96),
            (768, 1000 - 100 - 96)
        );
    }

    #[test]
    fn nearest_corner_picks_closest() {
        // window near the top-right
        assert_eq!(
            nearest_corner(900, 96, 200, 100, AREA, 32, 96),
            Corner::TopRight
        );
        // window near the bottom-left
        assert_eq!(
            nearest_corner(32, 800, 200, 100, AREA, 32, 96),
            Corner::BottomLeft
        );
    }

    #[test]
    fn scale_mode_parse() {
        assert_eq!(ScaleMode::parse("fill"), Some(ScaleMode::Fill));
        assert_eq!(ScaleMode::parse("fit"), Some(ScaleMode::Fit));
        assert_eq!(ScaleMode::parse("stretch"), Some(ScaleMode::Stretch));
        assert_eq!(ScaleMode::parse("xyz"), None);
    }
}
