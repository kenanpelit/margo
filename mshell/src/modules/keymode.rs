//! Margo binding-mode indicator. Hidden when the mode is the
//! default (no signal worth showing); a coloured pill when a
//! resize / move / scratchpad mode is active.

use gtk::prelude::*;

use crate::services::keymode;
use crate::widgets::indicator::Indicator;

const ICON: &str = "\u{f11c}"; // keyboard
const REFRESH_SECS: u32 = 2;

pub fn build() -> Indicator {
    let initial = keymode::current();
    let ind = Indicator::icon_text("keymode", ICON, &initial);
    apply(&ind, &initial);

    let widget = ind.widget.clone();
    let label = ind.label.clone();
    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        let mode = keymode::current();
        widget.set_visible(mode != "default");
        if let Some(lbl) = &label {
            lbl.set_text(&mode);
        }
        widget.remove_css_class("resize");
        widget.remove_css_class("move");
        widget.remove_css_class("scratchpad");
        if mode != "default" {
            widget.add_css_class(&mode);
        }
        glib::ControlFlow::Continue
    });

    ind
}

fn apply(ind: &Indicator, mode: &str) {
    ind.widget.set_visible(mode != "default");
    if mode != "default" {
        ind.widget.add_css_class(mode);
    }
}
