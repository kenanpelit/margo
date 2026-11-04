//! Accessible names for icon-only controls (DESIGN.md §13.8).
//!
//! An icon-only button has no text for a screen reader to announce (Orca
//! said just "button" for shutdown / reboot / logout), and a tooltip is not
//! a substitute: GTK exposes a tooltip as the accessible *description*, not
//! the name. [`name`] sets both — the accessible label always, the tooltip
//! only when the control has none of its own.

use relm4::gtk;
use relm4::gtk::prelude::*;

/// The tooltip to add for `label`, or `None` when the widget already has
/// tooltip handling of its own (static text or a dynamic `query-tooltip`).
pub fn tooltip_to_add(has_tooltip: bool, label: &str) -> Option<&str> {
    (!has_tooltip && !label.trim().is_empty()).then_some(label)
}

/// Give `widget` the accessible name `label`, plus `label` as its tooltip
/// when it has none.
pub fn name(widget: &impl IsA<gtk::Widget>, label: &str) {
    let w = widget.upcast_ref::<gtk::Widget>();
    w.update_property(&[gtk::accessible::Property::Label(label)]);
    if let Some(tip) = tooltip_to_add(w.has_tooltip(), label) {
        w.set_tooltip_text(Some(tip));
    }
}

/// Where and how a bar pill's display name goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PillNaming {
    /// Put it on the pill's single button (`true`) or on the root (`false`).
    pub on_button: bool,
    /// Set it as the accessible *label* (`true`), or only as the
    /// *description* (`false`) so the name GTK derives from visible text (a
    /// clock's time, a tag's number) is kept.
    pub as_label: bool,
    /// Also give the root the name as a hover tooltip.
    pub tooltip: bool,
}

/// Decide how to name a pill from what it contains when it is mounted:
/// `buttons` focusable buttons, `has_text` = a `gtk::Label` with text,
/// `has_tooltip` = the pill already handles a tooltip somewhere.
///
/// - One button, icon only (shutdown, reboot, logout, …): the button gets
///   the label, the root a tooltip.
/// - Several buttons (tags, tray, dock): each child is its own control; the
///   root only gets a description of the group, and no tooltip that would
///   cover the children's.
/// - Visible text: keep the text-derived name, add a description.
pub fn pill_naming(buttons: usize, has_text: bool, has_tooltip: bool) -> PillNaming {
    let single = buttons <= 1;
    PillNaming {
        on_button: buttons == 1,
        as_label: single && !has_text,
        tooltip: single && !has_tooltip,
    }
}

/// Name a bar pill `label` (its display name) by [`pill_naming`]. A widget
/// that later sets its own tooltip on the root simply replaces ours.
pub fn name_pill(root: &gtk::Widget, label: &str) {
    if label.trim().is_empty() {
        return;
    }
    let mut buttons = Vec::new();
    collect_buttons(root, &mut buttons);
    let naming = pill_naming(buttons.len(), has_text(root), owns_tooltip(root));
    let target = match (naming.on_button, buttons.first()) {
        (true, Some(b)) => b.clone(),
        _ => root.clone(),
    };
    if naming.as_label {
        target.update_property(&[gtk::accessible::Property::Label(label)]);
    } else {
        target.update_property(&[gtk::accessible::Property::Description(label)]);
    }
    if naming.tooltip {
        root.set_tooltip_text(Some(label));
    }
}

fn collect_buttons(w: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
    if w.is::<gtk::Button>() || w.is::<gtk::MenuButton>() {
        out.push(w.clone());
        return; // a button's insides are not separate controls
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        collect_buttons(&c, out);
        child = c.next_sibling();
    }
}

fn any_descendant(w: &gtk::Widget, pred: &impl Fn(&gtk::Widget) -> bool) -> bool {
    if pred(w) {
        return true;
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if any_descendant(&c, pred) {
            return true;
        }
        child = c.next_sibling();
    }
    false
}

fn has_text(root: &gtk::Widget) -> bool {
    any_descendant(root, &|w| {
        w.downcast_ref::<gtk::Label>()
            .is_some_and(|l| l.is_visible() && !l.text().trim().is_empty())
    })
}

fn owns_tooltip(root: &gtk::Widget) -> bool {
    any_descendant(root, &|w| w.has_tooltip())
}

#[cfg(test)]
mod tests {
    use super::{PillNaming, pill_naming, tooltip_to_add};

    #[test]
    fn tooltip_only_where_none_exists() {
        assert_eq!(tooltip_to_add(false, "Shut down"), Some("Shut down"));
        assert_eq!(
            tooltip_to_add(true, "Shut down"),
            None,
            "keep the widget's own"
        );
        assert_eq!(tooltip_to_add(false, "  "), None);
    }

    #[test]
    fn icon_only_pill_names_its_button() {
        assert_eq!(
            pill_naming(1, false, false),
            PillNaming {
                on_button: true,
                as_label: true,
                tooltip: true
            }
        );
    }

    #[test]
    fn pill_with_own_tooltip_keeps_it() {
        let n = pill_naming(1, false, true);
        assert!(!n.tooltip);
        assert!(n.as_label, "a tooltip is not the accessible name");
    }

    #[test]
    fn text_pill_keeps_its_text_as_the_name() {
        let n = pill_naming(1, true, false);
        assert!(!n.as_label, "a clock must still read out the time");
        assert!(n.on_button);
    }

    #[test]
    fn multi_button_pill_only_describes_the_group() {
        // Tags / tray: labelling the first tag "Margo Tags" would hide its
        // number, and a root tooltip would cover the items' own.
        assert_eq!(
            pill_naming(5, true, false),
            PillNaming {
                on_button: false,
                as_label: false,
                tooltip: false
            }
        );
    }

    #[test]
    fn buttonless_pill_labels_the_root() {
        let n = pill_naming(0, false, false);
        assert!(!n.on_button && n.as_label && n.tooltip);
    }
}
