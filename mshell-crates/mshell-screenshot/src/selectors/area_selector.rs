use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::common::*;
use crate::selectors::window_selector::{WindowRect, build_window_rects};
use crate::utils::find_gdk_monitor;
use gtk4::prelude::*;
use gtk4::{cairo, gdk, glib};
use gtk4_layer_shell::LayerShell;

/// Snap distance in output-local pixels — if the drag's bounding
/// box edges land this close to a window edge, the selection
/// snaps to that window. Loose enough that a sloppy drag still
/// catches the intended window; tight enough that intentional
/// "I want this slice" drags through a window don't get hijacked.
const SNAP_THRESHOLD_PX: f64 = 18.0;

#[derive(Debug, Clone)]
pub struct RegionSelection {
    pub output: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

struct SharedState {
    drag_start: Cell<Option<(f64, f64)>>,
    drag_current: Cell<Option<(f64, f64)>>,
    on_done: RefCell<Option<Box<dyn FnOnce(Result<RegionSelection>)>>>,
    windows: RefCell<Vec<gtk4::Window>>,
    /// Snapshot of every visible window across all outputs taken
    /// once at selector startup. Used by drag-end snap-to-window:
    /// fresh-fetching per drag-end would race with the user
    /// dragging windows around mid-selection, and the visible set
    /// rarely changes inside a single selector lifetime anyway.
    window_rects: Vec<WindowRect>,
}

impl SharedState {
    fn fire(&self, result: Result<RegionSelection>) {
        for window in self.windows.borrow().iter() {
            window.close();
        }
        if let Some(f) = self.on_done.borrow_mut().take() {
            f(result);
        }
    }

    fn is_done(&self) -> bool {
        self.on_done.borrow().is_none()
    }
}

/// Open fullscreen layer-shell overlays on ALL outputs for region selection.
///
/// This returns immediately. When the user drags a region on any monitor
/// (or presses Escape), `on_done` is called with the result. All overlay
/// windows are destroyed automatically.
pub fn select_region<F>(outputs: &[OutputInfo], on_done: F)
where
    F: FnOnce(Result<RegionSelection>) + 'static,
{
    if outputs.is_empty() {
        on_done(Err(ScreenshotError::CaptureFailed(
            "no outputs found".into(),
        )));
        return;
    }

    let state = Rc::new(SharedState {
        drag_start: Cell::new(None),
        drag_current: Cell::new(None),
        on_done: RefCell::new(Some(Box::new(on_done))),
        windows: RefCell::new(Vec::new()),
        window_rects: build_window_rects(),
    });

    let gdk_display = gdk::Display::default().expect("no display");
    let monitors = gdk_display.monitors();

    for output in outputs {
        let gdk_monitor = find_gdk_monitor(&monitors, output);
        let window = create_overlay_window(output, gdk_monitor.as_ref(), &state);
        state.windows.borrow_mut().push(window);
    }

    for window in state.windows.borrow().iter() {
        window.present();
    }
}

/// Create a single overlay window for one output.
fn create_overlay_window(
    output: &OutputInfo,
    gdk_monitor: Option<&gdk::Monitor>,
    state: &Rc<SharedState>,
) -> gtk4::Window {
    let window = gtk4::Window::new();
    window.set_decorated(false);

    // ── Layer shell setup ──
    window.init_layer_shell();
    window.set_layer(gtk4_layer_shell::Layer::Overlay);
    window.set_anchor(gtk4_layer_shell::Edge::Top, true);
    window.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
    window.set_anchor(gtk4_layer_shell::Edge::Left, true);
    window.set_anchor(gtk4_layer_shell::Edge::Right, true);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);
    window.set_namespace(Some("mshell-screenshot"));

    if let Some(monitor) = gdk_monitor {
        window.set_monitor(Some(monitor));
    }

    window.add_css_class("screenshot-overlay");
    window.set_cursor_from_name(Some("crosshair"));

    // ── Drawing area ──
    let drawing_area = gtk4::DrawingArea::new();
    drawing_area.add_css_class("screenshot-area-selector");
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);
    window.set_child(Some(&drawing_area));

    let state_draw = Rc::clone(state);
    let da_for_draw = drawing_area.clone();
    drawing_area.set_draw_func(move |_area, cr, width, height| {
        let color = da_for_draw.color();
        draw_overlay(cr, width, height, &state_draw, &color);
    });

    // ── Drag gesture ──
    let drag = gtk4::GestureDrag::new();

    let state_begin = Rc::clone(state);
    drag.connect_drag_begin(move |_, x, y| {
        state_begin.drag_start.set(Some((x, y)));
        state_begin.drag_current.set(Some((x, y)));
        // Redraw all windows so the non-active ones stay dark.
        for w in state_begin.windows.borrow().iter() {
            if let Some(child) = w.child() {
                child.queue_draw();
            }
        }
    });

    let state_update = Rc::clone(state);
    let da_update = drawing_area.clone();
    drag.connect_drag_update(move |gesture, offset_x, offset_y| {
        if let Some((sx, sy)) = gesture.start_point() {
            let max_w = da_update.width() as f64;
            let max_h = da_update.height() as f64;
            let cx = (sx + offset_x).clamp(0.0, max_w);
            let cy = (sy + offset_y).clamp(0.0, max_h);
            state_update.drag_current.set(Some((cx, cy)));
            da_update.queue_draw();
        }
    });

    let state_end = Rc::clone(state);
    let output_name = output.name.clone();
    let da_end = drawing_area.clone();
    drag.connect_drag_end(move |gesture, offset_x, offset_y| {
        if state_end.is_done() {
            return;
        }
        if let Some((sx, sy)) = gesture.start_point() {
            let max_w = da_end.width() as f64;
            let max_h = da_end.height() as f64;
            let ex = (sx + offset_x).clamp(0.0, max_w);
            let ey = (sy + offset_y).clamp(0.0, max_h);

            let rx = sx.min(ex);
            let ry = sy.min(ey);
            let rw = (ex - sx).abs();
            let rh = (ey - sy).abs();

            if rw > 5.0 && rh > 5.0 {
                // Snap-to-window: if the user's rough drag lines
                // up with a visible window's bbox on this output
                // (every edge within SNAP_THRESHOLD_PX), replace
                // the freehand rect with the window's exact
                // bounds. Shift-modifier holds suppress the snap
                // for cases where the user explicitly wants a
                // slice through a window.
                let shift_held = gesture
                    .current_event_state()
                    .contains(gdk::ModifierType::SHIFT_MASK);
                let (final_x, final_y, final_w, final_h) = if shift_held {
                    (rx, ry, rw, rh)
                } else {
                    snap_to_window(&state_end.window_rects, &output_name, rx, ry, rw, rh)
                        .unwrap_or((rx, ry, rw, rh))
                };

                let region = RegionSelection {
                    output: output_name.clone(),
                    x: final_x as i32,
                    y: final_y as i32,
                    width: final_w as i32,
                    height: final_h as i32,
                };
                state_end.fire(Ok(region));
            } else {
                // Too small — reset and let them try again.
                state_end.drag_start.set(None);
                state_end.drag_current.set(None);
            }
        }
    });

    drawing_area.add_controller(drag);

    // ── Keyboard: Escape to cancel ──
    let key_ctrl = gtk4::EventControllerKey::new();
    let state_key = Rc::clone(state);
    key_ctrl.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gdk::Key::Escape && !state_key.is_done() {
            state_key.fire(Err(ScreenshotError::Cancelled));
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_ctrl);

    // ── Handle unexpected window destruction ──
    let state_destroy = Rc::clone(state);
    window.connect_destroy(move |_| {
        if !state_destroy.is_done() {
            state_destroy.fire(Err(ScreenshotError::Cancelled));
        }
    });

    window
}

/// Draw the dark overlay with a cutout for the current selection.
fn draw_overlay(
    cr: &cairo::Context,
    width: i32,
    height: i32,
    state: &SharedState,
    outline: &gdk::RGBA,
) {
    // Dark overlay.
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
    cr.rectangle(0.0, 0.0, width as f64, height as f64);
    cr.fill().ok();

    if let Some((sx, sy, sw, sh)) = compute_rect(state) {
        // Cut out the selected area.
        cr.set_operator(cairo::Operator::Clear);
        cr.rectangle(sx, sy, sw, sh);
        cr.fill().ok();

        // Border around selection using outline color from CSS.
        cr.set_operator(cairo::Operator::Over);
        cr.set_source_rgba(
            outline.red() as f64,
            outline.green() as f64,
            outline.blue() as f64,
            outline.alpha() as f64,
        );
        cr.set_line_width(2.0);
        cr.rectangle(sx, sy, sw, sh);
        cr.stroke().ok();

        // Dimension + aspect-ratio label. The ratio chunk lands
        // right after the px count so the user reading the badge
        // gets "what is this region exactly" in one glance —
        // useful both for capture sizing (16:9 thumbnail vs 1:1
        // avatar) and for editing decisions (does this fit a
        // socials post format).
        cr.set_font_size(14.0);
        let w_i = sw as i32;
        let h_i = sh as i32;
        let label = format!("{}×{}  ({})", w_i, h_i, aspect_label(w_i, h_i));
        if let Ok(extents) = cr.text_extents(&label) {
            let tx = sx + (sw - extents.width()) / 2.0;
            let ty = sy + sh + 20.0;
            cr.move_to(tx, ty);
            cr.show_text(&label).ok();
        }
    }
}

/// Friendly description of `w:h` — named tag when it matches a
/// common screen / social format within a small ε, otherwise the
/// reduced fraction so e.g. 800×600 reads as "4:3" and 1735×973
/// reads as "1735:973" (the user's odd custom drag). ε is 0.5 %
/// to absorb sub-pixel drift from the drag clamp.
fn aspect_label(w: i32, h: i32) -> String {
    if w <= 0 || h <= 0 {
        return "—".into();
    }
    let ratio = w as f64 / h as f64;
    let presets = [
        ("1:1", 1.0),
        ("4:3", 4.0 / 3.0),
        ("3:2", 3.0 / 2.0),
        ("16:10", 16.0 / 10.0),
        ("16:9", 16.0 / 9.0),
        ("21:9", 21.0 / 9.0),
        ("32:9", 32.0 / 9.0),
        ("9:16", 9.0 / 16.0),
        ("3:4", 3.0 / 4.0),
        ("2:3", 2.0 / 3.0),
    ];
    for (name, target) in presets {
        if (ratio - target).abs() / target < 0.005 {
            return name.into();
        }
    }
    // Fall back to the reduced fraction so odd drags still carry
    // a readable label (e.g. "5:3" for 1920×1152).
    let g = gcd(w.unsigned_abs(), h.unsigned_abs()) as i32;
    format!("{}:{}", w / g.max(1), h / g.max(1))
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

/// Replace a sloppy drag rect with a window's exact bbox when
/// every drag edge lands within `SNAP_THRESHOLD_PX` of that
/// window's edge on the same output. Returns the snapped
/// rectangle on success, `None` when no window qualifies (caller
/// keeps the raw drag rect). When multiple windows qualify the
/// smallest-area one wins so nested layouts (a popup inside a
/// terminal inside a stacked workspace) snap to the most
/// specific match.
fn snap_to_window(
    windows: &[WindowRect],
    output: &str,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
) -> Option<(f64, f64, f64, f64)> {
    let mut best: Option<(&WindowRect, f64)> = None;
    for w in windows {
        if w.output != output {
            continue;
        }
        let edge_left = (rx - w.x).abs();
        let edge_top = (ry - w.y).abs();
        let edge_right = ((rx + rw) - (w.x + w.width)).abs();
        let edge_bottom = ((ry + rh) - (w.y + w.height)).abs();
        if edge_left > SNAP_THRESHOLD_PX
            || edge_top > SNAP_THRESHOLD_PX
            || edge_right > SNAP_THRESHOLD_PX
            || edge_bottom > SNAP_THRESHOLD_PX
        {
            continue;
        }
        let area = w.width * w.height;
        match best {
            None => best = Some((w, area)),
            Some((_, ba)) if area < ba => best = Some((w, area)),
            _ => {}
        }
    }
    best.map(|(w, _)| (w.x, w.y, w.width, w.height))
}

/// Compute the current rectangle from an in-progress drag.
fn compute_rect(state: &SharedState) -> Option<(f64, f64, f64, f64)> {
    let (sx, sy) = state.drag_start.get()?;
    let (cx, cy) = state.drag_current.get()?;
    let x = sx.min(cx);
    let y = sy.min(cy);
    let w = (cx - sx).abs();
    let h = (cy - sy).abs();
    if w > 1.0 && h > 1.0 {
        Some((x, y, w, h))
    } else {
        None
    }
}
