//! Small GTK builders shared by the CPU dashboard's list tabs (Processes,
//! Ports, Drives). They only assemble the house row shapes — DESIGN.md §5
//! boxed-list rows (`label-medium-bold` title + `label-small dim-label`
//! subtitle, trailing value + flat icon action), the §17 placeholder line
//! and the inline confirm bar — so every tab reads like the podman / ufw
//! menus instead of inventing its own chrome.

use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ListBoxRowExt, WidgetExt};

/// Remove every row from a list (rebuilt each poll, like podman's lists).
pub(crate) fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

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
    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    let t = gtk::Label::builder()
        .label(title)
        .css_classes(["label-medium-bold"])
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    texts.append(&t);
    if !subtitle.is_empty() {
        let s = gtk::Label::builder()
            .label(subtitle)
            .css_classes(["label-small", "dim-label"])
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        texts.append(&s);
    }
    texts
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

/// Labelled text button in the house surface style; `danger` uses the
/// error ladder (§2) for destructive confirms.
pub(crate) fn text_button(label: &str, danger: bool) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.set_css_classes(if danger {
        &["ok-button-error", "label-small"]
    } else {
        &["ok-button-surface", "label-small"]
    });
    b
}
