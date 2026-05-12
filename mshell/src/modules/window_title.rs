//! Active window title — read straight off margo's state.json.
//!
//! Holds the last-known toplevel title when focus shifts off-toplevel
//! (e.g. into one of mshell's own popup menus once Stage 8 lands) so
//! the bar item doesn't flash empty every time you click something.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Label, pango::EllipsizeMode};

use crate::state::Compositor;

const POLL_MS: u64 = 500;
const MAX_CHARS: i32 = 40;

pub fn build() -> Label {
    let label = Label::builder()
        .name("window-title")
        .max_width_chars(MAX_CHARS)
        .ellipsize(EllipsizeMode::End)
        .build();
    label.add_css_class("module");
    label.add_css_class("window-title");

    let last: Rc<Cell<Option<String>>> = Rc::new(Cell::new(None));

    refresh(&label, &last);

    let label_tick = label.clone();
    let last_tick = last.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(POLL_MS), move || {
        refresh(&label_tick, &last_tick);
        glib::ControlFlow::Continue
    });

    label
}

fn refresh(label: &Label, last: &Rc<Cell<Option<String>>>) {
    let state = Compositor::current();
    let title = state
        .focused_client
        .as_ref()
        .map(|c| c.title.clone())
        .filter(|t| !t.is_empty());

    // Hold the previous value if focus moved onto a non-toplevel
    // surface (`focused_client = None`) or an as-yet-titleless
    // window. Otherwise the bar would blink empty every menu open.
    if let Some(t) = title {
        if last.replace(Some(t.clone())) != Some(t.clone()) {
            label.set_label(&t);
        }
    }
}
