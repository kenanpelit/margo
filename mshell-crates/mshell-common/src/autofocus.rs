//! Typing goes where the menu expects it.
//!
//! A menu built around a text field — the app launcher's search, the AI
//! prompt, a filter — marks that field with [`mark`]. When the frame hands
//! its layer surface keyboard focus for a newly opened menu
//! (`Frame::sync_keyboard_mode`), it calls [`focus_in`], which finds the one
//! marked field on screen and focuses it, so the first key typed lands in it
//! — from a bar pill, a keybind or `mshellctl menu …` alike.
//!
//! Why a central pass instead of each menu's own `grab_focus` on reveal: the
//! layer surface only receives keyboard focus once the frame's debounced
//! keyboard-mode switch (~90 ms) has reached the compositor. A grab made
//! before that sets GTK's focus pointer but does not stick, so most menus
//! opened with nothing focused and a typed key went nowhere. [`focus_in`]
//! runs after the switch and focuses the field again once the window
//! really holds the keyboard.

use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// The CSS class that marks a menu's typing field.
pub const CLASS: &str = "menu-autofocus";

/// Mark `widget` (an entry, a search entry, a text view) as the field its
/// menu focuses when it opens.
pub fn mark(widget: &impl IsA<gtk::Widget>) {
    widget.as_ref().add_css_class(CLASS);
}

/// Stamped on a field once [`focus_in`] focused it for the current open, so
/// a re-sync of the keyboard mode while that menu stays open does not pull
/// the focus back from a row the user moved to. [`reset`] (every menu
/// closed) and a different menu taking over clear it.
const DONE: &str = "menu-autofocus-done";

/// The marked field under `root` that is on screen: mapped, sensitive, and
/// inside no revealer that is closing (a menu sliding away keeps its child
/// mapped until its animation ends). The first in tree order wins.
pub fn find(root: &gtk::Widget) -> Option<gtk::Widget> {
    let mut todo = vec![root.clone()];
    while let Some(widget) = todo.pop() {
        if !widget.is_mapped() {
            continue;
        }
        // A closed menu keeps its widgets mapped (the frame's revealers
        // only shrink it away), so skip every revealer that is closing or
        // closed.
        if let Some(revealer) = widget.downcast_ref::<gtk::Revealer>()
            && !revealer.reveals_child()
        {
            continue;
        }
        if let Some(revealer) = widget.downcast_ref::<crate::diagonal_revealer::DiagonalRevealer>()
            && !revealer.is_revealed()
        {
            continue;
        }
        if widget.has_css_class(CLASS) && widget.is_sensitive() && widget.can_focus() {
            return Some(widget);
        }
        // A stack shows one menu; a crossfade keeps the old one mapped
        // for a moment.
        if let Some(stack) = widget.downcast_ref::<gtk::Stack>() {
            todo.extend(stack.visible_child());
            continue;
        }
        // Push children last-first so the first child is visited first.
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            children.push(c);
        }
        todo.extend(children.into_iter().rev());
    }
    None
}

/// Whether `widget` is really on screen: mapped, and no ancestor is a
/// closed revealer or a stack page that is not the visible one.
pub fn on_screen(widget: &gtk::Widget) -> bool {
    if !widget.is_mapped() {
        return false;
    }
    let mut child = widget.clone();
    while let Some(parent) = child.parent() {
        if let Some(revealer) = parent.downcast_ref::<gtk::Revealer>()
            && !revealer.reveals_child()
        {
            return false;
        }
        if let Some(revealer) = parent.downcast_ref::<crate::diagonal_revealer::DiagonalRevealer>()
            && !revealer.is_revealed()
        {
            return false;
        }
        if let Some(stack) = parent.downcast_ref::<gtk::Stack>()
            && stack.visible_child().as_ref() != Some(&child)
        {
            return false;
        }
        child = parent;
    }
    true
}

/// Whether the keyboard focus of `target`'s window is `target` or inside it
/// (an entry's focus sits on its inner text widget) and the window has the
/// keyboard.
pub fn holds_focus(target: &gtk::Widget) -> bool {
    let Some(window) = target.root().and_downcast::<gtk::Window>() else {
        return false;
    };
    window.is_active()
        && gtk::prelude::GtkWindowExt::focus(&window)
            .is_some_and(|f| &f == target || f.is_ancestor(target))
}

/// Focus `target` now, and again the moment its window gets the keyboard.
///
/// The compositor hands the layer surface the keyboard only once it has
/// seen the commit carrying the new keyboard mode — a frame later on a
/// fast machine, most of a second later while the menu's open animation
/// keeps the GPU busy (seconds under software rendering). So instead of
/// guessing a delay, wait for the window to turn active and focus the field
/// then — if it is still on screen and the user has not clicked into
/// another field meanwhile. The handler disconnects itself on that first
/// activation.
pub fn focus(target: &gtk::Widget) {
    target.grab_focus();
    let Some(window) = target.root().and_downcast::<gtk::Window>() else {
        return;
    };
    if window.is_active() {
        return;
    }
    let weak = target.downgrade();
    let handler: Rc<Cell<Option<glib::SignalHandlerId>>> = Rc::default();
    let handler_in = handler.clone();
    let id = window.connect_is_active_notify(move |window| {
        if !window.is_active() {
            return;
        }
        if let Some(id) = handler_in.take() {
            window.disconnect(id);
        }
        let Some(target) = weak.upgrade() else {
            return;
        };
        let focus = gtk::prelude::GtkWindowExt::focus(window);
        tracing::debug!(
            focus_parent = ?focus.as_ref().and_then(|f| f.parent()).map(|p| p.css_classes()),
            "autofocus: window became active"
        );
        // A field the user clicked into meanwhile keeps the focus — but
        // only one on screen: the focus GTK left in a menu closed earlier
        // (still mapped behind its shrunk revealer) is no choice of theirs.
        let elsewhere = focus.is_some_and(|f| {
            is_text_field(&f) && !f.is_ancestor(&target) && f != target && on_screen(&f)
        });
        if target.is_mapped() && !elsewhere && !holds_focus(&target) {
            target.grab_focus();
        }
    });
    handler.set(Some(id));
}

/// Every marked field under `root`, on screen or not.
fn all_marked(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    let mut todo = vec![root.clone()];
    while let Some(widget) = todo.pop() {
        if widget.has_css_class(CLASS) {
            out.push(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            todo.push(c);
        }
    }
    out
}

/// An editable text widget (an entry's inner text, a text view).
fn is_text_field(widget: &gtk::Widget) -> bool {
    widget.is::<gtk::Text>() || widget.is::<gtk::TextView>() || widget.is::<gtk::Entry>()
}

/// The frame's pass once its surface holds the keyboard: focus the marked
/// field of the menu on screen under `root`, unless it was already focused
/// for this open.
pub fn focus_in(root: &gtk::Widget) {
    let found = find(root);
    tracing::debug!(
        found = ?found.as_ref().map(|w| w.css_classes()),
        "autofocus: keyboard handed to the frame"
    );
    for other in all_marked(root) {
        if Some(&other) != found.as_ref() {
            other.remove_css_class(DONE);
        }
    }
    if let Some(found) = found
        && !found.has_css_class(DONE)
    {
        found.add_css_class(DONE);
        focus(&found);
    }
}

/// Every menu under `root` closed: the next open focuses its field again.
pub fn reset(root: &gtk::Widget) {
    for widget in all_marked(root) {
        widget.remove_css_class(DONE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marks_are_distinct_classes() {
        assert_ne!(CLASS, DONE);
        assert!(!CLASS.contains(' ') && !DONE.contains(' '));
    }
}
