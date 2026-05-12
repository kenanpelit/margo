//! Twilight (night-light) indicator. Uses margo's built-in blue-
//! light filter — `mctl twilight toggle` flips it, state comes
//! from `state.json:twilight`. Sun glyph during day, moon during
//! night, dimmed when disabled.

use gtk::prelude::*;

use crate::services::twilight;
use crate::widgets::indicator::Indicator;

const ICON_DAY: &str = "\u{f185}";       // nf-fa-sun
const ICON_NIGHT: &str = "\u{f186}";     // nf-fa-moon
const ICON_TRANSITION: &str = "\u{f76b}"; // nf-md-weather_sunset

const REFRESH_SECS: u32 = 5;

pub fn build() -> Option<Indicator> {
    let initial = twilight::current()?;
    let ind = Indicator::icon_only("twilight", glyph_for(&initial))
        .on_click(twilight::toggle);
    apply(&ind, &initial);

    let widget = ind.widget.clone();
    let icon = ind.icon.clone();
    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        if let Some(snap) = twilight::current() {
            icon.set_label(glyph_for(&snap));
            apply_classes(&widget, &snap);
        }
        glib::ControlFlow::Continue
    });

    Some(ind)
}

fn apply(ind: &Indicator, snap: &twilight::Snapshot) {
    apply_classes(&ind.widget, snap);
}

fn apply_classes(widget: &gtk::Button, snap: &twilight::Snapshot) {
    widget.remove_css_class("disabled");
    widget.remove_css_class("day");
    widget.remove_css_class("night");
    widget.remove_css_class("transition");
    if !snap.enabled {
        widget.add_css_class("disabled");
    } else {
        widget.add_css_class(&snap.phase);
    }
}

fn glyph_for(snap: &twilight::Snapshot) -> &'static str {
    if !snap.enabled {
        return ICON_DAY;
    }
    match snap.phase.as_str() {
        "night" => ICON_NIGHT,
        "transition" => ICON_TRANSITION,
        _ => ICON_DAY,
    }
}
