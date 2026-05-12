//! Memory ring + detail popover.
//!
//! Bar widget: eww `(mem)` — ring + Nerd-Font memory glyph. Click
//! opens a popover with used / total GiB + percentage, same shape
//! as the battery popup.

use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, GestureClick, Label, Orientation, Popover, PositionType};

use crate::services::memory::Snapshot;
use crate::widgets::circular::Ring;

const REFRESH_SECS: u32 = 15;
const MEM_GLYPH: &str = "\u{f538}"; // nf-mdi-memory

pub fn build() -> Ring {
    let ring = crate::widgets::circular::build("memory", MEM_GLYPH, "ring-fg-mem");
    if let Some(snap) = Snapshot::current() {
        apply(&ring, snap);
    }

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

    ring
}

fn apply(ring: &Ring, snap: Snapshot) {
    ring.set_value(snap.used_percent as f64 / 100.0);
    ring.widget.remove_css_class("high");
    if snap.used_percent >= 85 {
        ring.widget.add_css_class("high");
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
        .label("Memory")
        .halign(Align::Start)
        .build();
    heading.add_css_class("sys-popup-heading");

    let usage = Label::builder()
        .name("memory-usage")
        .halign(Align::Start)
        .build();
    usage.add_css_class("sys-popup-line");

    let fraction = Label::builder()
        .name("memory-fraction")
        .halign(Align::Start)
        .build();
    fraction.add_css_class("sys-popup-line");

    body.append(&heading);
    body.append(&usage);
    body.append(&fraction);

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
    let used_gib = snap.used_kib as f64 / (1024.0 * 1024.0);
    let total_gib = snap.total_kib as f64 / (1024.0 * 1024.0);
    for_each_label(&child, &mut |lbl| match lbl.widget_name().as_str() {
        "memory-usage" => lbl.set_text(&format!("{}% in use", snap.used_percent)),
        "memory-fraction" => {
            lbl.set_text(&format!("{:.2} / {:.2} GiB", used_gib, total_gib))
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
