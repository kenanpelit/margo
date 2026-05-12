//! Battery ring + system-detail popover.
//!
//! Bar widget: eww `(bat)` — small ring + Nerd-Font battery glyph.
//! Click opens a popover (eww `system_win`) with the capacity %,
//! charging status and an extra "low" indicator when the cell is
//! below 20 %.

use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, GestureClick, Label, Orientation, Popover, PositionType};

use crate::services::battery::{Snapshot, Status};
use crate::widgets::circular::Ring;

const REFRESH_SECS: u32 = 15;
const BATTERY_GLYPH: &str = "\u{f240}"; // nf-fa-battery_full

pub fn build() -> Option<Ring> {
    let initial = Snapshot::current()?;
    let ring = crate::widgets::circular::build("battery", BATTERY_GLYPH, "ring-fg-batt");
    apply(&ring, initial);

    // Detail popover anchored under the ring.
    let popover = build_popover();
    popover.set_parent(&ring.widget);

    let popover_for_click = popover.clone();
    let click = GestureClick::builder().button(1).build();
    click.connect_pressed(move |_, _, _, _| {
        refresh_popover(&popover_for_click);
        popover_for_click.popup();
    });
    ring.widget.add_controller(click);

    let tick_ring = ring.clone();
    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        if let Some(snap) = Snapshot::current() {
            apply(&tick_ring, snap);
        }
        glib::ControlFlow::Continue
    });

    Some(ring)
}

fn apply(ring: &Ring, snap: Snapshot) {
    ring.set_value(snap.capacity as f64 / 100.0);
    ring.widget.remove_css_class("charging");
    ring.widget.remove_css_class("full");
    ring.widget.remove_css_class("low");
    match snap.status {
        Status::Charging => ring.widget.add_css_class("charging"),
        Status::Full => ring.widget.add_css_class("full"),
        _ if snap.capacity < 20 => ring.widget.add_css_class("low"),
        _ => {}
    }
}

fn build_popover() -> Popover {
    let body = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .halign(Align::Start)
        .build();
    body.add_css_class("sys-popup");

    let heading = Label::builder()
        .label("Battery")
        .halign(Align::Start)
        .build();
    heading.add_css_class("sys-popup-heading");

    let capacity = Label::builder()
        .name("battery-capacity")
        .halign(Align::Start)
        .build();
    capacity.add_css_class("sys-popup-line");

    let status = Label::builder()
        .name("battery-status")
        .halign(Align::Start)
        .build();
    status.add_css_class("sys-popup-line");

    body.append(&heading);
    body.append(&capacity);
    body.append(&status);

    let popover = Popover::builder()
        .child(&body)
        .position(PositionType::Bottom)
        .has_arrow(true)
        .autohide(true)
        .build();
    popover.add_css_class("popover-sys");
    popover
}

fn refresh_popover(popover: &Popover) {
    let Some(snap) = Snapshot::current() else {
        return;
    };
    let Some(child) = popover.child() else {
        return;
    };
    for_each_label(&child, &mut |lbl| match lbl.widget_name().as_str() {
        "battery-capacity" => lbl.set_text(&format!("{}%", snap.capacity)),
        "battery-status" => {
            let status = match snap.status {
                Status::Charging => "Charging",
                Status::Discharging => "On battery",
                Status::Full => "Fully charged",
                Status::Unknown => "Unknown",
            };
            lbl.set_text(status);
        }
        _ => {}
    });
}

fn for_each_label(widget: &gtk::Widget, f: &mut impl FnMut(&Label)) {
    if let Some(lbl) = widget.downcast_ref::<Label>() {
        f(lbl);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        for_each_label(&c, f);
        child = c.next_sibling();
    }
}
