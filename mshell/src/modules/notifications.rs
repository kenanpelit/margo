//! Notifications bell + history popover.
//!
//! Stage-6 had a bare bell. This expands it to:
//!   * a count badge (unread notifications) painted on the icon
//!   * a click-popover that lists recent notifications
//!
//! The actual `org.freedesktop.Notifications` D-Bus server is
//! still queued for Stage 9 — until then the history shows a
//! placeholder "no notifications" line. Once the daemon lands, the
//! same widget consumes it: the popover already knows how to
//! render a list, the bell already knows how to display a count.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, GestureClick, Label, Orientation, Popover, PositionType,
};

const ICON_BELL: &str = "\u{f0f3}";

/// Shared, in-memory notification history. Stage-6 has no live
/// producer; Stage 9 will replace this with a D-Bus-fed channel.
#[derive(Default, Clone)]
struct Entry {
    summary: String,
    body: String,
}

pub fn build() -> GtkBox {
    let row = GtkBox::builder()
        .name("notifications")
        .orientation(Orientation::Horizontal)
        .spacing(0)
        .build();
    row.add_css_class("module");
    row.add_css_class("notifications");

    let icon = Label::builder().label(ICON_BELL).build();
    icon.add_css_class("notif-icon");
    row.append(&icon);

    // Count badge — hidden until Stage 9 starts writing into the
    // history. Kept here so the layout doesn't shift the moment
    // the daemon lands.
    let badge = Label::builder().label("").visible(false).build();
    badge.add_css_class("notif-badge");
    row.append(&badge);

    let history: Rc<RefCell<Vec<Entry>>> = Rc::new(RefCell::new(Vec::new()));

    let popover = build_popover(&history);
    popover.set_parent(&row);

    let click = GestureClick::builder().button(1).build();
    let popover_for_click = popover.clone();
    click.connect_pressed(move |_, _, _, _| popover_for_click.popup());
    row.add_controller(click);

    row
}

fn build_popover(history: &Rc<RefCell<Vec<Entry>>>) -> Popover {
    let body = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .build();
    body.add_css_class("notif-popup");

    let heading = Label::builder()
        .label("Notifications")
        .halign(Align::Start)
        .build();
    heading.add_css_class("notif-heading");
    body.append(&heading);

    let list = GtkBox::builder()
        .name("notif-list")
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    body.append(&list);

    refresh_list(&list, &history.borrow());

    let popover = Popover::builder()
        .child(&body)
        .position(PositionType::Bottom)
        .has_arrow(true)
        .autohide(true)
        .build();
    popover.add_css_class("popover-notif");

    let list_for_show = list.clone();
    let history_for_show = history.clone();
    popover.connect_show(move |_| {
        refresh_list(&list_for_show, &history_for_show.borrow());
    });

    popover
}

fn refresh_list(list: &GtkBox, history: &[Entry]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    if history.is_empty() {
        let empty = Label::builder()
            .label("No notifications")
            .halign(Align::Center)
            .build();
        empty.add_css_class("notif-empty");
        list.append(&empty);
        return;
    }

    for e in history.iter().take(20) {
        let card = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .build();
        card.add_css_class("notif-card");
        let summary = Label::builder().label(&e.summary).halign(Align::Start).build();
        summary.add_css_class("notif-summary");
        let body = Label::builder()
            .label(&e.body)
            .halign(Align::Start)
            .wrap(true)
            .build();
        body.add_css_class("notif-body");
        card.append(&summary);
        if !e.body.is_empty() {
            card.append(&body);
        }
        list.append(&card);
    }
}
