//! Power profile indicator. Click cycles balanced → performance →
//! power-saver → balanced via `powerprofilesctl set`. Hidden when
//! powerprofilesctl isn't installed (desktops without TLP-style
//! profiles, mostly).

use gtk::prelude::*;

use crate::services::power_profile;
use crate::widgets::indicator::Indicator;

const REFRESH_SECS: u32 = 5;

pub fn build() -> Option<Indicator> {
    let initial = power_profile::current()?;
    let ind = Indicator::icon_only("power", glyph_for(&initial))
        .on_click(power_profile::cycle);
    apply_class(&ind, &initial);

    let icon = ind.icon.clone();
    let widget = ind.widget.clone();
    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        if let Some(p) = power_profile::current() {
            icon.set_label(glyph_for(&p));
            widget.remove_css_class("balanced");
            widget.remove_css_class("performance");
            widget.remove_css_class("power-saver");
            widget.add_css_class(&p);
        }
        glib::ControlFlow::Continue
    });

    Some(ind)
}

fn apply_class(ind: &Indicator, profile: &str) {
    ind.widget.add_css_class(profile);
}

fn glyph_for(profile: &str) -> &'static str {
    match profile {
        "performance" => "\u{f0e7}", // bolt
        "power-saver" => "\u{f06c}", // leaf
        _ => "\u{f042}",             // adjust
    }
}
