//! Shared drag-to-reorder wiring for Settings lists.
//!
//! Uses a plain `GtkGestureDrag` on each row's ≡ grip handle — **not**
//! GTK drag-and-drop. GtkListBox swallows real DnD motion/drop before its
//! rows see it, which made `DragSource`/`DropTarget` reordering
//! unreliable; a gesture on the grip has none of that baggage.
//!
//! Grab the grip and drag vertically: the live drop indicator follows the
//! **cursor's real position** by hit-testing the sibling rows' actual
//! geometry (`compute_bounds`), so it tracks 1:1 no matter how tall a row
//! is. (The previous fixed `STEP_PX` step advanced the indicator faster
//! than the cursor whenever rows were taller than the step — the "moves
//! faster than the cursor" bug.) On release, `on_drag(delta)` gets the
//! signed number of positions to move; the caller clamps it.
//!
//! **Live feedback (during the drag):** the grabbed row dims (`.dragging`)
//! and the row under the cursor is highlighted (`.drop-target`). This is
//! purely visual (no data/config change mid-drag), found by walking the
//! position element's siblings, so it works the same whether the list is
//! a `GtkListBox` (rows are `ListBoxRow`s) or a plain `gtk::Box`.

use std::cell::Cell;
use std::rc::Rc;

use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::prelude::*;

/// Clear `.drop-target` from every sibling of `pos` (incl. itself).
fn clear_drop_targets(pos: &gtk::Widget) {
    if let Some(parent) = pos.parent() {
        let mut child = parent.first_child();
        while let Some(w) = child {
            w.remove_css_class("drop-target");
            child = w.next_sibling();
        }
    }
}

/// Index of `target` among `parent`'s children, or `None`.
fn sibling_index(parent: &gtk::Widget, target: &gtk::Widget) -> Option<i32> {
    let mut i = 0;
    let mut child = parent.first_child();
    while let Some(w) = child {
        if &w == target {
            return Some(i);
        }
        i += 1;
        child = w.next_sibling();
    }
    None
}

/// `(index, count)` of the sibling whose vertical extent contains `y`
/// (in `parent` coordinates), clamped to `[0, count-1]`. This is what
/// makes the indicator move in lock-step with the cursor: it picks the
/// row the cursor is actually over, independent of row height.
fn target_index_at(parent: &gtk::Widget, y: f64) -> Option<(i32, i32)> {
    let mut count = 0i32;
    let mut hit = -1i32;
    let mut child = parent.first_child();
    while let Some(w) = child {
        if hit < 0
            && let Some(b) = w.compute_bounds(parent)
            && y < (b.y() + b.height()) as f64
        {
            // First row whose bottom edge is below the cursor = the row
            // the cursor is in (rows are contiguous + ordered).
            hit = count;
        }
        count += 1;
        child = w.next_sibling();
    }
    if count == 0 {
        return None;
    }
    // Cursor past the last row's bottom → land on the last row.
    let target = if hit < 0 { count - 1 } else { hit };
    Some((target, count))
}

/// Highlight the sibling at `target` with `.drop-target`.
fn highlight_target(parent: &gtk::Widget, target: i32) {
    let mut i = 0i32;
    let mut child = parent.first_child();
    while let Some(w) = child {
        if i == target {
            w.add_css_class("drop-target");
            break;
        }
        i += 1;
        child = w.next_sibling();
    }
}

/// Attach drag-to-reorder to a row's `grip` handle. `pos` is the element
/// whose siblings represent list positions (the `ListBoxRow` for a
/// `GtkListBox`, or the row box for a plain `gtk::Box` list); it gets the
/// `.dragging` highlight and anchors the live drop indicator. On release,
/// `on_drag(delta)` is called with the signed number of positions to move
/// (0 is suppressed); the caller clamps it to the list length.
pub(crate) fn attach_grip_drag<F>(
    grip: &impl IsA<gtk::Widget>,
    pos: &impl IsA<gtk::Widget>,
    on_drag: F,
) where
    F: Fn(i32) + 'static,
{
    let gesture = gtk::GestureDrag::new();
    gesture.set_button(gdk::BUTTON_PRIMARY);
    // Capture phase so the grip claims the drag before the enclosing
    // GtkListBox's own gestures get a chance to.
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    let pos = pos.clone().upcast::<gtk::Widget>();
    let grip_w = grip.clone().upcast::<gtk::Widget>();
    let grip_end = grip.clone().upcast::<gtk::Widget>();
    // Grab affordance — GTK4 has no `cursor` CSS property, so set it in
    // code: an open hand at rest, a closed hand while dragging.
    grip.set_cursor_from_name(Some("grab"));

    // Cursor Y at drag-start, in the parent (list) coordinate space. The
    // gesture reports offsets relative to the grip; translating the start
    // point into parent space once lets us recover the absolute cursor Y
    // as `start_y + offset_y` and hit-test it against the rows.
    let start_y = Rc::new(Cell::new(0.0f64));

    let pos_begin = pos.clone();
    let start_begin = start_y.clone();
    gesture.connect_drag_begin(move |_, sx, sy| {
        pos_begin.add_css_class("dragging");
        grip_w.set_cursor_from_name(Some("grabbing"));
        if let Some(parent) = pos_begin.parent() {
            // `translate_coordinates` is deprecated since GTK 4.12; the
            // replacement is `compute_point` (graphene).
            let yp = grip_w
                .compute_point(&parent, &gtk::graphene::Point::new(sx as f32, sy as f32))
                .map(|p| p.y() as f64)
                .unwrap_or(sy);
            start_begin.set(yp);
        }
    });

    // Live drop indicator: highlight the row currently under the cursor.
    let pos_update = pos.clone();
    let start_update = start_y.clone();
    gesture.connect_drag_update(move |_, _offset_x, offset_y| {
        clear_drop_targets(&pos_update);
        let Some(parent) = pos_update.parent() else {
            return;
        };
        let cur_y = start_update.get() + offset_y;
        let Some((target, _count)) = target_index_at(&parent, cur_y) else {
            return;
        };
        let Some(from) = sibling_index(&parent, &pos_update) else {
            return;
        };
        if target != from {
            highlight_target(&parent, target);
        }
    });

    let pos_end = pos.clone();
    let start_end = start_y.clone();
    gesture.connect_drag_end(move |_, _offset_x, offset_y| {
        pos_end.remove_css_class("dragging");
        grip_end.set_cursor_from_name(Some("grab"));
        clear_drop_targets(&pos_end);
        let Some(parent) = pos_end.parent() else {
            return;
        };
        let cur_y = start_end.get() + offset_y;
        let Some((target, _count)) = target_index_at(&parent, cur_y) else {
            return;
        };
        let Some(from) = sibling_index(&parent, &pos_end) else {
            return;
        };
        let delta = target - from;
        if delta != 0 {
            on_drag(delta);
        }
    });

    grip.add_controller(gesture);
}
