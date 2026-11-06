// SPDX-License-Identifier: GPL-3.0-or-later

//! The station directory: search radio-browser.info by name, country and
//! genre, ordered by votes / trend / name, a page at a time.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use gtk::glib;

use super::{
    Ctx, play,
    rows::{self, States},
    shared::{self, Change},
};
use crate::{
    i18n::i18n,
    net,
    radio::{
        Station,
        api::{self, Order, Query},
    },
};

struct Page {
    ctx: Rc<Ctx>,
    search: gtk::SearchEntry,
    country: gtk::DropDown,
    genre: gtk::DropDown,
    order: gtk::DropDown,
    list: gtk::ListBox,
    more: gtk::Button,
    states: States,
    /// ISO codes parallel to the country dropdown ("" = all countries).
    codes: RefCell<Vec<String>>,
    /// Everything loaded for the current query, in list order.
    results: RefCell<Vec<Station>>,
    /// (station key, star) for live re-sync with My Stations.
    stars: RefCell<Vec<(String, gtk::ToggleButton)>>,
    query: RefCell<Query>,
    /// Bumped per search; a late answer for an older one is dropped.
    generation: Cell<u64>,
    /// Set while the page itself changes widgets (no feedback loops).
    quiet: Cell<bool>,
}

fn dropdown(items: &[String]) -> gtk::DropDown {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let dd = gtk::DropDown::builder()
        .model(&gtk::StringList::new(&refs))
        .enable_search(items.len() > 12)
        .hexpand(true)
        .build();
    // Type-to-filter matches the item text.
    dd.set_expression(Some(gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        gtk::Expression::NONE,
        "string",
    )));
    dd
}

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let search = gtk::SearchEntry::builder()
        .placeholder_text(i18n("Search stations"))
        .hexpand(true)
        .build();
    let country = dropdown(&[i18n("All countries")]);
    country.set_tooltip_text(Some(&i18n("Country")));
    let mut genres = vec![i18n("Any genre")];
    genres.extend(api::GENRES.iter().map(|g| g.to_string()));
    let genre = dropdown(&genres);
    genre.set_tooltip_text(Some(&i18n("Genre")));
    let order = dropdown(&[i18n("Most voted"), i18n("Trending"), i18n("Name")]);
    order.set_tooltip_text(Some(&i18n("Order")));

    let filters = gtk::Box::builder().spacing(6).homogeneous(true).build();
    filters.append(&country);
    filters.append(&genre);
    filters.append(&order);

    let list = rows::boxed_list();
    let more = gtk::Button::builder()
        .label(i18n("Load More"))
        .halign(gtk::Align::Center)
        .css_classes(["pill"])
        .visible(false)
        .build();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(6)
        .margin_bottom(18)
        .margin_start(12)
        .margin_end(12)
        .build();
    column.append(&list);
    column.append(&more);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(
            &adw::Clamp::builder()
                .maximum_size(720)
                .child(&column)
                .build(),
        )
        .build();
    let states = States::new(&scroller);

    let top = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    top.append(&search);
    top.append(&filters);
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    root.append(&adw::Clamp::builder().maximum_size(720).child(&top).build());
    root.append(&states.stack);

    let page = Rc::new(Page {
        ctx: ctx.clone(),
        search,
        country,
        genre,
        order,
        list,
        more,
        states,
        codes: RefCell::new(vec![String::new()]),
        results: RefCell::new(Vec::new()),
        stars: RefCell::new(Vec::new()),
        query: RefCell::new(Query::default()),
        generation: Cell::new(0),
        quiet: Cell::new(false),
    });
    page.wire();
    page.load_countries();
    page.run(page.query_from_widgets());
    // Every handler holds the page weakly; the page lives as long as its
    // widgets.
    root.connect_destroy(move |_| {
        let _ = &page;
    });
    root.upcast()
}

impl Page {
    fn wire(self: &Rc<Self>) {
        let p = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            if let Some(p) = p.upgrade()
                && !p.quiet.get()
            {
                p.run(p.query_from_widgets());
            }
        });
        for dd in [&self.country, &self.genre, &self.order] {
            let p = Rc::downgrade(self);
            dd.connect_selected_notify(move |_| {
                if let Some(p) = p.upgrade()
                    && !p.quiet.get()
                {
                    p.remember_country();
                    p.run(p.query_from_widgets());
                }
            });
        }
        let p = Rc::downgrade(self);
        self.more.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                let next = p.query.borrow().next_page();
                p.fetch(next, true);
            }
        });
        let p = Rc::downgrade(self);
        self.states.on_retry(move || {
            if let Some(p) = p.upgrade() {
                p.run(p.query_from_widgets());
            }
        });
        let p = Rc::downgrade(self);
        self.ctx.listen(move |change| {
            if change == Change::Stations
                && let Some(p) = p.upgrade()
            {
                p.sync_stars();
            }
        });
    }

    fn query_from_widgets(&self) -> Query {
        let genre = self.genre.selected() as usize;
        Query {
            name: self.search.text().to_string(),
            countrycode: self
                .codes
                .borrow()
                .get(self.country.selected() as usize)
                .cloned()
                .unwrap_or_default(),
            tag: genre
                .checked_sub(1)
                .and_then(|i| api::GENRES.get(i))
                .map(|g| g.to_string())
                .unwrap_or_default(),
            order: match self.order.selected() {
                1 => Order::Trending,
                2 => Order::Name,
                _ => Order::Votes,
            },
            offset: 0,
        }
    }

    /// Keep the chosen country for next time.
    fn remember_country(&self) {
        let code = self
            .codes
            .borrow()
            .get(self.country.selected() as usize)
            .cloned()
            .unwrap_or_default();
        let lib = shared::radio();
        if lib.data.borrow().country != code {
            lib.data.borrow_mut().country = code;
            shared::radio_changed();
        }
    }

    fn load_countries(self: &Rc<Self>) {
        let p = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Some(Ok(countries)) = net::off_main(api::countries).await else {
                return; // The dropdown keeps "All countries"; searches still work.
            };
            let Some(p) = p.upgrade() else { return };
            let saved = shared::radio().data.borrow().country.clone();
            let mut names = vec![i18n("All countries")];
            let mut codes = vec![String::new()];
            for c in countries {
                names.push(c.name);
                codes.push(c.code);
            }
            let selected = codes.iter().position(|c| *c == saved).unwrap_or(0);
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            p.quiet.set(true);
            *p.codes.borrow_mut() = codes;
            p.country.set_model(Some(&gtk::StringList::new(&refs)));
            p.country.set_enable_search(true);
            p.country.set_selected(selected as u32);
            p.quiet.set(false);
            if selected != 0 {
                p.run(p.query_from_widgets());
            }
        });
    }

    /// A new search from the first page.
    fn run(self: &Rc<Self>, query: Query) {
        self.states.loading();
        self.fetch(query, false);
    }

    fn fetch(self: &Rc<Self>, query: Query, append: bool) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.more.set_sensitive(false);
        *self.query.borrow_mut() = query.clone();
        let p = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let res = net::off_main(move || api::search(&query)).await;
            let Some(p) = p.upgrade() else { return };
            if p.generation.get() != generation {
                return;
            }
            p.more.set_sensitive(true);
            match res {
                Some(Ok(stations)) => p.show(stations, append),
                Some(Err(e)) if append => p.ctx.toast(&e.to_string()),
                Some(Err(e)) => p.states.status(
                    "network-error-symbolic",
                    &i18n("Cannot Reach the Station Directory"),
                    &e.to_string(),
                    Some(&i18n("Try Again")),
                ),
                None => {}
            }
        });
    }

    fn show(self: &Rc<Self>, stations: Vec<Station>, append: bool) {
        if !append {
            rows::clear(&self.list);
            self.results.borrow_mut().clear();
            self.stars.borrow_mut().clear();
        }
        let full_page = stations.len() as u32 >= api::PAGE;
        let favourites = shared::radio();
        for st in stations {
            if self.results.borrow().iter().any(|s| s.url == st.url) {
                continue; // a later page repeating an entry
            }
            let index = self.results.borrow().len();
            let star = rows::star_button(favourites.data.borrow().contains(&st.key()));
            let p = Rc::downgrade(self);
            let station = st.clone();
            star.connect_toggled(move |b| {
                if let Some(p) = p.upgrade()
                    && !p.quiet.get()
                {
                    let lib = shared::radio();
                    let now = lib.data.borrow_mut().toggle_favourite(&station);
                    if now != b.is_active() {
                        // Out of sync with the file: follow the library.
                        p.quiet.set(true);
                        b.set_active(now);
                        p.quiet.set(false);
                    }
                    shared::radio_changed();
                }
            });
            let p = Rc::downgrade(self);
            let row = rows::station_row(&st, &[star.clone().upcast()], move || {
                if let Some(p) = p.upgrade() {
                    play::play_stations(&p.ctx, &p.results.borrow(), index);
                }
            });
            self.list.append(&row);
            self.stars.borrow_mut().push((st.key(), star));
            self.results.borrow_mut().push(st);
        }
        self.more.set_visible(full_page);
        if self.results.borrow().is_empty() {
            self.states.status(
                "edit-find-symbolic",
                &i18n("No Stations Found"),
                &i18n("Try another name, country or genre"),
                None,
            );
        } else {
            self.states.show_content();
        }
    }

    /// Re-check every star after My Stations changed elsewhere.
    fn sync_stars(&self) {
        let lib = shared::radio();
        let lib = lib.data.borrow();
        self.quiet.set(true);
        for (key, star) in self.stars.borrow().iter() {
            let fav = lib.contains(key);
            if star.is_active() != fav {
                star.set_active(fav);
            }
        }
        self.quiet.set(false);
    }
}
