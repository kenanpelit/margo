//! Weather indicator — Noctalia's `plugin:weather-indicator`.
//!
//! Icon + temp text on the bar. Tooltip carries the long
//! "Atasehir 18°C, Partly cloudy · 65% humidity · 12 km/h wind"
//! line. Fetched on a background thread so the bar's first paint
//! doesn't block on the curl round-trip.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::services::weather;
use crate::widgets::indicator::Indicator;

const REFRESH_SECS: u32 = 30 * 60; // 30 min

pub fn build() -> Indicator {
    let ind = Indicator::icon_text("weather", "\u{f077}", "—");
    let cached: Rc<RefCell<Option<weather::Snapshot>>> = Rc::new(RefCell::new(None));

    schedule_fetch(&ind, &cached);

    let ind_for_tick = ind_clone(&ind);
    let cached_for_tick = cached.clone();
    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        schedule_fetch(&ind_for_tick, &cached_for_tick);
        glib::ControlFlow::Continue
    });

    ind
}

/// `Indicator` doesn't derive Clone (it holds owned GtkButton/Label
/// children that GTK reference-counts; clone just bumps refs).
fn ind_clone(ind: &Indicator) -> Indicator {
    Indicator {
        widget: ind.widget.clone(),
        icon: ind.icon.clone(),
        label: ind.label.clone(),
    }
}

/// Kick off a background fetch; on completion update the bar
/// widget on the GTK main thread.
fn schedule_fetch(ind: &Indicator, cache: &Rc<RefCell<Option<weather::Snapshot>>>) {
    let ind = ind_clone(ind);
    let cache = cache.clone();
    glib::MainContext::default().spawn_local(async move {
        let snap = gio::spawn_blocking(weather::current)
            .await
            .ok()
            .flatten();
        let Some(snap) = snap else {
            return;
        };
        apply(&ind, &snap);
        *cache.borrow_mut() = Some(snap);
    });
}

fn apply(ind: &Indicator, snap: &weather::Snapshot) {
    ind.icon.set_label(weather::glyph(snap.weather_code));
    if let Some(label) = &ind.label {
        label.set_text(&format!("{}°C", snap.temp_c));
    }
    let tooltip = format!(
        "{} {}°C, {} · {}% humidity · {} km/h wind",
        snap.city,
        snap.temp_c,
        weather::label(snap.weather_code),
        snap.humidity_pct,
        snap.wind_kmh,
    );
    ind.widget.set_tooltip_text(Some(&tooltip));
}
