//! Native radio browser. All service work runs on the service Tokio runtime;
//! Relm commands deliver results and the model rejects stale directory responses.
use super::*;
use radio_state::station_current;
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub(crate) enum Action {
    Play(String),
    Preview(String, String),
    Save(String, String),
    Remove(String),
}

pub(super) struct RadioPanel {
    sender: ComponentSender<MtuneMenuWidgetModel>,
    saved: gtk::Box,
    results: gtk::Box,
    status: gtk::Label,
    spinner: gtk::Spinner,
    search_button: gtk::Button,
    add_button: gtk::Button,
    // Do not rebuild focused rows for position ticks or unrelated property changes.
    rendered: RefCell<Option<(Vec<Station>, Vec<Station>, bool, String)>>,
}

fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class(class);
    label
}

fn action_button(icon: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    mshell_common::a11y::name(&button, name);
    button.set_css_classes(&["panel-action-btn"]);
    button
}

impl RadioPanel {
    pub fn new(host: &gtk::Box, sender: &ComponentSender<MtuneMenuWidgetModel>) -> Self {
        host.set_spacing(12);
        let query = gtk::SearchEntry::builder()
            .placeholder_text("Find stations by name or genre…")
            .hexpand(true)
            .css_classes(["menu-search"])
            .build();
        mshell_common::a11y::name(&query, "Search radio directory");
        let country = gtk::Entry::builder()
            .placeholder_text("Country (optional)")
            .hexpand(true)
            .css_classes(["menu-search"])
            .build();
        mshell_common::a11y::name(&country, "Filter radio by country");
        let search_button = gtk::Button::with_label("Search");
        search_button.add_css_class("mtune-menu-action");
        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        search_row.append(&country);
        search_row.append(&search_button);
        host.append(&query);
        host.append(&search_row);
        let s = sender.clone();
        let q = query.clone();
        let c = country.clone();
        search_button.connect_clicked(move |_| {
            s.input(MtuneMenuInput::SearchRadio(
                q.text().to_string(),
                c.text().to_string(),
            ))
        });
        let s = sender.clone();
        let c = country.clone();
        query.connect_activate(move |q| {
            s.input(MtuneMenuInput::SearchRadio(
                q.text().to_string(),
                c.text().to_string(),
            ))
        });
        let s = sender.clone();
        let q = query.clone();
        country.connect_activate(move |c| {
            s.input(MtuneMenuInput::SearchRadio(
                q.text().to_string(),
                c.text().to_string(),
            ))
        });

        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let spinner = gtk::Spinner::new();
        let status = label(
            "Your saved stations, or discover something new.",
            "mtune-menu-status",
        );
        status.set_hexpand(true);
        status_row.append(&spinner);
        status_row.append(&status);
        host.append(&status_row);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.append(&label("Saved stations", "mtune-menu-section-label"));
        let saved = gtk::Box::new(gtk::Orientation::Vertical, 0);
        saved.set_css_classes(&["ok-boxed-list", "compact"]);
        content.append(&saved);
        content.append(&label("Directory results", "mtune-menu-section-label"));
        let results = gtk::Box::new(gtk::Orientation::Vertical, 0);
        results.set_css_classes(&["ok-boxed-list", "compact"]);
        queue::keyboard_rows(&saved);
        queue::keyboard_rows(&results);
        content.append(&results);
        let scroller = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(0)
            .propagate_natural_height(true)
            .child(&content)
            .build();
        list_fit::mark_menu_list(&scroller, 320);
        host.append(&scroller);

        // Inline editor is collapsed by default, not a modal or main-window link.
        let custom = gtk::Expander::new(Some("Add a station URL"));
        let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let name = gtk::Entry::builder()
            .placeholder_text("Station name")
            .hexpand(true)
            .build();
        let url = gtk::Entry::builder()
            .placeholder_text("https://… stream URL")
            .hexpand(true)
            .build();
        mshell_common::a11y::name(&name, "Custom station name");
        mshell_common::a11y::name(&url, "Custom station stream URL");
        let add_button = gtk::Button::with_label("Save station");
        add_button.add_css_class("mtune-menu-action");
        form.append(&name);
        form.append(&url);
        form.append(&add_button);
        let s = sender.clone();
        add_button.connect_clicked(move |_| {
            s.input(MtuneMenuInput::RadioAction(Action::Save(
                name.text().to_string(),
                url.text().to_string(),
            )))
        });
        custom.set_child(Some(&form));
        content.append(&custom);
        Self {
            sender: sender.clone(),
            saved,
            results,
            status,
            spinner,
            search_button,
            add_button,
            rendered: RefCell::new(None),
        }
    }

    pub fn refresh(&self, m: &MtuneMenuWidgetModel) {
        self.spinner.set_spinning(m.radio.loading || m.radio_busy);
        self.spinner.set_visible(m.radio.loading || m.radio_busy);
        self.search_button.set_sensitive(!m.radio.loading);
        self.add_button.set_sensitive(!m.radio_busy);
        let message = if m.radio.loading {
            "Searching the radio directory…"
        } else if m.radio_busy {
            "Updating radio…"
        } else if !m.radio_message.is_empty() {
            &m.radio_message
        } else if !m.radio.error.is_empty() {
            &m.radio.error
        } else if !m.running {
            "Tune is not running. Launch Tune to use radio; your search stays here."
        } else if m.radio.searched && m.radio.results.is_empty() {
            "No stations found. Try another name or country."
        } else {
            "Play to listen · Star to save a station"
        };
        self.status.set_label(message);
        if m.radio.error.is_empty() && m.radio_message.is_empty() {
            self.status.remove_css_class("mtune-stream-error");
        } else {
            self.status.add_css_class("mtune-stream-error");
        }
        let state = (
            m.radio_stations.clone(),
            m.radio.results.clone(),
            m.is_live,
            m.station_name.clone(),
        );
        for container in [&self.saved, &self.results] {
            let mut row = container.first_child();
            while let Some(current) = row {
                current.set_sensitive(!m.radio_busy);
                row = current.next_sibling();
            }
        }
        if self.rendered.borrow().as_ref() == Some(&state) {
            return;
        }
        *self.rendered.borrow_mut() = Some(state);
        for (container, entries, saved) in [
            (&self.saved, &m.radio_stations, true),
            (&self.results, &m.radio.results, false),
        ] {
            while let Some(child) = container.first_child() {
                container.remove(&child);
            }
            if entries.is_empty() {
                container.append(&label(
                    if saved {
                        "No saved stations. Star a result or add a URL below."
                    } else if m.radio.searched {
                        "No directory results."
                    } else {
                        "Search by station, genre, or country."
                    },
                    "mtune-menu-status",
                ));
            }
            for station in entries {
                let (key, name, url, subtitle) = station;
                let current = station_current(m.is_live, &m.station_name, station);
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                row.add_css_class("mtune-radio-row");
                if current {
                    row.add_css_class("current");
                }
                let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
                let title = label(name, "mtune-queue-row-title");
                title.set_wrap(false);
                title.set_ellipsize(pango::EllipsizeMode::End);
                let detail = label(subtitle, "mtune-queue-row-artist");
                detail.set_wrap(false);
                detail.set_ellipsize(pango::EllipsizeMode::End);
                text.append(&title);
                text.append(&detail);
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let icon = gtk::Image::from_icon_name(if current {
                    "object-select-symbolic"
                } else {
                    "media-playback-start-symbolic"
                });
                content.append(&icon);
                content.append(&text);
                text.set_hexpand(true);
                let play = gtk::Button::builder()
                    .child(&content)
                    .hexpand(true)
                    .css_classes(["ok-button-flat"])
                    .build();
                mshell_common::a11y::name(
                    &play,
                    &format!("{} {name}", if current { "Playing" } else { "Play" }),
                );
                let action = if saved {
                    Action::Play(key.clone())
                } else {
                    Action::Preview(name.clone(), url.clone())
                };
                let s = self.sender.clone();
                play.connect_clicked(move |_| s.input(MtuneMenuInput::RadioAction(action.clone())));
                row.append(&play);
                let favorite = m.radio_stations.iter().find(|station| station.2 == *url);
                let button = action_button(
                    if saved {
                        "user-trash-symbolic"
                    } else if favorite.is_some() {
                        "starred-symbolic"
                    } else {
                        "non-starred-symbolic"
                    },
                    if saved {
                        "Remove saved station"
                    } else if favorite.is_some() {
                        "Remove from saved stations"
                    } else {
                        "Save station"
                    },
                );
                let action = if saved {
                    Action::Remove(key.clone())
                } else if let Some(station) = favorite {
                    Action::Remove(station.0.clone())
                } else {
                    Action::Save(name.clone(), url.clone())
                };
                let s = self.sender.clone();
                button
                    .connect_clicked(move |_| s.input(MtuneMenuInput::RadioAction(action.clone())));
                row.append(&button);
                row.set_sensitive(!m.radio_busy);
                container.append(&row);
            }
        }
    }
}

pub(super) fn search(
    m: &mut MtuneMenuWidgetModel,
    query: String,
    country: String,
    sender: &ComponentSender<MtuneMenuWidgetModel>,
) {
    m.radio_message.clear();
    let generation = m.radio.begin();
    let out = sender.command_sender().clone();
    tokio_rt_spawn(async move {
        let result = mtune_service()
            .player
            .search_radio(query.trim(), country.trim())
            .await;
        let _ = out.send(MtuneMenuCmd::SearchResult(generation, result));
    });
}

pub(super) fn act(
    m: &mut MtuneMenuWidgetModel,
    action: Action,
    sender: &ComponentSender<MtuneMenuWidgetModel>,
) {
    if m.radio_busy {
        return;
    }
    m.radio_busy = true;
    m.radio_message.clear();
    let out = sender.command_sender().clone();
    tokio_rt_spawn(async move {
        let p = mtune_service().player.clone();
        let result = match action {
            Action::Play(key) => p.play_radio_station(&key).await,
            Action::Preview(name, url) => p.preview_radio_station(&name, &url).await,
            Action::Save(name, url) => p.add_radio_station(name.trim(), url.trim()).await,
            Action::Remove(key) => p.remove_radio_station(&key).await,
        };
        let _ = out.send(MtuneMenuCmd::RadioFinished(result));
    });
}
