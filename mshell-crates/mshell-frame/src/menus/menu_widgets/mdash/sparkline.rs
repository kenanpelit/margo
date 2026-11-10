//! A small area sparkline (the CPU dashboard's look): a fixed-length
//! history drawn as a filled curve in the widget's CSS `color` (the
//! accent), so it re-themes with the wallpaper.

use relm4::gtk;
use relm4::gtk::prelude::{DrawingAreaExt, DrawingAreaExtManual, WidgetExt};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Samples kept (2 s apart ≈ 1 min).
pub(crate) const SAMPLES: usize = 30;

/// A bounded history of non-negative samples.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct History {
    values: VecDeque<f64>,
}

impl History {
    pub(crate) fn push(&mut self, v: f64) {
        if self.values.len() == SAMPLES {
            self.values.pop_front();
        }
        self.values
            .push_back(if v.is_finite() { v.max(0.0) } else { 0.0 });
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    /// The samples scaled to 0..=1 against `ceiling` (a fixed 100 for
    /// percentages; `None` = the largest sample, for throughput).
    pub(crate) fn normalised(&self, ceiling: Option<f64>) -> Vec<f64> {
        let top = ceiling
            .unwrap_or_else(|| self.values.iter().copied().fold(0.0, f64::max))
            .max(f64::EPSILON);
        self.values
            .iter()
            .map(|v| (v / top).clamp(0.0, 1.0))
            .collect()
    }
}

/// The drawing area + its history.
#[derive(Clone)]
pub(crate) struct Sparkline {
    pub(crate) area: gtk::DrawingArea,
    history: Rc<RefCell<History>>,
}

impl Sparkline {
    /// `ceiling`: see [`History::normalised`].
    pub(crate) fn new(ceiling: Option<f64>) -> Self {
        let area = gtk::DrawingArea::new();
        area.add_css_class("mdash-sparkline");
        area.set_content_height(28);
        area.set_hexpand(true);
        let history: Rc<RefCell<History>> = Rc::default();
        let h = history.clone();
        area.set_draw_func(move |area, cr, w, hgt| {
            let points = h.borrow().normalised(ceiling);
            if points.len() < 2 {
                return;
            }
            let (w, hgt) = (f64::from(w), f64::from(hgt));
            let c = area.color();
            let (r, g, b) = (
                f64::from(c.red()),
                f64::from(c.green()),
                f64::from(c.blue()),
            );
            // Always span the full width with the configured sample count,
            // so a young history grows in from the right.
            let step = w / (SAMPLES - 1) as f64;
            let x0 = w - step * (points.len() - 1) as f64;
            let y = |p: f64| hgt - p * (hgt - 3.0) - 1.5;
            cr.move_to(x0, hgt);
            for (i, p) in points.iter().enumerate() {
                cr.line_to(x0 + i as f64 * step, y(*p));
            }
            cr.line_to(w, hgt);
            cr.close_path();
            let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, hgt);
            grad.add_color_stop_rgba(0.0, r, g, b, 0.30);
            grad.add_color_stop_rgba(1.0, r, g, b, 0.02);
            if cr.set_source(&grad).is_ok() {
                let _ = cr.fill();
            }
            for (i, p) in points.iter().enumerate() {
                let x = x0 + i as f64 * step;
                if i == 0 {
                    cr.move_to(x, y(*p));
                } else {
                    cr.line_to(x, y(*p));
                }
            }
            cr.set_source_rgba(r, g, b, 0.95);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
        });
        Self { area, history }
    }

    pub(crate) fn push(&self, v: f64) {
        self.history.borrow_mut().push(v);
        self.area.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_drops_the_oldest() {
        let mut h = History::default();
        for i in 0..(SAMPLES + 5) {
            h.push(i as f64);
        }
        assert_eq!(h.len(), SAMPLES);
        assert_eq!(h.normalised(Some(100.0))[0], 0.05);
    }

    #[test]
    fn normalising_clamps_and_survives_zero_and_nan() {
        let mut h = History::default();
        h.push(0.0);
        h.push(f64::NAN);
        h.push(150.0);
        assert_eq!(h.normalised(Some(100.0)), [0.0, 0.0, 1.0]);
        let mut flat = History::default();
        flat.push(0.0);
        flat.push(0.0);
        assert_eq!(
            flat.normalised(None),
            [0.0, 0.0],
            "an idle link is a flat line"
        );
    }
}
