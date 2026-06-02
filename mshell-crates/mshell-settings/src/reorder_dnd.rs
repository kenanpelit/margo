//! Shared drag-to-reorder wiring for Settings lists.
//!
//! Uses a plain `GtkGestureDrag` on each row's ≡ grip handle — **not**
//! GTK drag-and-drop. GtkListBox swallows real DnD motion/drop before its
//! rows see it, which made `DragSource`/`DropTarget` reordering
//! unreliable; a gesture on a leaf grip widget has none of that baggage.
//!
//! Grab the grip, drag vertically, release: the row moves by the number
//! of row-heights dragged (down = positive). The caller turns that delta
//! into a concrete move and clamps it to the list length. The grabbed row
//! gets the `.dragging` class while the gesture is active.

use relm4::gtk;
use relm4::gtk::prelude::*;

/// Attach drag-to-reorder to a row's `grip` handle. `row` is the widget
/// that visually represents the row (used for the `.dragging` highlight
/// and to measure row height). On release, `on_drag(delta)` is called
/// with the signed number of positions to move (0 is suppressed).
pub(crate) fn attach_grip_drag<F>(
    grip: &impl IsA<gtk::Widget>,
    row: &impl IsA<gtk::Widget>,
    on_drag: F,
) where
    F: Fn(i32) + 'static,
{
    let gesture = gtk::GestureDrag::new();
    let row = row.clone().upcast::<gtk::Widget>();

    let row_begin = row.clone();
    gesture.connect_drag_begin(move |_, _, _| {
        row_begin.add_css_class("dragging");
    });

    let row_end = row.clone();
    gesture.connect_drag_end(move |_, _offset_x, offset_y| {
        row_end.remove_css_class("dragging");
        // Round the vertical travel to whole rows. `height()` is the row's
        // current allocation, so one row-height of drag == one position.
        let h = row_end.height().max(1);
        let delta = (offset_y / h as f64).round() as i32;
        if delta != 0 {
            on_drag(delta);
        }
    });

    grip.add_controller(gesture);
}
