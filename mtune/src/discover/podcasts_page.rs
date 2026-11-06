// SPDX-License-Identifier: GPL-3.0-or-later

//! Podcasts: your subscriptions, iTunes search, or a pasted RSS feed URL.
//! Picking a show opens its episodes (see `episodes_page`).

use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
use gtk::glib;

use super::{
    Ctx, art, episodes_page,
    rows::{self, States},
    shared::{self, Change},
};
use crate::{
    i18n::{i18n, ni18n_f},
    net,
    podcasts::{Podcast, itunes, rss},
};

struct Page {
    ctx: Rc<Ctx>,
    nav: adw::NavigationView,
    search: gtk::SearchEntry,
    heading: gtk::Label,
    list: gtk::ListBox,
    states: States,
    generation: Cell<u64>,
}

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let search = gtk::SearchEntry::builder()
        .placeholder_text(i18n("Search podcasts or paste a feed URL"))
        .search_delay(600)
        .hexpand(true)
        .margin_top(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let heading = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    let list = rows::boxed_list();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(6)
        .margin_bottom(18)
        .margin_start(12)
        .margin_end(12)
        .build();
    column.append(&heading);
    column.append(&list);
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

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    root.append(
        &adw::Clamp::builder()
            .maximum_size(720)
            .child(&search)
            .build(),
    );
    root.append(&states.stack);

    let nav = adw::NavigationView::new();
    nav.add(
        &adw::NavigationPage::builder()
            .title(i18n("Podcasts"))
            .tag("podcasts")
            .child(&root)
            .build(),
    );

    let page = Rc::new(Page {
        ctx: ctx.clone(),
        nav: nav.clone(),
        search,
        heading,
        list,
        states,
        generation: Cell::new(0),
    });
    page.wire();
    page.show_subscriptions();
    // Handlers hold the page weakly; it lives as long as its widgets.
    root.connect_destroy(move |_| {
        let _ = &page;
    });
    nav.upcast()
}

impl Page {
    fn wire(self: &Rc<Self>) {
        let p = Rc::downgrade(self);
        self.search.connect_search_changed(move |e| {
            let Some(p) = p.upgrade() else { return };
            let text = e.text();
            if text.trim().is_empty() {
                p.generation.set(p.generation.get() + 1);
                p.show_subscriptions();
            } else if net::validate_http_url(&text).is_none() {
                p.search(text.trim().to_string());
            }
            // A URL waits for Enter (opening a feed is a deliberate act).
        });
        let p = Rc::downgrade(self);
        self.search.connect_activate(move |e| {
            if let Some(p) = p.upgrade()
                && let Some(url) = net::validate_http_url(&e.text())
            {
                p.open_feed_url(url);
            }
        });
        let p = Rc::downgrade(self);
        self.states.on_retry(move || {
            if let Some(p) = p.upgrade() {
                let text = p.search.text().trim().to_string();
                if !text.is_empty() {
                    p.search(text);
                }
            }
        });
        let p = Rc::downgrade(self);
        self.ctx.listen(move |change| {
            if change == Change::Subscriptions
                && let Some(p) = p.upgrade()
                && p.search.text().trim().is_empty()
            {
                p.show_subscriptions();
            }
        });
    }

    fn show_subscriptions(self: &Rc<Self>) {
        rows::clear(&self.list);
        let subs = shared::podcasts().data.borrow().subscriptions.clone();
        if subs.is_empty() {
            self.states.status(
                "application-rss+xml-symbolic",
                &i18n("No Subscriptions"),
                &i18n("Search for a show above, or paste its RSS feed URL and press Enter"),
                None,
            );
            return;
        }
        self.heading.set_label(&i18n("Subscriptions"));
        for p in subs {
            let subtitle = p.author.clone();
            self.add_row(p, subtitle);
        }
        self.states.show_content();
    }

    fn add_row(self: &Rc<Self>, podcast: Podcast, subtitle: String) {
        let row = adw::ActionRow::builder()
            .title(&podcast.title)
            .subtitle(subtitle)
            .use_markup(false)
            .title_lines(1)
            .subtitle_lines(1)
            .activatable(true)
            .build();
        row.add_prefix(&art::thumbnail(
            &podcast.artwork,
            "application-rss+xml-symbolic",
        ));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let p = Rc::downgrade(self);
        row.connect_activated(move |_| {
            if let Some(p) = p.upgrade() {
                episodes_page::open(&p.ctx, &p.nav, podcast.clone());
            }
        });
        self.list.append(&row);
    }

    fn search(self: &Rc<Self>, term: String) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.states.loading();
        let p = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let t = term.clone();
            let res = net::off_main(move || itunes::search(&t)).await;
            let Some(p) = p.upgrade() else { return };
            if p.generation.get() != generation {
                return;
            }
            match res {
                Some(Ok(found)) if found.is_empty() => p.states.status(
                    "edit-find-symbolic",
                    &i18n("No Podcasts Found"),
                    &i18n("Try another name, or paste the show's RSS feed URL"),
                    None,
                ),
                Some(Ok(found)) => {
                    rows::clear(&p.list);
                    p.heading.set_label(&i18n("Results"));
                    for show in found {
                        let mut parts = vec![show.author.clone()];
                        if !show.genre.is_empty() {
                            parts.push(show.genre.clone());
                        }
                        if show.episodes > 0 {
                            parts.push(ni18n_f(
                                "{} episode",
                                "{} episodes",
                                show.episodes,
                                &[&show.episodes.to_string()],
                            ));
                        }
                        parts.retain(|s| !s.is_empty());
                        p.add_row(show, parts.join(" · "));
                    }
                    p.states.show_content();
                }
                Some(Err(e)) => p.states.status(
                    "network-error-symbolic",
                    &i18n("Search Failed"),
                    &e.to_string(),
                    Some(&i18n("Try Again")),
                ),
                None => {}
            }
        });
    }

    /// A pasted feed: read it once for its title/artwork, then open it.
    fn open_feed_url(self: &Rc<Self>, url: String) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.states.loading();
        let p = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let u = url.clone();
            let res = net::off_main(move || rss::fetch(&u)).await;
            let Some(p) = p.upgrade() else { return };
            if p.generation.get() != generation {
                return;
            }
            match res {
                Some(Ok(feed)) => {
                    let show = Podcast {
                        title: if feed.title.is_empty() {
                            url.clone()
                        } else {
                            feed.title.clone()
                        },
                        author: feed.author.clone(),
                        feed_url: url,
                        artwork: feed.image.clone(),
                        ..Default::default()
                    };
                    episodes_page::remember_feed(&show.feed_url, feed);
                    p.search.set_text("");
                    episodes_page::open(&p.ctx, &p.nav, show);
                }
                Some(Err(e)) => p.states.status(
                    "dialog-warning-symbolic",
                    &i18n("Cannot Read That Feed"),
                    &e.to_string(),
                    None,
                ),
                None => {}
            }
        });
    }
}
