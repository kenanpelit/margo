//! Pill state classes that don't clobber the pill standard.
//!
//! `bar.rs` adds `.bar-pill-std` (the one pill surface / hover / 28 px
//! height, `_bar_widget.scss`) to every pill root AFTER the widget's `init`.
//! A root that re-applies `#[watch] set_css_classes: …` replaces the whole
//! class list on its next update and silently drops `.bar-pill-std` — the
//! pill then renders with a different height / hover / islands surface from
//! its neighbours, but only after its first state change. State classes on a
//! pill root are therefore toggled one by one: relm4's
//! `#[watch] set_class_active: ("active", cond)` for a single flag, or
//! [`PillClasses::set_state_classes`] for a set (accent variants, error vs.
//! has-updates). Both leave every class they don't own alone. Design lint
//! L13 rejects a `#[watch] set_css_classes` on a pill root.

use relm4::gtk;
use relm4::gtk::prelude::{IsA, WidgetExt};

/// The classes to add and remove so that, of the `managed` classes, exactly
/// `wanted` are set. Classes outside `managed` are never touched.
pub(crate) fn class_diff<'a>(
    current: &[String],
    managed: &[&'a str],
    wanted: &[&'a str],
) -> (Vec<&'a str>, Vec<&'a str>) {
    let has = |c: &str| current.iter().any(|x| x == c);
    let add = wanted
        .iter()
        .copied()
        .filter(|c| managed.contains(c) && !has(c))
        .collect();
    let remove = managed
        .iter()
        .copied()
        .filter(|c| has(c) && !wanted.contains(c))
        .collect();
    (add, remove)
}

pub(crate) trait PillClasses {
    /// Of the `managed` state classes, set exactly `wanted`; every other
    /// class (the pill's base classes, `.bar-pill-std`) stays.
    fn set_state_classes(&self, managed: &[&str], wanted: &[&str]);
}

impl<W: IsA<gtk::Widget>> PillClasses for W {
    fn set_state_classes(&self, managed: &[&str], wanted: &[&str]) {
        let current: Vec<String> = self.css_classes().iter().map(|c| c.to_string()).collect();
        let (add, remove) = class_diff(&current, managed, wanted);
        for c in remove {
            self.remove_css_class(c);
        }
        for c in add {
            self.add_css_class(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|c| c.to_string()).collect()
    }

    const STATES: &[&str] = &["error", "has-updates"];

    /// The bug: a state change replaced the whole class list and the
    /// `.bar-pill-std` class `bar.rs` added was gone.
    #[test]
    fn a_state_change_keeps_the_pill_standard() {
        let current = s(&["system-update-bar-widget", "bar-pill-std", "error"]);
        let (add, remove) = class_diff(&current, STATES, &["has-updates"]);
        assert_eq!(add, vec!["has-updates"]);
        assert_eq!(remove, vec!["error"]);
    }

    #[test]
    fn clearing_every_state_removes_only_managed_classes() {
        let current = s(&["usb-bar-widget", "bar-pill-std", "has-updates"]);
        let (add, remove) = class_diff(&current, STATES, &[]);
        assert!(add.is_empty());
        assert_eq!(remove, vec!["has-updates"]);
    }

    #[test]
    fn an_unchanged_state_is_a_no_op() {
        let current = s(&["bar-pill-std", "error"]);
        assert_eq!(class_diff(&current, STATES, &["error"]), (vec![], vec![]));
    }

    #[test]
    fn a_wanted_class_outside_the_managed_set_is_ignored() {
        let (add, remove) = class_diff(&s(&[]), STATES, &["bar-pill-std"]);
        assert!(add.is_empty() && remove.is_empty());
    }
}
