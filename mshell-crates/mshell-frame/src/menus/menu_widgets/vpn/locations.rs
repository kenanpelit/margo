//! The VPN menu's location list: countries, each opening to its cities,
//! from `mvpn locations` (the relay list as a country → city tree).
//!
//! A country row connects to a random relay in the country; its chevron
//! (only when it has more than one city) reveals one row per city, each
//! connecting to a random relay there. Search keeps a country whose name
//! matches with all its cities, or just the matching cities (shown open).

use mshell_utils::mvpn::locations::{self, Country};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use std::collections::HashSet;
use std::rc::Rc;

/// What a click in the list asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocationAction {
    /// Open / close a country's cities.
    Toggle(String),
    /// Connect to a random relay in the country (`mvpn <cc>`).
    Country(String),
    /// Connect to a random relay in the city (`mvpn <cc> <city>`).
    City(String, String),
}

/// The list's state, as the menu holds it.
pub(crate) struct LocationList<'a> {
    pub countries: &'a [Country],
    pub filter: &'a str,
    pub expanded: &'a HashSet<String>,
    /// `mvpn locations` has answered (an empty answer is then "none",
    /// not "loading").
    pub loaded: bool,
}

/// Clear and refill `b` with the rows `list` shows.
pub(crate) fn rebuild(b: &gtk::Box, list: &LocationList<'_>, on: Rc<dyn Fn(LocationAction)>) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    let rows = locations::filter(list.countries, list.filter);
    if rows.is_empty() {
        let msg = if !list.loaded {
            "Loading locations…"
        } else if list.countries.is_empty() {
            "No locations: is the Mullvad service running?"
        } else {
            "No matches."
        };
        let l = gtk::Label::new(Some(msg));
        l.add_css_class("label-small");
        l.add_css_class("dim-label");
        l.set_xalign(0.0);
        b.append(&l);
        return;
    }
    for (country, cities, by_city) in rows {
        let open = by_city || list.expanded.contains(&country.code);
        b.append(&country_row(country, open, &on));
        if open && country.cities.len() > 1 {
            for city in cities {
                let (cc, code) = (country.code.clone(), city.code.clone());
                let on = on.clone();
                let row = place_row(
                    &city.name,
                    &city.relays.to_string(),
                    &format!("Connect to {}", city.name),
                    move || on(LocationAction::City(cc.clone(), code.clone())),
                );
                row.add_css_class("vpn-city-row");
                b.append(&row);
            }
        }
    }
}

/// A country row: chevron (more than one city), name, relay count and
/// Connect. A one-city country names its city in the count instead.
fn country_row(country: &Country, open: bool, on: &Rc<dyn Fn(LocationAction)>) -> gtk::Box {
    let count = match country.cities.as_slice() {
        [only] => format!("{} · {}", only.name, only.relays),
        _ => country.relays().to_string(),
    };
    let code = country.code.clone();
    let connect_on = on.clone();
    let row = place_row(
        &country.name,
        &count,
        &format!("Connect to {}", country.name),
        move || connect_on(LocationAction::Country(code.clone())),
    );
    if country.cities.len() > 1 {
        let chevron = gtk::Button::from_icon_name(if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        });
        chevron.set_css_classes(&["flat", "vpn-chevron"]);
        chevron.set_valign(gtk::Align::Center);
        mshell_common::a11y::name(
            &chevron,
            &format!(
                "{} cities in {}",
                if open { "Hide" } else { "Show" },
                country.name
            ),
        );
        let (code, on) = (country.code.clone(), on.clone());
        chevron.connect_clicked(move |_| on(LocationAction::Toggle(code.clone())));
        row.prepend(&chevron);
    }
    row
}

/// One `name · count · Connect` row.
fn place_row(name: &str, count: &str, tooltip: &str, on_connect: impl Fn() + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("ok-button-surface");
    let label = gtk::Label::new(Some(name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let n = gtk::Label::new(Some(count));
    n.add_css_class("label-small");
    n.add_css_class("dim-label");
    let connect = gtk::Button::with_label("Connect");
    connect.set_css_classes(&["ok-button-surface", "dns-action"]);
    connect.set_tooltip_text(Some(tooltip));
    connect.connect_clicked(move |_| on_connect());
    row.append(&label);
    row.append(&n);
    row.append(&connect);
    row
}

/// The `mvpn` argv a connect click runs.
pub(crate) fn connect_args(action: &LocationAction) -> Option<Vec<String>> {
    match action {
        LocationAction::Toggle(_) => None,
        LocationAction::Country(cc) => Some(vec![cc.clone()]),
        LocationAction::City(cc, city) => Some(vec![cc.clone(), city.clone()]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_args_name_the_country_and_city() {
        assert_eq!(
            connect_args(&LocationAction::Country("de".into())),
            Some(vec!["de".to_string()])
        );
        assert_eq!(
            connect_args(&LocationAction::City("us".into(), "nyc".into())),
            Some(vec!["us".to_string(), "nyc".to_string()])
        );
        assert_eq!(connect_args(&LocationAction::Toggle("de".into())), None);
    }
}
