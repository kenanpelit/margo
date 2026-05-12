//! UFW firewall status indicator. Lock glyph when active, slash
//! glyph when inactive (or no sudo permission). Click is a no-op
//! for now; the iced module opened a popup with default rules
//! which lands in Stage 8 if revived.

use gtk::prelude::*;

use crate::services::ufw;
use crate::widgets::indicator::Indicator;

const ICON_ACTIVE: &str = "\u{f023}"; // lock
const ICON_INACTIVE: &str = "\u{f3ed}"; // lock-open
const REFRESH_SECS: u32 = 30;

pub fn build() -> Indicator {
    let active = ufw::enabled();
    let ind = Indicator::icon_only("ufw", icon_for(active));
    apply(&ind, active);

    let widget = ind.widget.clone();
    let icon = ind.icon.clone();
    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        let active = ufw::enabled();
        icon.set_label(icon_for(active));
        widget.remove_css_class("inactive");
        if !active {
            widget.add_css_class("inactive");
        }
        glib::ControlFlow::Continue
    });

    ind
}

fn apply(ind: &Indicator, active: bool) {
    ind.widget.remove_css_class("inactive");
    if !active {
        ind.widget.add_css_class("inactive");
    }
}

fn icon_for(active: bool) -> &'static str {
    if active { ICON_ACTIVE } else { ICON_INACTIVE }
}
