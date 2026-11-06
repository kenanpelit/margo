// SPDX-License-Identifier: GPL-3.0-or-later

//! Row and state widgets shared by the dialog's pages (libadwaita boxed
//! lists, like the rest of Tune).

use adw::prelude::*;

use super::art;
use crate::{i18n::i18n, radio::Station};

/// A flat round icon button with a tooltip that doubles as its
/// accessible name.
pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    b.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    b
}

/// The star that adds / removes a station from My Stations.
pub fn star_button(active: bool) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::builder()
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .active(active)
        .build();
    sync_star(&b);
    b.connect_toggled(sync_star);
    b
}

fn sync_star(b: &gtk::ToggleButton) {
    let (icon, tip) = if b.is_active() {
        ("starred-symbolic", i18n("Remove from My Stations"))
    } else {
        ("non-starred-symbolic", i18n("Add to My Stations"))
    };
    b.set_icon_name(icon);
    b.set_tooltip_text(Some(&tip));
    b.update_property(&[gtk::accessible::Property::Label(&tip)]);
}

/// A station row: logo, name, "country · genres · codec", then the
/// caller's suffix widgets and a play button. Activating the row plays.
pub fn station_row(
    st: &Station,
    suffixes: &[gtk::Widget],
    on_play: impl Fn() + 'static,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(&st.name)
        .subtitle(st.subtitle())
        .use_markup(false)
        .subtitle_lines(1)
        .title_lines(1)
        .activatable(true)
        .build();
    row.add_prefix(&art::thumbnail(&st.favicon, "network-wireless-symbolic"));
    for w in suffixes {
        row.add_suffix(w);
    }
    let play = icon_button("media-playback-start-symbolic", &i18n("Play"));
    let on_play = std::rc::Rc::new(on_play);
    let p = on_play.clone();
    play.connect_clicked(move |_| p());
    row.add_suffix(&play);
    row.connect_activated(move |_| on_play());
    row
}

/// A boxed list (Tune's list style).
pub fn boxed_list() -> gtk::ListBox {
    gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .valign(gtk::Align::Start)
        .build()
}

/// Content states every page cycles through.
pub struct States {
    pub stack: gtk::Stack,
    status: adw::StatusPage,
    retry: gtk::Button,
}

impl States {
    /// `content` is shown for [`States::show_content`].
    pub fn new(content: &impl IsA<gtk::Widget>) -> Self {
        let stack = gtk::Stack::builder()
            .vexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let spinner = gtk::Spinner::builder()
            .spinning(true)
            .width_request(32)
            .height_request(32)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        stack.add_named(&spinner, Some("loading"));
        let retry = gtk::Button::builder()
            .halign(gtk::Align::Center)
            .css_classes(["pill"])
            .visible(false)
            .build();
        let status = adw::StatusPage::builder().child(&retry).build();
        stack.add_named(&status, Some("status"));
        stack.add_named(content, Some("content"));
        Self {
            stack,
            status,
            retry,
        }
    }

    pub fn loading(&self) {
        self.stack.set_visible_child_name("loading");
    }

    pub fn show_content(&self) {
        self.stack.set_visible_child_name("content");
    }

    /// An empty / error state with an icon, a title, a hint and an
    /// optional action button (see [`States::on_retry`]).
    pub fn status(&self, icon: &str, title: &str, hint: &str, action: Option<&str>) {
        self.status.set_icon_name(Some(icon));
        self.status.set_title(title);
        self.status.set_description(Some(hint));
        self.retry.set_visible(action.is_some());
        if let Some(label) = action {
            self.retry.set_label(label);
        }
        self.stack.set_visible_child_name("status");
    }

    pub fn on_retry(&self, f: impl Fn() + 'static) {
        self.retry.connect_clicked(move |_| f());
    }
}

/// Remove every row of a list box.
pub fn clear(list: &gtk::ListBox) {
    list.remove_all();
}
