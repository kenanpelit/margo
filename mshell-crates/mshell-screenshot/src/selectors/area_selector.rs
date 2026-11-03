use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::OutputTarget;
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

/// What the selector wants the caller to do with the rect once
/// the user commits. `None` is the default — the caller (e.g.
/// `take_screenshot`) keeps whatever `OutputTarget` it was
/// originally given. `Some(target)` is a user-driven override
/// triggered by an in-selector shortcut: Ctrl+S forces
/// `OutputTarget::File`, Ctrl+E forces `OutputTarget::EditAndSave`,
/// so the user can re-purpose any "Area" capture button into a
/// save-to-disk or open-in-satty flow without backing out and
/// reconfiguring the widget.
pub type RegionCommit = (RegionSelection, Option<OutputTarget>);

struct SharedState {
    drag_start: Cell<Option<(f64, f64)>>,
    drag_current: Cell<Option<(f64, f64)>>,
    on_done: RefCell<Option<Box<dyn FnOnce(Result<RegionCommit>)>>>,
    windows: RefCell<Vec<gtk4::Window>>,
    /// Snapshot of every visible window across all outputs taken
    /// once at selector startup. Used by drag-end snap-to-window:
    /// fresh-fetching per drag-end would race with the user
    /// dragging windows around mid-selection, and the visible set
    /// rarely changes inside a single selector lifetime anyway.
    window_rects: Vec<WindowRect>,
    /// Preview-state selection: set when the drag finishes with a
    /// valid (size > 5) rect. Drives the sticky highlight that
    /// lets the user nudge with arrows or commit with Enter
    /// instead of being forced to release-commit on the spot.
    /// `(output_name, x, y, width, height)` — coordinates are
    /// output-local for the named output.
    selection: RefCell<Option<(String, f64, f64, f64, f64)>>,
    /// Output-name → (width, height) snapshot taken at startup,
    /// used by the arrow-nudge handler to clamp the preview-state
    /// rect to the owning output's bounds (so the user can't
    /// slide the selection past the screen edge).
    output_dims: std::collections::HashMap<String, (f64, f64)>,
}

impl SharedState {
    fn fire(&self, result: Result<RegionCommit>) {
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
    F: FnOnce(Result<RegionCommit>) + 'static,
{
    if outputs.is_empty() {
        on_done(Err(ScreenshotError::CaptureFailed(
            "no outputs found".into(),
        )));
        return;
    }

    let output_dims: std::collections::HashMap<String, (f64, f64)> = outputs
        .iter()
        .map(|o| (o.name.clone(), (o.width as f64, o.height as f64)))
        .collect();

    let state = Rc::new(SharedState {
        drag_start: Cell::new(None),
        drag_current: Cell::new(None),
        on_done: RefCell::new(Some(Box::new(on_done))),
        windows: RefCell::new(Vec::new()),
        window_rects: build_window_rects(),
        selection: RefCell::new(None),
        output_dims,
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
    // Exclusive (not OnDemand): the region selector is a modal capture tool
    // that must own the keyboard for the whole drag→Enter/Esc cycle. OnDemand
    // only grabs focus on pointer interaction and loses to any Exclusive menu
    // (dashboard / quick-settings) left open underneath — so Enter never
    // committed the selection while a layer-shell menu was up. Exclusive on the
    // Overlay layer outranks the Top-layer menu in the compositor's focus
    // policy. All overlays share one selection, so whichever wins focus commits.
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::Exclusive);
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
    let output_for_draw = output.name.clone();
    drawing_area.set_draw_func(move |_area, cr, width, height| {
        let color = da_for_draw.color();
        draw_overlay(cr, width, height, &state_draw, &color, &output_for_draw);
    });

    // ── Drag gesture ──
    let drag = gtk4::GestureDrag::new();

    let state_begin = Rc::clone(state);
    drag.connect_drag_begin(move |_, x, y| {
        state_begin.drag_start.set(Some((x, y)));
        state_begin.drag_current.set(Some((x, y)));
        // A fresh drag wipes the preview-state selection — the
        // user is starting over.
        *state_begin.selection.borrow_mut() = None;
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

            let (rx, ry, rw, rh) = normalized_rect((sx, sy), (ex, ey));

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

                // Stash the selection into preview state instead
                // of firing the callback immediately. The user can
                // now nudge it with arrows or commit it with Enter
                // — see the keyboard handler below. Drag itself
                // collapses (drag_start/current cleared) so a
                // fresh drag starts a brand new rect rather than
                // extending the preview.
                state_end.drag_start.set(None);
                state_end.drag_current.set(None);
                *state_end.selection.borrow_mut() =
                    Some((output_name.clone(), final_x, final_y, final_w, final_h));
                for w in state_end.windows.borrow().iter() {
                    if let Some(child) = w.child() {
                        child.queue_draw();
                    }
                }
            } else {
                // Too small — reset and let them try again.
                state_end.drag_start.set(None);
                state_end.drag_current.set(None);
            }
        }
    });

    drawing_area.add_controller(drag);

    // ── Keyboard handlers ──
    //   • Escape         → cancel (always).
    //   • Enter / KP_Enter → commit the preview-state selection
    //     (the one drag_end stashed) and close the overlays.
    //   • Arrow keys     → nudge the preview-state selection by
    //     1 px (Shift+arrow = 10 px). The selection rect itself
    //     moves; size stays constant. Allocated area is clamped
    //     to the owning output's bounds so the user can't slide
    //     it past the screen edge.
    let key_ctrl = gtk4::EventControllerKey::new();
    let state_key = Rc::clone(state);
    key_ctrl.connect_key_pressed(move |_, keyval, _, modifier| {
        if state_key.is_done() {
            return glib::Propagation::Proceed;
        }
        match keyval {
            gdk::Key::Escape => {
                state_key.fire(Err(ScreenshotError::Cancelled));
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => fire_preview(&state_key, None),
            // Ctrl+S → commit but force the OutputTarget to File
            // (save-to-disk, no clipboard, no editor) regardless
            // of whatever target the calling widget originally
            // wanted. Lets the user repurpose any "Area" button
            // into a quick save without backing out.
            gdk::Key::S | gdk::Key::s if modifier.contains(gdk::ModifierType::CONTROL_MASK) => {
                fire_preview(&state_key, Some(OutputTarget::File))
            }
            // Ctrl+E → commit + force EditAndSave (open in
            // satty / swappy). Same override mechanic as Ctrl+S.
            gdk::Key::E | gdk::Key::e if modifier.contains(gdk::ModifierType::CONTROL_MASK) => {
                fire_preview(&state_key, Some(OutputTarget::EditAndSave(None)))
            }
            gdk::Key::Up | gdk::Key::Down | gdk::Key::Left | gdk::Key::Right => {
                let step = if modifier.contains(gdk::ModifierType::SHIFT_MASK) {
                    10.0
                } else {
                    1.0
                };
                let (dx, dy) = match keyval {
                    gdk::Key::Up => (0.0, -step),
                    gdk::Key::Down => (0.0, step),
                    gdk::Key::Left => (-step, 0.0),
                    gdk::Key::Right => (step, 0.0),
                    _ => unreachable!(),
                };
                if nudge_selection(&state_key, dx, dy) {
                    for w in state_key.windows.borrow().iter() {
                        if let Some(child) = w.child() {
                            child.queue_draw();
                        }
                    }
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
            _ => glib::Propagation::Proceed,
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
    this_output: &str,
) {
    // Dark overlay.
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.45); // design-lint: allow-colour neutral scrim over arbitrary screen content
    cr.rectangle(0.0, 0.0, width as f64, height as f64);
    cr.fill().ok();

    if let Some((sx, sy, sw, sh)) = current_rect_for(state, this_output) {
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

/// Friendly description of `w:h` — the named format whose ratio is
/// *nearest* to it, when that is within [`ASPECT_TOLERANCE`], otherwise
/// the reduced fraction so e.g. 800×600 reads as "4:3" and 1700×973
/// reads as "1700:973" (the user's odd custom drag).
///
/// "21:9" is a marketing name: real ultrawide panels are 64:27
/// (2560×1080), 43:18 (3440×1440) or 12:5 (3840×1600), all 1.6–2.9 %
/// off 21/9 itself, so each of those ratios is listed under that name.
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
        ("21:9", 64.0 / 27.0),
        ("21:9", 43.0 / 18.0),
        ("21:9", 12.0 / 5.0),
        ("32:9", 32.0 / 9.0),
        ("9:16", 9.0 / 16.0),
        ("3:4", 3.0 / 4.0),
        ("2:3", 2.0 / 3.0),
    ];
    let nearest = presets
        .iter()
        .map(|&(name, target)| (name, (ratio - target).abs() / target))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((name, err)) = nearest
        && err <= ASPECT_TOLERANCE
    {
        return name.into();
    }
    // Fall back to the reduced fraction so odd drags still carry
    // a readable label (e.g. "5:3" for 1920×1152).
    let g = gcd(w.unsigned_abs(), h.unsigned_abs()) as i32;
    format!("{}:{}", w / g.max(1), h / g.max(1))
}

/// Relative ratio error still read as a named format: 0.5 % absorbs the
/// sub-pixel drift of a drag clamp without swallowing genuinely odd drags.
const ASPECT_TOLERANCE: f64 = 0.005;

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

/// Common commit path used by Enter, Ctrl+S, and Ctrl+E. Reads
/// the preview-state selection, builds a `RegionSelection`, and
/// fires the on_done callback with the (possibly empty) target
/// override. `override_target` is `None` for Enter (caller's
/// chosen target wins) and `Some(t)` for Ctrl+S / Ctrl+E (the
/// shortcut overrides whatever target the caller passed in).
fn fire_preview(state: &SharedState, override_target: Option<OutputTarget>) -> glib::Propagation {
    let region = state
        .selection
        .borrow()
        .as_ref()
        .map(|(o, x, y, w, h)| RegionSelection {
            output: o.clone(),
            x: *x as i32,
            y: *y as i32,
            width: *w as i32,
            height: *h as i32,
        });
    if let Some(region) = region {
        state.fire(Ok((region, override_target)));
        glib::Propagation::Stop
    } else {
        glib::Propagation::Proceed
    }
}

/// Nudge the preview-state selection by `(dx, dy)` pixels and
/// clamp the result to the owning output's bounds. Returns true
/// when the selection moved (so the caller knows to queue a
/// redraw); false when there's no selection yet or the rect was
/// already pinned against the requested edge.
fn nudge_selection(state: &SharedState, dx: f64, dy: f64) -> bool {
    let mut sel = state.selection.borrow_mut();
    let Some((out, x, y, w, h)) = sel
        .as_mut()
        .map(|s| (s.0.clone(), &mut s.1, &mut s.2, &mut s.3, &mut s.4))
    else {
        return false;
    };
    let bounds = state
        .output_dims
        .get(&out)
        .copied()
        .unwrap_or((f64::INFINITY, f64::INFINITY));
    let Some((new_x, new_y)) = nudged_origin((*x, *y, *w, *h), (dx, dy), bounds) else {
        return false;
    };
    *x = new_x;
    *y = new_y;
    true
}

/// Origin of rect `(x, y, w, h)` moved by `(dx, dy)` and clamped so the
/// rect stays inside `(0, 0)..bounds`. `None` when it can't move (already
/// pinned against that edge, or a zero step).
fn nudged_origin(
    (x, y, w, h): (f64, f64, f64, f64),
    (dx, dy): (f64, f64),
    (max_w, max_h): (f64, f64),
) -> Option<(f64, f64)> {
    let new_x = (x + dx).clamp(0.0, (max_w - w).max(0.0));
    let new_y = (y + dy).clamp(0.0, (max_h - h).max(0.0));
    if (new_x - x).abs() < f64::EPSILON && (new_y - y).abs() < f64::EPSILON {
        return None;
    }
    Some((new_x, new_y))
}

/// `(x, y, w, h)` of the rect spanned by two drag corners, whichever
/// direction the drag went.
fn normalized_rect((sx, sy): (f64, f64), (ex, ey): (f64, f64)) -> (f64, f64, f64, f64) {
    (sx.min(ex), sy.min(ey), (ex - sx).abs(), (ey - sy).abs())
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

/// Resolve which rectangle a given output's overlay should draw.
/// Preview-state selection wins when it's set for THIS output —
/// that's the post-drag-end rect the user is nudging with arrow
/// keys. Otherwise fall back to the in-progress drag rect (which
/// is global state; on a multi-output drag every overlay sees
/// it, but renders it at the output-local coords that match the
/// drag origin). Returns `None` for outputs that aren't the
/// selection owner and have no active drag — those overlays
/// stay pure dark masks.
fn current_rect_for(state: &SharedState, this_output: &str) -> Option<(f64, f64, f64, f64)> {
    if let Some((owner, sx, sy, sw, sh)) = state.selection.borrow().as_ref() {
        if owner == this_output {
            return Some((*sx, *sy, *sw, *sh));
        }
        return None;
    }
    let start = state.drag_start.get()?;
    let current = state.drag_current.get()?;
    let (x, y, w, h) = normalized_rect(start, current);
    if w > 1.0 && h > 1.0 {
        Some((x, y, w, h))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(output: &str, x: f64, y: f64, width: f64, height: f64) -> WindowRect {
        WindowRect {
            output: output.into(),
            address: format!("{output}@{x},{y}"),
            x,
            y,
            width,
            height,
            global_x: x as i32,
            global_y: y as i32,
            global_w: width as i32,
            global_h: height as i32,
        }
    }

    #[test]
    fn aspect_label_names_common_formats() {
        let table: &[((i32, i32), &str)] = &[
            ((1920, 1080), "16:9"),
            ((1280, 720), "16:9"),
            ((1921, 1080), "16:9"),
            ((1080, 1920), "9:16"),
            ((500, 500), "1:1"),
            ((800, 600), "4:3"),
            ((600, 800), "3:4"),
            ((1920, 1200), "16:10"),
            ((2520, 1080), "21:9"),
            // Real ultrawides — none is exactly 21:9.
            ((2560, 1080), "21:9"),
            ((3440, 1440), "21:9"),
            ((3840, 1600), "21:9"),
            ((5120, 2160), "21:9"),
            ((5120, 1440), "32:9"),
            ((1500, 1000), "3:2"),
            ((1000, 1500), "2:3"),
        ];
        for ((w, h), want) in table {
            assert_eq!(aspect_label(*w, *h), *want, "{w}x{h}");
        }
    }

    #[test]
    fn aspect_label_reduces_odd_drags() {
        assert_eq!(aspect_label(1920, 1152), "5:3");
        assert_eq!(aspect_label(1700, 973), "1700:973");
        // Within 0.5 % of 16:9 still reads as the preset.
        assert_eq!(aspect_label(1735, 973), "16:9");
        assert_eq!(aspect_label(7, 1), "7:1");
        assert_eq!(aspect_label(1, 7), "1:7");
    }

    #[test]
    fn aspect_label_of_degenerate_sizes() {
        assert_eq!(aspect_label(0, 100), "—");
        assert_eq!(aspect_label(100, 0), "—");
        assert_eq!(aspect_label(-16, 9), "—");
    }

    #[test]
    fn gcd_basics() {
        assert_eq!(gcd(1920, 1080), 120);
        assert_eq!(gcd(17, 5), 1);
        assert_eq!(gcd(0, 9), 9);
        assert_eq!(gcd(9, 0), 9);
        assert_eq!(gcd(0, 0), 1, "never 0, so callers can divide by it");
    }

    #[test]
    fn normalized_rect_handles_every_drag_direction() {
        let want = (10.0, 20.0, 30.0, 40.0);
        assert_eq!(normalized_rect((10.0, 20.0), (40.0, 60.0)), want);
        assert_eq!(normalized_rect((40.0, 60.0), (10.0, 20.0)), want);
        assert_eq!(normalized_rect((40.0, 20.0), (10.0, 60.0)), want);
        assert_eq!(normalized_rect((10.0, 60.0), (40.0, 20.0)), want);
        assert_eq!(
            normalized_rect((5.0, 5.0), (5.0, 5.0)),
            (5.0, 5.0, 0.0, 0.0)
        );
    }

    #[test]
    fn nudge_moves_and_clamps_to_the_output() {
        let r = (100.0, 100.0, 200.0, 100.0);
        let out = (1920.0, 1080.0);
        assert_eq!(nudged_origin(r, (1.0, 0.0), out), Some((101.0, 100.0)));
        assert_eq!(nudged_origin(r, (0.0, -10.0), out), Some((100.0, 90.0)));
        assert_eq!(
            nudged_origin(r, (-500.0, 0.0), out),
            Some((0.0, 100.0)),
            "clamped at the left edge"
        );
        assert_eq!(
            nudged_origin(r, (5000.0, 5000.0), out),
            Some((1720.0, 980.0)),
            "the far edge keeps the whole rect on the output"
        );
    }

    #[test]
    fn nudge_reports_no_move_when_pinned() {
        let out = (1920.0, 1080.0);
        assert_eq!(
            nudged_origin((0.0, 0.0, 10.0, 10.0), (-1.0, -1.0), out),
            None
        );
        assert_eq!(
            nudged_origin((1910.0, 50.0, 10.0, 10.0), (10.0, 0.0), out),
            None
        );
        assert_eq!(nudged_origin((5.0, 5.0, 10.0, 10.0), (0.0, 0.0), out), None);
        // A rect larger than the output pins at the origin.
        assert_eq!(
            nudged_origin((0.0, 0.0, 4000.0, 10.0), (1.0, 0.0), out),
            None
        );
    }

    #[test]
    fn nudge_without_known_output_bounds_is_unclamped_on_the_far_side() {
        let inf = (f64::INFINITY, f64::INFINITY);
        assert_eq!(
            nudged_origin((10.0, 10.0, 5.0, 5.0), (1e6, 0.0), inf),
            Some((1e6 + 10.0, 10.0))
        );
        assert_eq!(
            nudged_origin((10.0, 10.0, 5.0, 5.0), (-20.0, 0.0), inf),
            Some((0.0, 10.0))
        );
    }

    #[test]
    fn snap_picks_a_window_whose_edges_are_all_close() {
        let windows = [win("DP-1", 100.0, 100.0, 800.0, 600.0)];
        assert_eq!(
            snap_to_window(&windows, "DP-1", 110.0, 95.0, 785.0, 610.0),
            Some((100.0, 100.0, 800.0, 600.0))
        );
    }

    #[test]
    fn snap_rejects_far_edges_and_other_outputs() {
        let windows = [win("DP-1", 100.0, 100.0, 800.0, 600.0)];
        // Right edge 30 px short: a deliberate slice, keep the drag.
        assert_eq!(
            snap_to_window(&windows, "DP-1", 100.0, 100.0, 770.0, 600.0),
            None
        );
        assert_eq!(
            snap_to_window(&windows, "HDMI-A-1", 100.0, 100.0, 800.0, 600.0),
            None
        );
        assert_eq!(snap_to_window(&[], "DP-1", 0.0, 0.0, 10.0, 10.0), None);
    }

    #[test]
    fn snap_threshold_is_inclusive() {
        let windows = [win("DP-1", 100.0, 100.0, 800.0, 600.0)];
        let t = SNAP_THRESHOLD_PX;
        assert!(snap_to_window(&windows, "DP-1", 100.0 + t, 100.0, 800.0 - t, 600.0).is_some());
        assert!(
            snap_to_window(
                &windows,
                "DP-1",
                100.0 + t + 0.5,
                100.0,
                800.0 - t - 0.5,
                600.0
            )
            .is_none()
        );
    }

    #[test]
    fn snap_prefers_the_smallest_matching_window() {
        let windows = [
            win("DP-1", 0.0, 0.0, 1000.0, 800.0),
            win("DP-1", 5.0, 5.0, 990.0, 790.0),
            win("DP-1", 10.0, 10.0, 985.0, 785.0),
        ];
        assert_eq!(
            snap_to_window(&windows, "DP-1", 5.0, 5.0, 990.0, 790.0),
            Some((10.0, 10.0, 985.0, 785.0))
        );
    }
}
