//! Running podman container count. Hidden when there are none —
//! the indicator is for "you've got containers up", not "podman is
//! installed".

use gtk::prelude::*;

use crate::services::podman;
use crate::widgets::indicator::Indicator;

const ICON: &str = "\u{f308}"; // nf-md-docker (closest podman match)
const REFRESH_SECS: u32 = 15;

pub fn build() -> Indicator {
    let count = podman::running_count();
    let ind = Indicator::icon_text("podman", ICON, &count.to_string());
    ind.widget.set_visible(count > 0);

    let widget = ind.widget.clone();
    let label = ind.label.clone();
    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        let count = podman::running_count();
        if let Some(label) = &label {
            label.set_text(&count.to_string());
        }
        widget.set_visible(count > 0);
        glib::ControlFlow::Continue
    });

    ind
}
