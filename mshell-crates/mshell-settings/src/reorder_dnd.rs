//! Shared drag-to-reorder wiring for Settings lists.
//!
//! Adds GTK4 drag-and-drop on top of the up/down buttons. Grab a row and
//! drop it within the **same** list; `on_drop(from, to)` fires with the
//! source + landing indices, with natural semantics (dragging down lands
//! after the target, up lands before).
//!
//! Two pieces because GtkListBox **swallows DnD motion/drop before its
//! rows see it** — a per-row `DropTarget` never fires:
//!   * [`attach_row_drag_source`] on each row records the drag origin.
//!   * [`attach_listbox_drop_target`] on the *ListBox* receives the drop
//!     and resolves the landing row from the pointer via `row_at_y`.
//! Both compare the source/target list widget so a drag started in one
//! list can't drop into another.
//!
//! [`attach_row_reorder_keyed`] is the single-widget variant for lists
//! built directly in a `gtk::Box` (control-center tiles) — a plain Box
//! does *not* swallow DnD, so there source and target both sit on the
//! row. Pairs with `.dragging` styling (dims the grabbed row).

use relm4::factory::DynamicIndex;
use relm4::gtk::prelude::*;
use relm4::gtk::{self, gdk, glib};
use std::cell::RefCell;

thread_local! {
    /// `(source list widget, source index)` for the in-flight drag. GTK
    /// content-providers are awkward to round-trip a typed payload
    /// through, so the identity travels via this main-thread-only side
    /// channel. Set on prepare, consumed on drop, cleared on drag end.
    static DRAG: RefCell<Option<(gtk::Widget, usize)>> = const { RefCell::new(None) };

    /// Same as [`DRAG`] but keyed by a stable string id, for lists that
    /// reorder by id rather than positional `DynamicIndex` (e.g. the
    /// control-center tiles). Independent slot so an index drag and a
    /// keyed drag can't be confused.
    static DRAG_KEYED: RefCell<Option<(gtk::Widget, String)>> = const { RefCell::new(None) };
}

/// Attach the drag **source** to a factory `row` (the returned
/// `ListBoxRow`), recording its logical `index` when a drag starts.
///
/// The matching drop side lives on the *ListBox* via
/// [`attach_listbox_drop_target`], not on each row: GtkListBox swallows
/// DnD motion/drop before its rows see it, so a per-row `DropTarget`
/// never fires (the drag starts but nothing accepts it). A single target
/// on the list works and resolves the landing row from the pointer.
pub(crate) fn attach_row_drag_source(row: &impl IsA<gtk::Widget>, index: &DynamicIndex) {
    let row = row.clone().upcast::<gtk::Widget>();

    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::MOVE);
    let src_index = index.clone();
    let src_row = row.clone();
    source.connect_prepare(move |_, _, _| {
        if let Some(list) = src_row.parent() {
            DRAG.with(|c| *c.borrow_mut() = Some((list, src_index.current_index())));
        }
        // Real payload travels via DRAG; the provider just needs to offer
        // the STRING type the DropTarget accepts.
        Some(gdk::ContentProvider::for_value(&"".to_value()))
    });
    source.connect_drag_begin(|src, _| {
        if let Some(w) = src.widget() {
            w.add_css_class("dragging");
        }
    });
    source.connect_drag_end(|src, _, _| {
        if let Some(w) = src.widget() {
            w.remove_css_class("dragging");
        }
        DRAG.with(|c| *c.borrow_mut() = None);
    });
    row.add_controller(source);
}

/// Attach the drop **target** to a `ListBox` whose rows carry
/// [`attach_row_drag_source`]. On drop the landing row is resolved from
/// the pointer's y via `row_at_y` (a drop in the empty space past the
/// last row lands at the end). `on_drop(from, to)` fires only when the
/// drag originated in this same list.
pub(crate) fn attach_listbox_drop_target<F>(list: &gtk::ListBox, on_drop: F)
where
    F: Fn(usize, usize) + 'static,
{
    let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    let list_for_drop = list.clone();
    target.connect_drop(move |_, _, _x, y| {
        let Some((src_list, from)) = DRAG.with(|c| c.borrow_mut().take()) else {
            return false;
        };
        // Reject drags that started in a different list.
        if src_list != list_for_drop.clone().upcast::<gtk::Widget>() {
            return false;
        }
        let count = list_for_drop.observe_children().n_items() as usize;
        let to = match list_for_drop.row_at_y(y as i32) {
            Some(row) => row.index().max(0) as usize,
            None => count.saturating_sub(1),
        };
        if from != to {
            on_drop(from, to);
        }
        true
    });
    list.add_controller(target);
}

/// Like [`attach_row_reorder`] but the row is identified by a stable
/// string `key` instead of a positional index — for lists that reorder
/// by id (control-center tiles). `on_drop(from_key, to_key)` runs only
/// for drops within the same parent list.
pub(crate) fn attach_row_reorder_keyed<F>(
    row: &impl IsA<gtk::Widget>,
    key: impl Into<String>,
    on_drop: F,
) where
    F: Fn(&str, &str) + 'static,
{
    let row = row.clone().upcast::<gtk::Widget>();
    let key: String = key.into();

    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::MOVE);
    source.set_propagation_phase(gtk::PropagationPhase::Capture);
    let src_row = row.clone();
    let src_key = key.clone();
    source.connect_prepare(move |_, _, _| {
        tracing::info!(key = %src_key, "reorder_dnd: keyed drag prepare");
        if let Some(list) = src_row.parent() {
            DRAG_KEYED.with(|c| *c.borrow_mut() = Some((list, src_key.clone())));
        }
        Some(gdk::ContentProvider::for_value(&"".to_value()))
    });
    source.connect_drag_begin(|src, _| {
        if let Some(w) = src.widget() {
            w.add_css_class("dragging");
        }
    });
    source.connect_drag_end(|src, _, _| {
        if let Some(w) = src.widget() {
            w.remove_css_class("dragging");
        }
        DRAG_KEYED.with(|c| *c.borrow_mut() = None);
    });
    row.add_controller(source);

    let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    let dst_row = row.clone();
    let dst_key = key.clone();
    target.connect_drop(move |_, _, _, _| {
        let taken = DRAG_KEYED.with(|c| c.borrow_mut().take());
        let same_list = taken
            .as_ref()
            .is_some_and(|(src_list, _)| dst_row.parent().as_ref() == Some(src_list));
        tracing::info!(to = %dst_key, ?taken, same_list, "reorder_dnd: keyed drop");
        if let Some((_, from_key)) = taken
            && same_list
            && from_key != dst_key
        {
            on_drop(&from_key, &dst_key);
            return true;
        }
        false
    });
    row.add_controller(target);
}
