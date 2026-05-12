//! Tempo (clock) module — Noctalia-style single-line clock.
//!
//! Renders the format string from
//! `~/.cachy/modules/noctalia/dotfiles/noctalia/settings.json`
//! → `bar.widgets.center[1].formatHorizontal` which the user has
//! set to `HH:mm ddd, MMM dd`. We hard-code the same format here
//! — once mshell grows a config file (Stage post-rewrite) it'll
//! become user-tunable.
//!
//! Click → GtkCalendar popover, matching Noctalia's behaviour.

use chrono::Local;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Calendar, GestureClick, Label, Orientation, Popover, PositionType,
};

const CLOCK_FORMAT: &str = "%H:%M  %a, %b %d";

pub fn build() -> GtkBox {
    let row = GtkBox::builder()
        .name("tempo")
        .orientation(Orientation::Horizontal)
        .build();
    row.add_css_class("module");

    let clock = Label::builder().name("clock-time").build();
    row.append(&clock);

    refresh(&clock);

    let clock_tick = clock.clone();
    glib::timeout_add_seconds_local(20, move || {
        refresh(&clock_tick);
        glib::ControlFlow::Continue
    });

    let cal = Calendar::new();
    cal.add_css_class("calendar");
    let popover = Popover::builder()
        .child(&cal)
        .position(PositionType::Bottom)
        .has_arrow(true)
        .autohide(true)
        .build();
    popover.add_css_class("popover-calendar");
    popover.set_parent(&row);

    let click = GestureClick::builder().button(1).build();
    let popover_for_click = popover.clone();
    click.connect_pressed(move |_, _, _, _| popover_for_click.popup());
    row.add_controller(click);

    row
}

fn refresh(label: &Label) {
    let now = Local::now();
    label.set_text(&now.format(CLOCK_FORMAT).to_string());
}
