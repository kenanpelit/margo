//! Bluetooth indicator. Adapter glyph that flips between
//! powered-on (Dracula tertiary cyan) and powered-off (outline
//! grey). Click toggles the adapter via `bluetoothctl power`.

use gtk::prelude::*;

use crate::services::bluetooth;
use crate::widgets::indicator::Indicator;

const ICON_ON: &str = "\u{f293}"; // nf-fa-bluetooth_b
const ICON_OFF: &str = "\u{f294}"; // nf-fa-bluetooth (lighter)
const REFRESH_SECS: u32 = 5;

pub fn build() -> Indicator {
    let snap = bluetooth::current();
    let ind = Indicator::icon_only("bluetooth", glyph_for(snap.enabled))
        .on_click(bluetooth::toggle_power);
    apply(&ind, &snap);

    let widget = ind.widget.clone();
    let icon = ind.icon.clone();
    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        let snap = bluetooth::current();
        icon.set_label(glyph_for(snap.enabled));
        widget.remove_css_class("off");
        widget.remove_css_class("connected");
        if !snap.enabled {
            widget.add_css_class("off");
        } else if snap.connected_devices > 0 {
            widget.add_css_class("connected");
        }
        glib::ControlFlow::Continue
    });

    ind
}

fn apply(ind: &Indicator, snap: &bluetooth::Snapshot) {
    if !snap.enabled {
        ind.widget.add_css_class("off");
    } else if snap.connected_devices > 0 {
        ind.widget.add_css_class("connected");
    }
}

fn glyph_for(enabled: bool) -> &'static str {
    if enabled { ICON_ON } else { ICON_OFF }
}
