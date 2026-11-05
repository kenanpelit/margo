//! Small GTK builders shared by the CPU dashboard's list tabs (Processes,
//! Ports, Drives). They only assemble the house row shapes — DESIGN.md §5
//! boxed-list rows (`label-medium-bold` title + `label-small dim-label`
//! subtitle, trailing value + flat icon action), the §17 placeholder line
//! and the inline confirm bar — so every tab reads like the podman / ufw
//! menus instead of inventing its own chrome.

use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ListBoxRowExt, WidgetExt};

/// A non-interactive list row wrapping `child`.
pub(crate) fn plain_row(child: &impl gtk::prelude::IsA<gtk::Widget>) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    row.set_child(Some(child));
    row
}

/// §17 empty / no-service / no-permission line: calm, dim, never red.
pub(crate) fn placeholder_row(text: &str) -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(text)
        .css_classes(["label-small", "dim-label"])
        .xalign(0.0)
        .wrap(true)
        .build();
    plain_row(&label)
}

/// Title + subtitle column (hexpand) — the Adwaita ActionRow text block.
pub(crate) fn title_block(title: &str, subtitle: &str) -> gtk::Box {
    let t = live_title_block();
    set_title(&t, title, subtitle);
    t.block
}

/// A [`title_block`] whose labels stay reachable, for rows updated in
/// place across polls. The subtitle hides while empty.
pub(crate) struct LiveTitle {
    pub block: gtk::Box,
    pub title: gtk::Label,
    pub subtitle: gtk::Label,
}

pub(crate) fn live_title_block() -> LiveTitle {
    let block = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    let title = gtk::Label::builder()
        .css_classes(["label-medium-bold"])
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    block.append(&title);
    let subtitle = gtk::Label::builder()
        .css_classes(["label-small", "dim-label"])
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .visible(false)
        .build();
    block.append(&subtitle);
    LiveTitle {
        block,
        title,
        subtitle,
    }
}

/// Set a [`LiveTitle`]'s texts, touching only what changed.
pub(crate) fn set_title(t: &LiveTitle, title: &str, subtitle: &str) {
    set_text(&t.title, title);
    set_text(&t.subtitle, subtitle);
    if t.subtitle.is_visible() == subtitle.is_empty() {
        t.subtitle.set_visible(!subtitle.is_empty());
    }
}

/// Set a label's text only when it differs (no relayout when unchanged).
pub(crate) fn set_text(label: &gtk::Label, text: &str) {
    if label.label() != text {
        label.set_label(text);
    }
}

/// Toggle a CSS class on `widget` to match `on`.
pub(crate) fn set_class(widget: &impl WidgetExt, class: &str, on: bool) {
    if widget.has_css_class(class) != on {
        if on {
            widget.add_css_class(class);
        } else {
            widget.remove_css_class(class);
        }
    }
}

/// Replace `slot` with `new` and say whether it changed (a poll that
/// returns what is already shown skips its re-render).
pub(crate) fn replace_if_changed<T: PartialEq>(slot: &mut T, new: T) -> bool {
    if *slot == new {
        return false;
    }
    *slot = new;
    true
}

/// A trailing right-aligned metric (tabular figures, fixed width so the
/// column lines up row to row).
pub(crate) fn value_label(text: &str, width_chars: i32, extra_class: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .width_chars(width_chars)
        .xalign(1.0)
        .valign(gtk::Align::Center)
        .build();
    if extra_class.is_empty() {
        l.set_css_classes(&["cpu-dashboard-stat-value"]);
    } else {
        l.set_css_classes(&["cpu-dashboard-stat-value", extra_class]);
    }
    l
}

/// Icon-only flat action button with an accessible name (§15 L12).
pub(crate) fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let btn = gtk::Button::from_icon_name(icon);
    btn.add_css_class("ok-button-flat");
    btn.set_valign(gtk::Align::Center);
    mshell_common::a11y::name(&btn, name);
    btn
}

/// One horizontal row body: `title_block` + the given trailing widgets.
pub(crate) fn row_body(title: &str, subtitle: &str, trailing: &[gtk::Widget]) -> gtk::Box {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    b.append(&title_block(title, subtitle));
    for w in trailing {
        b.append(w);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::replace_if_changed;

    #[test]
    fn an_unchanged_poll_skips_the_render() {
        let mut shown = Some(Ok::<_, String>(vec![1, 2]));
        assert!(!replace_if_changed(&mut shown, Some(Ok(vec![1, 2]))));
        assert!(replace_if_changed(&mut shown, Some(Ok(vec![1, 3]))));
        assert_eq!(shown, Some(Ok(vec![1, 3])));
    }
}
