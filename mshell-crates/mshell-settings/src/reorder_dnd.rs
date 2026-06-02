//! Shared drag-to-reorder wiring for Settings lists.
//!
//! Uses a plain `GtkGestureDrag` on each row's ≡ grip handle — **not**
//! GTK drag-and-drop. GtkListBox swallows real DnD motion/drop before its
//! rows see it, which made `DragSource`/`DropTarget` reordering
//! unreliable; a gesture on the grip has none of that baggage.
//!
//! Grab the grip, drag vertically, release: the row moves by the number
//! of fixed [`STEP_PX`] steps dragged (down = positive). A *fixed* step
//! (not the row's own height) keeps it responsive — menu rows with an
//! expanded inline config area are very tall, so dividing by their height
//! rounded almost everything to zero. The caller turns the delta into a
//! concrete move and clamps it to the list length. The grabbed row gets
//! the `.dragging` class while the gesture is active.

use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::prelude::*;

/// Vertical travel (px) that equals one position of movement.
const STEP_PX: f64 = 32.0;

/// Attach drag-to-reorder to a row's `grip` handle. `row` is the widget
/// that visually represents the row (gets the `.dragging` highlight). On
/// release, `on_drag(delta)` is called with the signed number of
/// positions to move (0 is suppressed).
pub(crate) fn attach_grip_drag<F>(
    grip: &impl IsA<gtk::Widget>,
    row: &impl IsA<gtk::Widget>,
    on_drag: F,
) where
    F: Fn(i32) + 'static,
{
    let gesture = gtk::GestureDrag::new();
    gesture.set_button(gdk::BUTTON_PRIMARY);
    // Capture phase so the grip claims the drag before the enclosing
    // GtkListBox's own gestures get a chance to.
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    let row = row.clone().upcast::<gtk::Widget>();

    let row_begin = row.clone();
    gesture.connect_drag_begin(move |_, _, _| {
        tracing::info!("reorder_dnd: drag begin");
        row_begin.add_css_class("dragging");
    });

    let row_end = row.clone();
    gesture.connect_drag_end(move |_, _offset_x, offset_y| {
        row_end.remove_css_class("dragging");
        let delta = (offset_y / STEP_PX).round() as i32;
        tracing::info!(offset_y, delta, "reorder_dnd: drag end");
        if delta != 0 {
            on_drag(delta);
        }
    });

    grip.add_controller(gesture);
}
