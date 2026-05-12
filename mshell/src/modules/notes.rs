//! Notes / scratchpad widget — Noctalia's `plugin:notes`.
//!
//! Pencil icon in the bar; click → popover with a GtkTextView
//! that's wired to the on-disk scratchpad. Autosave is debounced
//! ~700 ms after the last edit (matches Noctalia's `autosaveDelay`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, GestureClick, Label, Orientation, Popover, PositionType, ScrolledWindow,
    TextBuffer, TextView, WrapMode,
};

use crate::services::notes;
use crate::widgets::indicator::Indicator;

const ICON: &str = "\u{f249}"; // sticky note
const AUTOSAVE_MS: u64 = 700;

pub fn build() -> Indicator {
    let ind = Indicator::icon_only("notes", ICON);

    let popover = build_popover();
    popover.set_parent(&ind.widget);

    let popover_for_click = popover.clone();
    ind.widget.connect_clicked(move |_| popover_for_click.popup());

    ind
}

fn build_popover() -> Popover {
    let body = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .build();
    body.add_css_class("notes-popup");

    let heading = Label::builder()
        .label("Scratchpad")
        .halign(Align::Start)
        .build();
    heading.add_css_class("notes-heading");
    body.append(&heading);

    let buffer = TextBuffer::new(None);
    buffer.set_text(&notes::load_scratchpad());

    let view = TextView::builder()
        .buffer(&buffer)
        .wrap_mode(WrapMode::Word)
        .accepts_tab(false)
        .build();
    view.add_css_class("notes-view");

    let scroller = ScrolledWindow::builder()
        .child(&view)
        .min_content_width(380)
        .min_content_height(220)
        .build();
    scroller.add_css_class("notes-scroller");
    body.append(&scroller);

    // Debounced autosave. Every `changed` signal cancels the
    // previous pending save and schedules a fresh one 700 ms out;
    // typing fast = one save when the user stops.
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let buffer_for_signal = buffer.clone();
    buffer.connect_changed(move |_| {
        if let Some(id) = pending.borrow_mut().take() {
            id.remove();
        }
        let pending_set = pending.clone();
        let buffer = buffer_for_signal.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(AUTOSAVE_MS), move || {
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), true)
                .to_string();
            notes::save_scratchpad(&text);
            *pending_set.borrow_mut() = None;
        });
        *pending.borrow_mut() = Some(id);
    });

    let popover = Popover::builder()
        .child(&body)
        .position(PositionType::Bottom)
        .has_arrow(true)
        .autohide(true)
        .build();
    popover.add_css_class("popover-notes");

    // Save synchronously on hide so a "click outside" doesn't lose
    // anything typed in the last 700 ms before the autosave tick.
    let buffer_for_hide = buffer.clone();
    popover.connect_hide(move |_| {
        let text = buffer_for_hide
            .text(
                &buffer_for_hide.start_iter(),
                &buffer_for_hide.end_iter(),
                true,
            )
            .to_string();
        notes::save_scratchpad(&text);
    });

    popover
}

// Suppress the unused warning until popover-only paths get used
// by another caller.
#[allow(dead_code)]
fn _unused() {
    let _ = GestureClick::builder();
}
