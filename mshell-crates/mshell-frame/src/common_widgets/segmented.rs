//! The one segmented control (DESIGN §12 "Segmented control", §18).
//!
//! Two looks, one behaviour (a row of mutually exclusive buttons, exactly
//! one active, keyboard-focusable with a visible ring):
//!
//! * [`SegmentedStyle::Capsule`] — the §12 panel view switch: one
//!   `.ok-segmented` capsule track, the active segment on
//!   `--secondary-container` (clipboard, agents, GitHub, crashes).
//! * [`SegmentedStyle::ButtonRow`] — a compact value picker: equal-width
//!   `.ok-button-surface` buttons, the active one `.selected` (CPU
//!   dashboard sort / protocol / fan level, like the VPN mode row).
//!
//! Build it imperatively with [`Segmented::new`], or — inside a `view!`
//! with `#[watch]` — put [`TRACK_CLASS`] on the box and
//! [`segment_classes`] on each button.

use relm4::gtk::{self, prelude::*};
use std::rc::Rc;

/// CSS class of the capsule track (`03-primitives/_segmented.scss`).
pub(crate) const TRACK_CLASS: &str = "ok-segmented";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentedStyle {
    Capsule,
    ButtonRow,
}

/// The CSS classes of one segment.
pub(crate) fn segment_classes(style: SegmentedStyle, active: bool) -> &'static [&'static str] {
    match (style, active) {
        (SegmentedStyle::Capsule, false) => &["ok-segment"],
        (SegmentedStyle::Capsule, true) => &["ok-segment", "active"],
        (SegmentedStyle::ButtonRow, false) => &["ok-button-surface", "label-small"],
        (SegmentedStyle::ButtonRow, true) => &["ok-button-surface", "label-small", "selected"],
    }
}

/// A built segmented control: the row box plus its buttons.
pub(crate) struct Segmented {
    root: gtk::Box,
    buttons: Vec<gtk::Button>,
    style: SegmentedStyle,
}

impl Segmented {
    /// `labels` in order; `active` starts selected; `on_select(i)` fires
    /// when segment `i` is clicked (the caller then calls [`Self::set_active`]
    /// — the model owns the choice).
    pub(crate) fn new(
        labels: &[&str],
        active: usize,
        style: SegmentedStyle,
        on_select: impl Fn(usize) + 'static,
    ) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        root.set_homogeneous(true);
        if style == SegmentedStyle::Capsule {
            root.add_css_class(TRACK_CLASS);
        }
        let on_select = Rc::new(on_select);
        let buttons = labels
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let b = gtk::Button::with_label(label);
                b.set_css_classes(segment_classes(style, i == active));
                let f = on_select.clone();
                b.connect_clicked(move |_| f(i));
                root.append(&b);
                b
            })
            .collect();
        Self {
            root,
            buttons,
            style,
        }
    }

    pub(crate) fn root(&self) -> &gtk::Box {
        &self.root
    }

    pub(crate) fn buttons(&self) -> &[gtk::Button] {
        &self.buttons
    }

    /// Mark segment `active` (any other index clears them all).
    pub(crate) fn set_active(&self, active: usize) {
        for (i, b) in self.buttons.iter().enumerate() {
            b.set_css_classes(segment_classes(self.style, i == active));
        }
    }

    pub(crate) fn set_label(&self, i: usize, label: &str) {
        if let Some(b) = self.buttons.get(i)
            && b.label().as_deref() != Some(label)
        {
            b.set_label(label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exactly one state class separates the active segment, and both looks
    /// keep their base class (the capsule's ring / the surface button chrome
    /// hang off it).
    #[test]
    fn classes_per_style() {
        for style in [SegmentedStyle::Capsule, SegmentedStyle::ButtonRow] {
            let off = segment_classes(style, false);
            let on = segment_classes(style, true);
            assert_eq!(on.len(), off.len() + 1, "{style:?}");
            assert!(off.iter().all(|c| on.contains(c)), "{style:?}");
        }
        assert_eq!(
            segment_classes(SegmentedStyle::Capsule, true),
            ["ok-segment", "active"]
        );
        assert!(segment_classes(SegmentedStyle::ButtonRow, true).contains(&"selected"));
    }
}
