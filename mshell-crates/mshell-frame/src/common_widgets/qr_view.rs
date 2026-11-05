//! A QR code on screen: the module map from `mshell_utils::qr` drawn
//! straight onto a `gtk::DrawingArea` (whole-pixel modules, so it stays
//! crisp at any output scale), on a light card with the quiet zone a phone
//! camera needs, plus a caption line for loading / error / "what is this".
//! "Copy image" puts a PNG of the same code on the clipboard.
//!
//! Shared by the Network menu's Wi-Fi / hotspot share and the QR menu.

use mshell_utils::qr::QrMatrix;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, DrawingAreaExt, DrawingAreaExtManual, WidgetExt};
use std::cell::RefCell;
use std::rc::Rc;

/// Light modules of quiet zone around the code (ISO/IEC 18004 asks 4).
const QUIET: usize = 4;
/// Logical side of the drawn code (the card around it adds padding).
const SIDE_PX: i32 = 220;
/// Pixels per module in the copied PNG.
const COPY_MODULE_PX: usize = 8;

/// Integer module size + offset for a `size`-module code (plus quiet zone)
/// in a `w × h` area: `(module_px, x0, y0)`. Whole pixels per module keep
/// the edges sharp; the code is centred.
pub(crate) fn layout(size: usize, w: i32, h: i32) -> (f64, f64, f64) {
    let modules = (size + 2 * QUIET) as f64;
    let side = f64::from(w.min(h)).max(0.0);
    let module = (side / modules).floor().max(1.0);
    let drawn = module * modules;
    let x0 = ((f64::from(w) - drawn) / 2.0).floor() + module * QUIET as f64;
    let y0 = ((f64::from(h) - drawn) / 2.0).floor() + module * QUIET as f64;
    (module, x0, y0)
}

#[derive(Clone)]
pub(crate) struct QrView {
    root: gtk::Box,
    area: gtk::DrawingArea,
    caption: gtk::Label,
    matrix: Rc<RefCell<Option<QrMatrix>>>,
}

impl QrView {
    pub(crate) fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("qr-view");
        let area = gtk::DrawingArea::new();
        area.add_css_class("qr-canvas");
        area.set_content_width(SIDE_PX);
        area.set_content_height(SIDE_PX);
        area.set_halign(gtk::Align::Center);
        area.set_overflow(gtk::Overflow::Hidden);
        area.set_visible(false);
        // Screen readers get the caption, not a picture of squares.
        area.set_can_target(false);
        let matrix: Rc<RefCell<Option<QrMatrix>>> = Rc::default();
        let m = matrix.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let guard = m.borrow();
            let Some(qr) = guard.as_ref() else {
                return;
            };
            // A QR code must be dark on light to scan, whatever the theme:
            // these two colours deliberately do not follow the palette.
            cr.set_source_rgb(1.0, 1.0, 1.0); // design-lint: allow-colour QR codes scan only dark-on-light
            let _ = cr.paint();
            cr.set_source_rgb(0.0, 0.0, 0.0); // design-lint: allow-colour QR codes scan only dark-on-light
            let (module, x0, y0) = layout(qr.size(), w, h);
            for y in 0..qr.size() {
                for x in 0..qr.size() {
                    if qr.dark(x, y) {
                        cr.rectangle(
                            x0 + x as f64 * module,
                            y0 + y as f64 * module,
                            module,
                            module,
                        );
                    }
                }
            }
            let _ = cr.fill();
        });
        let caption = gtk::Label::new(None);
        caption.set_css_classes(&["label-small", "dim-label"]);
        caption.set_wrap(true);
        caption.set_justify(gtk::Justification::Center);
        caption.set_halign(gtk::Align::Center);
        root.append(&area);
        root.append(&caption);
        Self {
            root,
            area,
            caption,
            matrix,
        }
    }

    pub(crate) fn root(&self) -> &gtk::Box {
        &self.root
    }

    /// Show `qr` with `caption` under it.
    pub(crate) fn show(&self, qr: QrMatrix, caption: &str) {
        *self.matrix.borrow_mut() = Some(qr);
        self.area.set_visible(true);
        self.area.queue_draw();
        self.set_caption(caption, false);
    }

    /// No code: just a line (loading, empty, an error in the §2 danger tone).
    pub(crate) fn message(&self, text: &str, error: bool) {
        self.clear();
        self.set_caption(text, error);
    }

    /// Forget the code (a Wi-Fi code is the network password: drop it as
    /// soon as it is off screen).
    pub(crate) fn clear(&self) {
        self.matrix.borrow_mut().take();
        self.area.set_visible(false);
        self.caption.set_label("");
    }

    pub(crate) fn has_code(&self) -> bool {
        self.matrix.borrow().is_some()
    }

    fn set_caption(&self, text: &str, error: bool) {
        self.caption.set_label(text);
        self.caption.set_visible(!text.is_empty());
        if error {
            self.caption.set_css_classes(&["label-small-error"]);
        } else {
            self.caption.set_css_classes(&["label-small", "dim-label"]);
        }
    }

    /// Put the shown code on the clipboard as an image. `false` when
    /// there is nothing to copy.
    pub(crate) fn copy_image(&self) -> bool {
        let Some(qr) = self.matrix.borrow().as_ref().cloned() else {
            return false;
        };
        let (side, px) = qr.to_gray(COPY_MODULE_PX, QUIET);
        let Ok(side_i) = i32::try_from(side) else {
            return false;
        };
        let bytes = gtk::glib::Bytes::from(&px[..]);
        let texture =
            gtk::gdk::MemoryTexture::new(side_i, side_i, gtk::gdk::MemoryFormat::G8, &bytes, side);
        self.root.clipboard().set_texture(&texture);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::layout;

    #[test]
    fn modules_are_whole_pixels_and_centred() {
        // 21 modules + 2×4 quiet = 29; 220 px → 7 px modules, 203 px drawn.
        let (m, x0, y0) = layout(21, 220, 220);
        assert_eq!(m, 7.0);
        assert_eq!(x0, 8.0 + 28.0);
        assert_eq!(y0, x0);
        // A wide area centres horizontally.
        let (m, x0, y0) = layout(21, 400, 220);
        assert_eq!(m, 7.0);
        assert!(x0 > y0);
        // Never below one pixel, even when squeezed.
        assert_eq!(layout(177, 10, 10).0, 1.0);
    }
}
