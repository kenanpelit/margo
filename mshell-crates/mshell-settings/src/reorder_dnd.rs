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
//! concrete move and clamps it to the list length.
//!
//! **Live feedback (during the drag):** the grabbed row dims (`.dragging`)
//! and the row the item will land on is highlighted (`.drop-target`),
//! moving as you drag — so the result is visible before release. This is
//! purely visual (no data/config change mid-drag), found by walking the
//! position element's siblings, so it works the same whether the list is
//! a `GtkListBox` (rows are `ListBoxRow`s) or a plain `gtk::Box`.

use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::prelude::*;

/// Vertical travel (px) that equals one position of movement.
const STEP_PX: f64 = 32.0;

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

    let pos_begin = pos.clone();
    gesture.connect_drag_begin(move |_, _, _| {
        pos_begin.add_css_class("dragging");
    });

    // Live drop indicator: highlight the sibling the row will land on.
    let pos_update = pos.clone();
    gesture.connect_drag_update(move |_, _offset_x, offset_y| {
        clear_drop_targets(&pos_update);
        let delta = (offset_y / STEP_PX).round() as i32;
        if delta == 0 {
            return;
        }
        let Some(parent) = pos_update.parent() else {
            return;
        };
        // Index of the dragged row among its siblings + the sibling count.
        let (mut from, mut count) = (-1i32, 0i32);
        let mut child = parent.first_child();
        while let Some(w) = child {
            if w == pos_update {
                from = count;
            }
            count += 1;
            child = w.next_sibling();
        }
        if from < 0 {
            return;
        }
        let target = (from + delta).clamp(0, count - 1);
        if target == from {
            return;
        }
        // Highlight the sibling at `target`.
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
    });

    let pos_end = pos.clone();
    gesture.connect_drag_end(move |_, _offset_x, offset_y| {
        pos_end.remove_css_class("dragging");
        clear_drop_targets(&pos_end);
        let delta = (offset_y / STEP_PX).round() as i32;
        if delta != 0 {
            on_drag(delta);
        }
    });

    grip.add_controller(gesture);
}
