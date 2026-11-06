//! The panel's Locations list: countries, each opening to its cities, from
//! the relay list as a country → city tree (`mvpn locations`, filtered by
//! `mvpn_proto::locations::filter` — the same tree and search as the shell
//! VPN menu). A country row connects to a random relay in the country
//! (`mvpn <cc>`), a city row to one in the city (`mvpn <cc> <city>`).

use std::collections::HashSet;
use std::rc::Rc;

use gtk4::prelude::*;

use mvpn_proto::locations::{self, Country};

/// What a click in the list asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    /// Open / close a country's cities.
    Toggle(String),
    /// Run `mvpn <args>` (connect there).
    Connect(Vec<String>),
}

/// Clear and refill `b` with the rows `countries` / `needle` show.
pub fn rebuild(
    b: &gtk4::Box,
    countries: &[Country],
    loaded: bool,
    needle: &str,
    expanded: &HashSet<String>,
    on: Rc<dyn Fn(Click)>,
) {
    super::clear_box(b);
    let rows = locations::filter(countries, needle);
    if rows.is_empty() {
        b.append(&super::dim(if !loaded {
            "Loading locations…"
        } else if countries.is_empty() {
            "No locations: is the Mullvad service running?"
        } else {
            "No matches."
        }));
        return;
    }
    for (country, cities, by_city) in rows {
        let open = by_city || expanded.contains(&country.code);
        b.append(&country_row(country, open, &on));
        if open && country.cities.len() > 1 {
            for city in cities {
                let row = super::relay_row(&city.name, "", &city.relays.to_string());
                row.add_css_class("mvpn-city");
                row.set_tooltip_text(Some(&format!("Connect to {}", city.name)));
                let (on, args) = (on.clone(), vec![country.code.clone(), city.code.clone()]);
                row.connect_clicked(move |_| on(Click::Connect(args.clone())));
                b.append(&row);
            }
        }
    }
}

/// A country row: chevron (more than one city), then the connect button
/// with the code, name and relay count. A one-city country names its city
/// in the count instead.
fn country_row(country: &Country, open: bool, on: &Rc<dyn Fn(Click)>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    if country.cities.len() > 1 {
        let chevron = gtk4::Button::from_icon_name(if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        });
        chevron.add_css_class("mvpn-action");
        chevron.add_css_class("mvpn-chevron");
        let name = format!(
            "{} cities in {}",
            if open { "Hide" } else { "Show" },
            country.name
        );
        chevron.set_tooltip_text(Some(&name));
        chevron.update_property(&[gtk4::accessible::Property::Label(&name)]);
        let (on, code) = (on.clone(), country.code.clone());
        chevron.connect_clicked(move |_| on(Click::Toggle(code.clone())));
        row.append(&chevron);
    }
    let count = match country.cities.as_slice() {
        [only] => format!("{} · {}", only.name, only.relays),
        _ => format!("{} relays", country.relays()),
    };
    let connect = super::relay_row(&country.name, &country.code.to_uppercase(), &count);
    connect.set_hexpand(true);
    connect.set_tooltip_text(Some(&format!("Connect to {}", country.name)));
    let (on, args) = (on.clone(), vec![country.code.clone()]);
    connect.connect_clicked(move |_| on(Click::Connect(args.clone())));
    row.append(&connect);
    row
}
