//! Shared drag-to-reorder wiring for Settings factory rows.
//!
//! Every reorderable list in Settings (bar widgets, menu widgets, quick
//! actions, control-center tiles) has up/down buttons; this adds GTK4
//! drag-and-drop on top. Grab a row and drop it onto another in the
//! **same** list and `on_drop(from, to)` fires with the source + target
//! indices. Cross-list drops are rejected by comparing the two rows'
//! parent list widget, so dragging between separate lists (e.g. the
//! bar's start vs end sections, or a nested container vs its parent) is
//! ignored — matching the buttons' within-list scope.
//!
//! Drop semantics are natural: dragging down lands after the target,
//! dragging up lands before it. Pairs with `.dragging` (dims the grabbed
//! row) and `:drop(active)` (highlights the landing row) styling.

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

/// Attach drag-to-reorder to a factory `row` (typically the returned
/// `ListBoxRow`) whose logical position is `index`. `on_drop(from, to)`
/// runs only for drops within the same parent list.
pub(crate) fn attach_row_reorder<F>(row: &impl IsA<gtk::Widget>, index: &DynamicIndex, on_drop: F)
where
    F: Fn(usize, usize) + 'static,
{
    let row = row.clone().upcast::<gtk::Widget>();

    // --- Source ---
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

    // --- Target ---
    let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    let dst_index = index.clone();
    let dst_row = row.clone();
    target.connect_drop(move |_, _, _, _| {
        let to = dst_index.current_index();
        let taken = DRAG.with(|c| c.borrow_mut().take());
        if let Some((src_list, from)) = taken
            && dst_row.parent().as_ref() == Some(&src_list)
            && from != to
        {
            on_drop(from, to);
            return true;
        }
        false
    });
    row.add_controller(target);
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
    let src_row = row.clone();
    let src_key = key.clone();
    source.connect_prepare(move |_, _, _| {
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
        if let Some((src_list, from_key)) = taken
            && dst_row.parent().as_ref() == Some(&src_list)
            && from_key != dst_key
        {
            on_drop(&from_key, &dst_key);
            return true;
        }
        false
    });
    row.add_controller(target);
}
