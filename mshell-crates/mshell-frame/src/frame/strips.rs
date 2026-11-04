//! The edge-strip surfaces: the top and bottom bars with their part of the
//! frame, and the thin left/right border strips. See [`super::pieces`] for
//! who paints what.

use super::pieces::{
    PieceInputs, PieceMode, edge_overhang, idle_style, piece_viewports, side_strip_width,
};
use crate::frame_draw_widget::{FrameDrawWidget, FrameStyle};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use mshell_common::box_with_resize::BoxWithResize;
use relm4::gtk::{self, gdk, glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;

/// Mutable sizes shared by the strip callbacks.
#[derive(Debug)]
struct StripState {
    top_band: Cell<f64>,
    bottom_band: Cell<f64>,
    top_strip_height: Cell<f64>,
    bottom_strip_height: Cell<f64>,
    overhang: Cell<i32>,
    side_width: Cell<i32>,
    mode: Cell<PieceMode>,
}

/// The four strip windows of one output, plus a handle on the overlay's draw
/// widget so one `relayout` places every piece.
pub(super) struct EdgeStrips {
    monitor: gdk::Monitor,
    windows: [gtk::Window; 4],
    top_draw: FrameDrawWidget,
    bottom_draw: FrameDrawWidget,
    left_draw: FrameDrawWidget,
    right_draw: FrameDrawWidget,
    overlay_draw: FrameDrawWidget,
    /// The strips' `gtk::Overlay`s, which carry `.frame-disabled` like the
    /// main overlay so the bar CSS matches.
    top_overlay: gtk::Overlay,
    bottom_overlay: gtk::Overlay,
    top_outer: BoxWithResize,
    bottom_outer: BoxWithResize,
    top_content: gtk::Box,
    bottom_content: gtk::Box,
    top_filler: gtk::Box,
    bottom_filler: gtk::Box,
    /// The boxes holding the bar widgets; their height is mirrored into the
    /// overlay's placeholders so menus sit where they did.
    pub top_bar_box: BoxWithResize,
    pub bottom_bar_box: BoxWithResize,
    state: StripState,
    geometry_handler: std::cell::RefCell<Option<glib::SignalHandlerId>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StripEdge {
    Top,
    Bottom,
    Left,
    Right,
}

fn strip_window(monitor: &gdk::Monitor, edge: StripEdge) -> gtk::Window {
    let window = gtk::Window::new();
    window.set_css_classes(&["frame-window", "window-opacity"]);
    window.init_layer_shell();
    window.set_monitor(Some(monitor));
    // Same namespace as the overlay: compositor rules for the frame apply to
    // every piece of it.
    window.set_namespace(Some("mshell-frame"));
    window.set_layer(Layer::Top);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_decorated(false);
    let (top, bottom, left, right) = match edge {
        StripEdge::Top => (true, false, true, true),
        StripEdge::Bottom => (false, true, true, true),
        StripEdge::Left => (true, true, true, false),
        StripEdge::Right => (true, true, false, true),
    };
    window.set_anchor(Edge::Top, top);
    window.set_anchor(Edge::Bottom, bottom);
    window.set_anchor(Edge::Left, left);
    window.set_anchor(Edge::Right, right);
    window.set_cursor_from_name(Some("default"));
    window
}

/// A bar strip: the draw widget under a vertical box of the bar and a
/// filler as tall as the frame's overhang below (above) the band.
fn bar_strip(
    window: &gtk::Window,
    bar: &gtk::Widget,
    edge: StripEdge,
) -> (
    FrameDrawWidget,
    gtk::Overlay,
    BoxWithResize,
    gtk::Box,
    gtk::Box,
    BoxWithResize,
) {
    let draw = FrameDrawWidget::new();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&draw));
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let bar_box = BoxWithResize::new();
    bar_box.set_hexpand(true);
    bar_box.append(bar);
    let filler = gtk::Box::new(gtk::Orientation::Vertical, 0);
    filler.set_can_target(false);
    if edge == StripEdge::Top {
        content.append(&bar_box);
        content.append(&filler);
    } else {
        content.append(&filler);
        content.append(&bar_box);
    }
    // A resize-reporting wrapper, so the strip's height is known.
    let outer = BoxWithResize::new();
    outer.append(&content);
    if edge == StripEdge::Bottom {
        outer.set_valign(gtk::Align::End);
    }
    overlay.add_overlay(&outer);
    // The window takes the content's size, not just the draw widget's.
    overlay.set_measure_overlay(&outer, true);
    // The strip's height for the viewport is the window's real content
    // height, measured around the whole overlay. `outer`'s own allocation can
    // be shorter than the window (the bottom strip then sat its frame band a
    // radius too high, leaving the output's last rows unpainted).
    let sized = BoxWithResize::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    sized.append(&overlay);
    window.set_child(Some(&sized));
    (draw, overlay, sized, content, filler, bar_box)
}

impl EdgeStrips {
    pub(super) fn new(
        monitor: &gdk::Monitor,
        top_bar: &gtk::Widget,
        bottom_bar: &gtk::Widget,
        overlay_draw: &FrameDrawWidget,
    ) -> Rc<Self> {
        let top_window = strip_window(monitor, StripEdge::Top);
        let bottom_window = strip_window(monitor, StripEdge::Bottom);
        let left_window = strip_window(monitor, StripEdge::Left);
        let right_window = strip_window(monitor, StripEdge::Right);
        let (top_draw, top_overlay, top_outer, top_content, top_filler, top_bar_box) =
            bar_strip(&top_window, top_bar, StripEdge::Top);
        let (
            bottom_draw,
            bottom_overlay,
            bottom_outer,
            bottom_content,
            bottom_filler,
            bottom_bar_box,
        ) = bar_strip(&bottom_window, bottom_bar, StripEdge::Bottom);
        let left_draw = FrameDrawWidget::new();
        left_window.set_child(Some(&left_draw));
        let right_draw = FrameDrawWidget::new();
        right_window.set_child(Some(&right_draw));

        // Until the theme CSS resolves, assume the widget defaults
        // (24 px radius, 2 px border); `metrics-changed` corrects it.
        let defaults = FrameStyle::default();
        let strips = Rc::new(Self {
            monitor: monitor.clone(),
            windows: [top_window, bottom_window, left_window, right_window],
            top_draw,
            bottom_draw,
            left_draw,
            right_draw,
            overlay_draw: overlay_draw.clone(),
            top_overlay,
            bottom_overlay,
            top_outer,
            bottom_outer,
            top_content,
            bottom_content,
            top_filler,
            bottom_filler,
            top_bar_box,
            bottom_bar_box,
            state: StripState {
                top_band: Cell::new(0.0),
                bottom_band: Cell::new(0.0),
                top_strip_height: Cell::new(0.0),
                bottom_strip_height: Cell::new(0.0),
                overhang: Cell::new(edge_overhang(defaults.border_radius, defaults.border_width)),
                side_width: Cell::new(side_strip_width(defaults.border_width)),
                // No menu is open at start, and the overlay is unmapped.
                mode: Cell::new(PieceMode::Idle),
            },
            geometry_handler: std::cell::RefCell::new(None),
        });
        for draw in strips.draws() {
            // Strips never show menus, and the bands come from the bars'
            // reserve (`set_band`), not the widget's 50 px defaults.
            draw.update_style(|s| {
                *s = idle_style(s);
                s.top_thickness = 0.0;
                s.bottom_thickness = 0.0;
            });
        }
        strips.apply_sizes();

        // The strips' own heights decide where the bottom strip sits and
        // where the side strips start. Relayout in the callback (not via the
        // component sender) so the viewport is current in the same frame.
        for (outer, top) in [(&strips.top_outer, true), (&strips.bottom_outer, false)] {
            let weak = Rc::downgrade(&strips);
            outer.connect_local("resized", false, move |values: &[glib::Value]| {
                let this = weak.upgrade()?;
                let height = f64::from(super::signal_i32(values, 2));
                let cell = if top {
                    &this.state.top_strip_height
                } else {
                    &this.state.bottom_strip_height
                };
                if cell.get() != height {
                    cell.set(height);
                    this.relayout();
                }
                None
            });
        }
        {
            let weak = Rc::downgrade(&strips);
            strips
                .top_draw
                .connect_local("metrics-changed", false, move |values| {
                    let this = weak.upgrade()?;
                    let radius = values.get(1).and_then(|v| v.get::<f64>().ok());
                    let width = values.get(2).and_then(|v| v.get::<f64>().ok());
                    if let (Some(radius), Some(width)) = (radius, width) {
                        this.state.overhang.set(edge_overhang(radius, width));
                        this.state.side_width.set(side_strip_width(width));
                        this.apply_sizes();
                        this.relayout();
                    }
                    None
                });
        }
        {
            let weak = Rc::downgrade(&strips);
            let id = monitor.connect_geometry_notify(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.relayout();
                }
            });
            strips.geometry_handler.replace(Some(id));
        }

        for window in &strips.windows {
            window.set_visible(true);
        }
        strips.relayout();
        strips
    }

    /// Push the size-dependent requests: filler = overhang, content at
    /// least band + overhang, side strips as wide as the border.
    fn apply_sizes(&self) {
        let overhang = self.state.overhang.get();
        self.top_filler.set_height_request(overhang);
        self.bottom_filler.set_height_request(overhang);
        self.top_content
            .set_height_request(self.state.top_band.get().ceil() as i32 + overhang);
        self.bottom_content
            .set_height_request(self.state.bottom_band.get().ceil() as i32 + overhang);
        let side = self.state.side_width.get();
        self.left_draw.set_width_request(side);
        self.right_draw.set_width_request(side);
    }

    /// Recompute every piece's viewport from the current sizes and mode.
    pub(super) fn relayout(&self) {
        let geometry = self.monitor.geometry();
        let inputs = PieceInputs {
            output_width: f64::from(geometry.width()),
            output_height: f64::from(geometry.height()),
            top_band: self.state.top_band.get(),
            bottom_band: self.state.bottom_band.get(),
            top_strip_height: self.state.top_strip_height.get(),
            bottom_strip_height: self.state.bottom_strip_height.get(),
            side_width: f64::from(self.state.side_width.get()),
        };
        let vps = piece_viewports(&inputs, self.state.mode.get());
        // The overlay is unmapped in Idle. While it maps or goes away it
        // keeps its Menu viewport, so for the frame or two of the hand-over
        // both it and the strips paint the corners (invisible with an opaque
        // frame colour) instead of neither.
        self.overlay_draw
            .set_viewport(piece_viewports(&inputs, PieceMode::Menu).overlay);
        self.top_draw.set_viewport(vps.top);
        self.bottom_draw.set_viewport(vps.bottom);
        self.left_draw.set_viewport(vps.left);
        self.right_draw.set_viewport(vps.right);
    }

    pub(super) fn set_mode(&self, mode: PieceMode) {
        if self.state.mode.get() != mode {
            self.state.mode.set(mode);
            self.relayout();
        }
    }

    /// Every strip's draw widget.
    fn draws(&self) -> [&FrameDrawWidget; 4] {
        [
            &self.top_draw,
            &self.bottom_draw,
            &self.left_draw,
            &self.right_draw,
        ]
    }

    /// The bars' reserved height drives the painted band, as on the overlay.
    pub(super) fn set_band(&self, is_top: bool, height: f64) {
        if is_top {
            self.state.top_band.set(height);
        } else {
            self.state.bottom_band.set(height);
        }
        for draw in self.draws() {
            draw.update_style(|s| {
                if is_top {
                    s.top_thickness = height;
                } else {
                    s.bottom_thickness = height;
                }
            });
        }
        self.apply_sizes();
        self.relayout();
    }

    /// Mirror the overlay's frame style switches onto the strips.
    pub(super) fn set_draw_frame(&self, draw_frame: bool) {
        for draw in self.draws() {
            draw.update_style(|s| {
                *s = idle_style(s);
                s.draw_frame = draw_frame;
            });
        }
        for overlay in [&self.top_overlay, &self.bottom_overlay] {
            if draw_frame {
                overlay.remove_css_class("frame-disabled");
            } else {
                overlay.add_css_class("frame-disabled");
            }
        }
    }

    pub(super) fn queue_draw(&self) {
        for draw in self.draws() {
            draw.queue_draw();
        }
    }

    pub(super) fn destroy(&self) {
        if let Some(id) = self.geometry_handler.take() {
            self.monitor.disconnect(id);
        }
        for window in &self.windows {
            window.destroy();
        }
    }
}

/// Keep `placeholder` as tall as the bar box, so the overlay's menus start
/// exactly where the bar ends, as when the bar lived in the overlay.
pub(super) fn mirror_height(bar_box: &BoxWithResize, placeholder: &gtk::Box) {
    let placeholder = placeholder.downgrade();
    bar_box.connect_local("resized", false, move |values: &[glib::Value]| {
        let height = super::signal_i32(values, 2);
        if let Some(p) = placeholder.upgrade()
            && p.height_request() != height
        {
            p.set_height_request(height);
        }
        None
    });
}
