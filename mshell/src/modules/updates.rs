//! Pending updates indicator.
//!
//! Polls `checkupdates` every 30 minutes (matching the old iced
//! mshell default) and shows count + arrow glyph. Hidden when 0.
//! Click → spawn `alacritty -e bash -c 'paru; read'` to apply,
//! same as the iced module's `update_cmd`.

use std::process::Command;

use gtk::prelude::*;

use crate::services::updates;
use crate::widgets::indicator::Indicator;

const ICON: &str = "\u{f062}"; // nf-fa-arrow_up
const REFRESH_SECS: u32 = 60 * 30;

pub fn build() -> Indicator {
    let count = updates::count();
    let ind = Indicator::icon_text("updates", ICON, &count.to_string())
        .on_click(|| {
            let _ = Command::new("alacritty")
                .args(["-e", "bash", "-c", "paru; echo; echo Done — press enter; read"])
                .spawn();
        });
    apply_state(&ind, count);

    let ind_tick = ind.widget.clone();
    let label_tick = ind.label.clone();
    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        let count = updates::count();
        if let Some(label) = &label_tick {
            label.set_text(&count.to_string());
        }
        ind_tick.set_visible(count > 0);
        glib::ControlFlow::Continue
    });

    ind
}

fn apply_state(ind: &Indicator, count: u32) {
    ind.widget.set_visible(count > 0);
}
