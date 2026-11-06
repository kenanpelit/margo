//! Settings → Notifications → "RSS feeds": the editor for `rss.*` (the
//! feed notifier in `mshell-frame/src/rss`). Same look as the sound-rules
//! list above it: an entry row + `ok-button-primary` Add, then one
//! `notification-mute-row` per feed with a trash button, then the
//! interval / notification options as a boxed list.

use crate::github_settings::row_text;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, RssConfig, RssConfigStoreFields, RssFeed};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, WidgetExt};

/// A URL the notifier will fetch: `http(s)://host…`, no spaces (the
/// service applies the same rule and skips anything else).
pub(crate) fn feed_url_ok(url: &str) -> bool {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|r| {
        !r.is_empty() && !r.starts_with(['/', '?', '#']) && !url.chars().any(char::is_whitespace)
    })
}

/// Add a feed unless its URL is already listed. Returns whether it was
/// added.
pub(crate) fn add_feed(feeds: &mut Vec<RssFeed>, name: &str, url: &str) -> bool {
    let url = url.trim();
    if !feed_url_ok(url) || feeds.iter().any(|f| f.url.trim() == url) {
        return false;
    }
    feeds.push(RssFeed {
        name: name.trim().to_string(),
        url: url.to_string(),
    });
    true
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(classes);
    l.set_halign(gtk::Align::Start);
    l.set_xalign(0.0);
    l
}

/// Build the section. The returned scope keeps the config mirror alive;
/// the owner stores it next to its other effects.
pub(crate) fn build_rss_section() -> (gtk::Box, EffectScope) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    section.set_widget_name("setting:/notifications/rss");
    section.append(&label("RSS feeds", &["label-large-bold"]));
    let hint = label(
        "New items in these RSS / Atom feeds pop a notification; click it (or Open) to read \
         the item in your browser. A feed is checked on the interval below with a conditional \
         request, so an unchanged feed costs almost nothing. Adding a feed only marks what it \
         already has as seen.",
        &["label-small"],
    );
    hint.set_wrap(true);
    hint.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
    section.append(&hint);

    let input = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Name (optional)"));
    mshell_common::a11y::name(&name, "Feed name");
    let url = gtk::Entry::new();
    url.set_hexpand(true);
    url.set_placeholder_text(Some("https://archlinux.org/feeds/news/"));
    mshell_common::a11y::name(&url, "Feed URL");
    let add = gtk::Button::with_label("Add");
    add.add_css_class("ok-button-primary");
    add.set_valign(gtk::Align::Center);
    input.append(&name);
    input.append(&url);
    input.append(&add);
    section.append(&input);

    let refused = label(
        "Use an http:// or https:// feed URL that isn't listed yet.",
        &["label-small", "sound-rule-missing"],
    );
    refused.set_visible(false);
    section.append(&refused);
    {
        let refused = refused.clone();
        url.connect_changed(move |e| {
            e.remove_css_class("error");
            refused.set_visible(false);
        });
    }
    let submit = {
        let (name, url, refused) = (name.clone(), url.clone(), refused.clone());
        move || {
            let (n, u) = (name.text().to_string(), url.text().to_string());
            if u.trim().is_empty() {
                return;
            }
            let mut feeds = config_manager().config().rss().feeds().get_untracked();
            if !add_feed(&mut feeds, &n, &u) {
                url.add_css_class("error");
                refused.set_visible(true);
                return;
            }
            config_manager().update_config(move |c| c.rss.feeds = feeds);
            name.set_text("");
            url.set_text("");
        }
    };
    {
        let submit = submit.clone();
        add.connect_clicked(move |_| submit());
    }
    {
        let submit = submit.clone();
        name.connect_activate(move |_| submit());
    }
    url.connect_activate(move |_| submit());

    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    section.append(&list);

    let options = gtk::Box::new(gtk::Orientation::Vertical, 0);
    options.add_css_class("boxed-list");
    let notify = gtk::Switch::new();
    notify.set_valign(gtk::Align::Center);
    options.append(&option_row(
        "Notify about new items",
        "Off = feeds are still tracked, nothing pops up.",
        &notify,
    ));
    let interval = gtk::SpinButton::with_range(5.0, 1440.0, 5.0);
    interval.set_valign(gtk::Align::Center);
    options.append(&option_row(
        "Check every (minutes)",
        "How often each feed is fetched (5 – 1440).",
        &interval,
    ));
    let max = gtk::SpinButton::with_range(1.0, 50.0, 1.0);
    max.set_valign(gtk::Align::Center);
    options.append(&option_row(
        "Notifications per check",
        "More new items than this are summed up in one notification.",
        &max,
    ));
    section.append(&options);

    notify.connect_state_set(|_, v| {
        if config_manager().config().rss().notify().get_untracked() != v {
            config_manager().update_config(move |c| c.rss.notify = v);
        }
        glib::Propagation::Proceed
    });
    interval.connect_value_changed(|s| {
        let v = s.value() as u32;
        if config_manager()
            .config()
            .rss()
            .interval_minutes()
            .get_untracked()
            != v
        {
            config_manager().update_config(move |c| c.rss.interval_minutes = v);
        }
    });
    max.connect_value_changed(|s| {
        let v = s.value() as u32;
        if config_manager()
            .config()
            .rss()
            .max_per_poll()
            .get_untracked()
            != v
        {
            config_manager().update_config(move |c| c.rss.max_per_poll = v);
        }
    });

    // Everything follows the config (our own edits and hand edits alike).
    let sync = move |cfg: &RssConfig| {
        rebuild_rows(&list, &cfg.feeds);
        if notify.is_active() != cfg.notify {
            notify.set_active(cfg.notify);
        }
        if interval.value() as u32 != cfg.interval_minutes {
            interval.set_value(f64::from(cfg.interval_minutes));
        }
        if max.value() as u32 != cfg.max_per_poll {
            max.set_value(f64::from(cfg.max_per_poll));
        }
    };
    sync(&config_manager().config().rss().get_untracked());
    let mut effects = EffectScope::new();
    effects.push(move |_| sync(&config_manager().config().rss().get()));
    (section, effects)
}

fn option_row(title: &str, help: &str, control: &impl gtk::prelude::IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    row.add_css_class("action-row");
    row.append(&row_text(title, help));
    row.append(control);
    row
}

fn rebuild_rows(list: &gtk::Box, feeds: &[RssFeed]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for (idx, feed) in feeds.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("notification-mute-row");
        let icon = gtk::Image::from_icon_name("application-rss+xml-symbolic");
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        let title = if feed.name.trim().is_empty() {
            "Feed title"
        } else {
            feed.name.trim()
        };
        text.append(&label(title, &["label-medium"]));
        let detail = label(&feed.url, &["label-small"]);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        text.append(&detail);
        row.append(&text);

        if !feed_url_ok(&feed.url) {
            let badge = label("skipped", &["label-small", "sound-rule-missing"]);
            badge.set_valign(gtk::Align::Center);
            badge.set_tooltip_text(Some("Not an http(s) URL — this feed is not checked."));
            row.append(&badge);
        }

        let remove = gtk::Button::new();
        remove.add_css_class("ok-button-surface");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some("Remove"));
        remove.set_child(Some(&gtk::Image::from_icon_name("user-trash-symbolic")));
        mshell_common::a11y::name(&remove, &format!("Remove feed {title}"));
        remove.connect_clicked(move |_| {
            config_manager().update_config(move |c| {
                if idx < c.rss.feeds.len() {
                    c.rss.feeds.remove(idx);
                }
            });
        });
        row.append(&remove);
        list.append(&row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_urls_must_be_web_and_unique() {
        assert!(feed_url_ok(" https://archlinux.org/feeds/news/ "));
        assert!(feed_url_ok("http://h.test:8080/rss"));
        assert!(!feed_url_ok("ftp://h.test/rss"));
        assert!(!feed_url_ok("https:///rss"));
        assert!(!feed_url_ok("https://h.test/a b"));
        assert!(!feed_url_ok("archlinux.org/feeds/news/"));
        let mut feeds = Vec::new();
        assert!(add_feed(
            &mut feeds,
            " Arch ",
            " https://archlinux.org/feeds/news/ "
        ));
        assert_eq!(feeds[0].name, "Arch");
        assert_eq!(feeds[0].url, "https://archlinux.org/feeds/news/");
        assert!(!add_feed(
            &mut feeds,
            "Again",
            "https://archlinux.org/feeds/news/"
        ));
        assert!(!add_feed(&mut feeds, "", "file:///etc/passwd"));
        assert_eq!(feeds.len(), 1);
    }
}
