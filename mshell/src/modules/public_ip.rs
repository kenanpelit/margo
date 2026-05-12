//! Public IP indicator — Noctalia's `plugin:6ee06e:nip`.
//!
//! Icon + IP text; hidden when curl can't reach the API (offline /
//! VPN switch transition). Refreshes every 15 min — the address
//! shouldn't churn faster than that on a normal connection.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::services::public_ip;
use crate::widgets::indicator::Indicator;

const ICON: &str = "\u{f0ac}"; // nf-fa-globe
const REFRESH_SECS: u32 = 15 * 60;

pub fn build() -> Indicator {
    // First IP is fetched in a deferred glib-idle tick so the
    // initial bar paint doesn't block on a 5 s curl timeout.
    let ind = Indicator::icon_text("public-ip", ICON, "");
    ind.widget.set_visible(false);

    let label = ind.label.clone();
    let widget = ind.widget.clone();
    let cached: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    let label_init = label.clone();
    let widget_init = widget.clone();
    let cached_init = cached.clone();
    glib::idle_add_local_once(move || {
        sync(&label_init, &widget_init, &cached_init);
    });

    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        sync(&label, &widget, &cached);
        glib::ControlFlow::Continue
    });

    ind
}

fn sync(label: &Option<gtk::Label>, widget: &gtk::Button, cached: &Rc<RefCell<String>>) {
    let ip = public_ip::fetch();
    if ip.is_empty() {
        widget.set_visible(false);
        return;
    }
    if *cached.borrow() != ip {
        cached.replace(ip.clone());
        if let Some(lbl) = label {
            lbl.set_text(&ip);
        }
    }
    widget.set_visible(true);
}
