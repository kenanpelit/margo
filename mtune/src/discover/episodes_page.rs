// SPDX-License-Identifier: GPL-3.0-or-later

//! One show's episodes: played state, resume points, play / queue, and
//! subscribe / unsubscribe.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
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
    i18n::{i18n, i18n_k},
    net,
    podcasts::{
        Episode, Podcast, format_length,
        rss::{self, Feed},
    },
};

/// Episodes rendered per "Show More".
const BATCH: usize = 100;

thread_local! {
    /// Feeds read this session, so going back and forth does not refetch.
    static FEEDS: RefCell<HashMap<String, Rc<Feed>>> = RefCell::new(HashMap::new());
}

pub fn remember_feed(url: &str, feed: Feed) {
    FEEDS.with(|f| f.borrow_mut().insert(url.to_string(), Rc::new(feed)));
}

struct EpisodeRow {
    key: String,
    episode: Episode,
    row: adw::ActionRow,
    played: gtk::ToggleButton,
}

struct Page {
    ctx: Rc<Ctx>,
    podcast: RefCell<Podcast>,
    title: gtk::Label,
    /// "12 unplayed of 240".
    count: gtk::Label,
    subscribe: gtk::ToggleButton,
    list: gtk::ListBox,
    more: gtk::Button,
    states: States,
    feed: RefCell<Option<Rc<Feed>>>,
    rows: RefCell<Vec<EpisodeRow>>,
    quiet: Cell<bool>,
}

/// Push the episode page for `podcast` onto `nav`.
pub fn open(ctx: &Rc<Ctx>, nav: &adw::NavigationView, podcast: Podcast) {
    let back = rows::icon_button("go-previous-symbolic", &i18n("Back"));
    let n = nav.clone();
    back.connect_clicked(move |_| {
        n.pop();
    });
    let title = gtk::Label::builder()
        .label(&podcast.title)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["heading"])
        .build();
    let count = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["caption", "dim-label"])
        .build();
    let titles = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    titles.append(&title);
    titles.append(&count);
    let subscribe = gtk::ToggleButton::builder()
        .valign(gtk::Align::Center)
        .build();
    let refresh = rows::icon_button("view-refresh-symbolic", &i18n("Reload Episodes"));
    let bar = gtk::Box::builder()
        .spacing(6)
        .margin_top(6)
        .margin_start(6)
        .margin_end(12)
        .build();
    bar.append(&back);
    bar.append(&titles);
    bar.append(&subscribe);
    bar.append(&refresh);

    let list = rows::boxed_list();
    let more = gtk::Button::builder()
        .label(i18n("Show More"))
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

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    root.append(&adw::Clamp::builder().maximum_size(720).child(&bar).build());
    root.append(&states.stack);

    let page = Rc::new(Page {
        ctx: ctx.clone(),
        podcast: RefCell::new(podcast.clone()),
        title,
        count,
        subscribe,
        list,
        more,
        states,
        feed: RefCell::new(None),
        rows: RefCell::new(Vec::new()),
        quiet: Cell::new(false),
    });
    page.sync_subscribe();
    page.wire(&refresh);

    nav.push(
        &adw::NavigationPage::builder()
            .title(&podcast.title)
            .child(&root)
            .build(),
    );

    let cached = FEEDS.with(|f| f.borrow().get(&podcast.feed_url).cloned());
    match cached {
        Some(feed) => page.show(feed),
        None => page.load(),
    }
    // Handlers hold the page weakly; it lives until the page is popped and
    // its widgets go.
    root.connect_destroy(move |_| {
        let _ = &page;
    });
}

impl Page {
    fn wire(self: &Rc<Self>, refresh: &gtk::Button) {
        let p = Rc::downgrade(self);
        self.subscribe.connect_toggled(move |b| {
            if let Some(p) = p.upgrade()
                && !p.quiet.get()
            {
                p.toggle_subscription(b.is_active());
            }
        });
        let p = Rc::downgrade(self);
        refresh.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.load();
            }
        });
        let p = Rc::downgrade(self);
        self.more.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.render_more();
            }
        });
        let p = Rc::downgrade(self);
        self.states.on_retry(move || {
            if let Some(p) = p.upgrade() {
                p.load();
            }
        });
        let p = Rc::downgrade(self);
        self.ctx.listen(move |change| {
            let Some(p) = p.upgrade() else { return };
            match change {
                Change::Progress => p.sync_rows(),
                Change::Subscriptions => p.sync_subscribe(),
                Change::Stations => {}
            }
        });
    }

    fn feed_url(&self) -> String {
        self.podcast.borrow().feed_url.clone()
    }

    fn sync_subscribe(&self) {
        let on = shared::podcasts()
            .data
            .borrow()
            .is_subscribed(&self.feed_url());
        self.quiet.set(true);
        self.subscribe.set_active(on);
        self.quiet.set(false);
        let label = if on {
            i18n("Subscribed")
        } else {
            i18n("Subscribe")
        };
        self.subscribe.set_label(&label);
        if on {
            self.subscribe.remove_css_class("suggested-action");
        } else {
            self.subscribe.add_css_class("suggested-action");
        }
    }

    fn toggle_subscription(self: &Rc<Self>, on: bool) {
        if on {
            shared::podcasts()
                .data
                .borrow_mut()
                .subscribe(self.podcast.borrow().clone());
            shared::podcasts_changed(Change::Subscriptions);
            return;
        }
        // Unsubscribing forgets played state: confirm.
        self.sync_subscribe(); // stays on until confirmed
        let dialog = adw::AlertDialog::builder()
            .heading(i18n("Unsubscribe?"))
            .body(i18n_k(
                // Translators: `{show}` is a podcast title; leave it untranslated.
                "“{show}” and its played episodes will be forgotten.",
                &[("show", &self.podcast.borrow().title)],
            ))
            .default_response("cancel")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[
            ("cancel", &i18n("_Cancel")),
            ("unsubscribe", &i18n("_Unsubscribe")),
        ]);
        dialog.set_response_appearance("unsubscribe", adw::ResponseAppearance::Destructive);
        let feed = self.feed_url();
        dialog.connect_response(Some("unsubscribe"), move |_, _| {
            shared::podcasts().data.borrow_mut().unsubscribe(&feed);
            shared::podcasts_changed(Change::Subscriptions);
        });
        dialog.present(Some(&self.list));
    }

    fn load(self: &Rc<Self>) {
        self.states.loading();
        let url = self.feed_url();
        let p = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let u = url.clone();
            let res = net::off_main(move || rss::fetch(&u)).await;
            let Some(p) = p.upgrade() else { return };
            match res {
                Some(Ok(feed)) => {
                    let feed = Rc::new(feed);
                    FEEDS.with(|f| f.borrow_mut().insert(url, feed.clone()));
                    p.show(feed);
                }
                Some(Err(e)) => p.states.status(
                    "network-error-symbolic",
                    &i18n("Cannot Load Episodes"),
                    &e.to_string(),
                    Some(&i18n("Try Again")),
                ),
                None => {}
            }
        });
    }

    fn show(self: &Rc<Self>, feed: Rc<Feed>) {
        // Keep a subscription's title/author/artwork current.
        {
            let mut p = self.podcast.borrow_mut();
            if !feed.title.is_empty() {
                p.title = feed.title.clone();
            }
            if !feed.author.is_empty() {
                p.author = feed.author.clone();
            }
            if !feed.image.is_empty() {
                p.artwork = feed.image.clone();
            }
        }
        self.title.set_label(&self.podcast.borrow().title);
        let lib = shared::podcasts();
        if lib.data.borrow().is_subscribed(&self.feed_url()) {
            let before = lib.data.borrow().subscriptions.clone();
            lib.data.borrow_mut().update_details(
                &self.feed_url(),
                &feed.title,
                &feed.author,
                &feed.image,
            );
            if lib.data.borrow().subscriptions != before {
                shared::podcasts_changed(Change::Subscriptions);
            }
        }

        rows::clear(&self.list);
        self.rows.borrow_mut().clear();
        *self.feed.borrow_mut() = Some(feed.clone());
        if feed.episodes.is_empty() {
            self.more.set_visible(false);
            self.states.status(
                "application-rss+xml-symbolic",
                &i18n("No Episodes"),
                &i18n("This feed has no audio episodes"),
                None,
            );
            return;
        }
        self.render_more();
        self.states.show_content();
    }

    fn render_more(self: &Rc<Self>) {
        let Some(feed) = self.feed.borrow().clone() else {
            return;
        };
        let start = self.rows.borrow().len();
        let feed_url = self.feed_url();
        for e in feed.episodes.iter().skip(start).take(BATCH) {
            self.add_row(&feed_url, e);
        }
        self.more
            .set_visible(self.rows.borrow().len() < feed.episodes.len());
        self.sync_rows();
    }

    fn add_row(self: &Rc<Self>, feed_url: &str, e: &Episode) {
        let row = adw::ActionRow::builder()
            .title(&e.title)
            .use_markup(false)
            .title_lines(2)
            .subtitle_lines(1)
            .activatable(true)
            .build();
        if !e.summary.is_empty() {
            row.set_tooltip_text(Some(&e.summary));
        }
        let played = gtk::ToggleButton::builder()
            .icon_name("object-select-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build();
        let queue = rows::icon_button("list-add-symbolic", &i18n("Add to Queue"));
        let play_btn = rows::icon_button("media-playback-start-symbolic", &i18n("Play"));
        row.add_suffix(&played);
        row.add_suffix(&queue);
        row.add_suffix(&play_btn);

        let key = e.key(feed_url);
        let k = key.clone();
        let p = Rc::downgrade(self);
        played.connect_toggled(move |b| {
            if let Some(p) = p.upgrade()
                && !p.quiet.get()
            {
                shared::podcasts()
                    .data
                    .borrow_mut()
                    .set_played(&k, b.is_active());
                shared::podcasts_changed(Change::Progress);
            }
        });
        for (button, now) in [(&queue, false), (&play_btn, true)] {
            let p = Rc::downgrade(self);
            let ep = e.clone();
            button.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    play::play_episode(&p.ctx, &p.podcast.borrow(), &ep, now);
                }
            });
        }
        let p = Rc::downgrade(self);
        let ep = e.clone();
        row.connect_activated(move |_| {
            if let Some(p) = p.upgrade() {
                play::play_episode(&p.ctx, &p.podcast.borrow(), &ep, true);
            }
        });
        self.list.append(&row);
        self.rows.borrow_mut().push(EpisodeRow {
            key,
            episode: e.clone(),
            row,
            played,
        });
    }

    /// Played marks, resume hints and the dimmed style, from the library.
    fn sync_rows(&self) {
        let lib = shared::podcasts();
        let lib = lib.data.borrow();
        self.quiet.set(true);
        for r in self.rows.borrow().iter() {
            let prog = lib.progress(&r.key);
            let mut parts = Vec::new();
            if !r.episode.date.is_empty() {
                parts.push(r.episode.date.clone());
            }
            let length = format_length(r.episode.duration);
            if !length.is_empty() {
                parts.push(length);
            }
            if prog.played {
                parts.push(i18n("Played"));
            } else if prog.position > 0 && r.episode.duration > prog.position {
                parts.push(i18n_k(
                    // Translators: `{time}` is a length like "12 min".
                    "{time} left",
                    &[("time", &format_length(r.episode.duration - prog.position))],
                ));
            } else if prog.position > 0 {
                parts.push(i18n("In progress"));
            }
            r.row.set_subtitle(&parts.join(" · "));
            if r.played.is_active() != prog.played {
                r.played.set_active(prog.played);
            }
            let tip = if prog.played {
                i18n("Mark as Unplayed")
            } else {
                i18n("Mark as Played")
            };
            r.played.set_tooltip_text(Some(&tip));
            r.played
                .update_property(&[gtk::accessible::Property::Label(&tip)]);
            if prog.played {
                r.row.add_css_class("played");
            } else {
                r.row.remove_css_class("played");
            }
        }
        self.quiet.set(false);
        if let Some(feed) = self.feed.borrow().as_ref() {
            let url = self.feed_url();
            let keys: Vec<String> = feed.episodes.iter().map(|e| e.key(&url)).collect();
            let unplayed = lib.unplayed(keys.iter().map(String::as_str));
            self.count.set_label(&i18n_k(
                // Translators: `{new}` and `{all}` are episode counts.
                "{new} unplayed of {all}",
                &[
                    ("new", &unplayed.to_string()),
                    ("all", &keys.len().to_string()),
                ],
            ));
        }
    }
}
