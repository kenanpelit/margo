//! Per-drive trend chart for the Drives tab: a time-scaled line over the
//! drive-health history (temperature, or wear when there is more than one
//! reading), drawn like the dashboard's CPU sparkline — accent from the
//! DrawingArea's CSS `color`, faint baseline, gradient fill — plus a dashed
//! threshold line. The geometry is pure and tested.

use relm4::gtk;
use relm4::gtk::prelude::{DrawingAreaExt, DrawingAreaExtManual, WidgetExt};

/// Map `(t, v)` points into a `w`×`h` box: x by time (oldest left), y by
/// value within `[lo, hi]` (inset 1.5 px so the stroke isn't clipped).
pub(crate) fn plot(points: &[(u64, f64)], lo: f64, hi: f64, w: f64, h: f64) -> Vec<(f64, f64)> {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return Vec::new();
    };
    let span = (last.0.saturating_sub(first.0)).max(1) as f64;
    let range = (hi - lo).max(1e-9);
    points
        .iter()
        .map(|(t, v)| {
            let x = (t.saturating_sub(first.0)) as f64 / span * w;
            let frac = ((v - lo) / range).clamp(0.0, 1.0);
            (x, h - frac * (h - 3.0) - 1.5)
        })
        .collect()
}

/// The value range a chart shows: the data's, widened to include the
/// threshold and padded so a flat line sits mid-height.
pub(crate) fn value_range(points: &[(u64, f64)], threshold: Option<f64>) -> (f64, f64) {
    let mut lo = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let mut hi = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    if let Some(t) = threshold {
        lo = lo.min(t);
        hi = hi.max(t);
    }
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    let pad = ((hi - lo) * 0.1).max(1.0);
    (lo - pad, hi + pad)
}

/// "38–52°C over 6 days" style caption.
pub(crate) fn caption(points: &[(u64, f64)], unit: &str) -> String {
    let lo = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let hi = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let span = match (points.first(), points.last()) {
        (Some(a), Some(b)) => b.0.saturating_sub(a.0),
        _ => 0,
    };
    let over = if span >= 2 * 86_400 {
        format!("{} days", span / 86_400)
    } else if span >= 7200 {
        format!("{} hours", span / 3600)
    } else {
        format!("{} min", (span / 60).max(1))
    };
    if !lo.is_finite() {
        return String::new();
    }
    if (hi - lo).abs() < 0.5 {
        format!("{lo:.0}{unit} over {over}")
    } else {
        format!("{lo:.0}–{hi:.0}{unit} over {over}")
    }
}

/// A chart of `points` (needs at least two) with an optional dashed
/// `threshold` line.
pub(crate) fn chart(points: Vec<(u64, f64)>, threshold: Option<f64>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class("cpu-dashboard-sparkline");
    area.set_hexpand(true);
    area.set_content_height(40);
    let (lo, hi) = value_range(&points, threshold);
    area.set_draw_func(move |area, cr, w, h| {
        if points.len() < 2 {
            return;
        }
        let (w, h) = (w as f64, h as f64);
        let c = area.color();
        let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
        let xy = plot(&points, lo, hi, w, h);

        cr.set_source_rgba(r, g, b, 0.12);
        cr.set_line_width(1.0);
        cr.move_to(0.0, h - 0.5);
        cr.line_to(w, h - 0.5);
        let _ = cr.stroke();

        if let Some(t) = threshold {
            let y = plot(&[(0, t)], lo, hi, w, h)[0].1;
            cr.set_source_rgba(r, g, b, 0.45);
            cr.set_dash(&[3.0, 3.0], 0.0);
            cr.move_to(0.0, y);
            cr.line_to(w, y);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }

        cr.move_to(xy[0].0, h);
        for (x, y) in &xy {
            cr.line_to(*x, *y);
        }
        cr.line_to(xy[xy.len() - 1].0, h);
        cr.close_path();
        let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
        grad.add_color_stop_rgba(0.0, r, g, b, 0.30);
        grad.add_color_stop_rgba(1.0, r, g, b, 0.03);
        if cr.set_source(&grad).is_ok() {
            let _ = cr.fill();
        }

        for (i, (x, y)) in xy.iter().enumerate() {
            if i == 0 {
                cr.move_to(*x, *y);
            } else {
                cr.line_to(*x, *y);
            }
        }
        cr.set_source_rgba(r, g, b, 0.95);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
    });
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plot_scales_time_and_value() {
        let pts = [(100, 40.0), (200, 50.0), (300, 45.0)];
        let xy = plot(&pts, 40.0, 50.0, 100.0, 43.0);
        assert_eq!(xy[0], (0.0, 41.5), "oldest, lowest: bottom-left");
        assert_eq!(xy[1], (50.0, 1.5), "highest: top");
        assert_eq!(xy[2].0, 100.0);
        assert!((xy[2].1 - 21.5).abs() < 1e-9);
        assert!(plot(&[], 0.0, 1.0, 10.0, 10.0).is_empty());
        // One point / zero span doesn't divide by zero.
        assert_eq!(plot(&[(5, 1.0)], 0.0, 2.0, 10.0, 13.0), [(0.0, 6.5)]);
    }

    #[test]
    fn range_includes_the_threshold_and_pads() {
        let pts = [(0, 40.0), (1, 50.0)];
        assert_eq!(value_range(&pts, None), (39.0, 51.0));
        assert_eq!(value_range(&pts, Some(70.0)), (37.0, 73.0));
        assert_eq!(value_range(&[(0, 45.0), (1, 45.0)], None), (44.0, 46.0));
        assert_eq!(value_range(&[], None), (0.0, 1.0));
    }

    #[test]
    fn captions() {
        let day = 86_400;
        assert_eq!(
            caption(&[(0, 38.0), (6 * day, 52.0)], "°C"),
            "38–52°C over 6 days"
        );
        assert_eq!(
            caption(&[(0, 3.0), (3 * 3600, 3.0)], "%"),
            "3% over 3 hours"
        );
        assert_eq!(
            caption(&[(0, 40.0), (30, 41.0)], "°C"),
            "40–41°C over 1 min"
        );
        assert_eq!(caption(&[], "°C"), "");
    }
}
