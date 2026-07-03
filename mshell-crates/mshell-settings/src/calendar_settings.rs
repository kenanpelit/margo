//! Settings → Calendar. Configures the sources feeding the clock-menu agenda
//! and the dashboard calendar grid (both consumed by the `mcal` core):
//!
//! 1. **Local folder** — a directory of `.ics` files / sub-directories.
//!    Blank resolves to `~/.config/margo/calendars`.
//! 2. **Refresh interval** — how often remote subscriptions are re-fetched.
//! 3. **Subscriptions** — remote iCal (`.ics`) URLs. Read-only, no OAuth:
//!    paste a public or "secret address in iCal format" URL. Each has an
//!    optional `#RRGGBB` colour.
//!
//! Everything round-trips through `config.calendars` — the same list the
//! calendar widgets read via `calendar_data::shell_calendar_config`.

use crate::row::Row;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    CalendarSubscription, CalendarsStoreFields, ConfigStoreFields,
};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) enum CalendarSettingsInput {
    SetLocalDir(String),
    SetRefreshMins(u64),
    AddSubscription,
    RemoveSubscription(usize),
    SetSubName(usize, String),
    SetSubUrl(usize, String),
    SetSubColor(usize, String),
}

#[derive(Debug)]
pub(crate) enum CalendarSettingsOutput {}
#[derive(Debug)]
pub(crate) enum CalendarSettingsCommandOutput {}
pub(crate) struct CalendarSettingsInit {}

pub(crate) struct CalendarSettingsModel {
    subs: Vec<CalendarSubscription>,
    subs_box: gtk::Box,
}

/// Snapshot `config.calendars.subscriptions`.
fn read_subscriptions() -> Vec<CalendarSubscription> {
    config_manager()
        .config()
        .calendars()
        .subscriptions()
        .get_untracked()
}

/// Apply `mutate` to the subscription at `idx`, then persist.
fn update_sub(idx: usize, mutate: impl FnOnce(&mut CalendarSubscription)) {
    config_manager().update_config(|config| {
        if let Some(sub) = config.calendars.subscriptions.get_mut(idx) {
            mutate(sub);
        }
    });
}

/// Repaint the subscription list — one card per entry, edited in place.
fn rebuild_subs(
    subs_box: &gtk::Box,
    subs: &[CalendarSubscription],
    sender: &ComponentSender<CalendarSettingsModel>,
) {
    while let Some(child) = subs_box.first_child() {
        subs_box.remove(&child);
    }
    if subs.is_empty() {
        let empty = gtk::Label::builder()
            .label("No subscriptions yet. Add one below.")
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .wrap(true)
            .build();
        empty.add_css_class("label-small");
        subs_box.append(&empty);
        return;
    }

    for (i, sub) in subs.iter().enumerate() {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.add_css_class("launcher-script-row");

        // ── Line 1: name · colour · remove ──
        let line1 = gtk::Box::new(gtk::Orientation::Horizontal, 8);

        let name = gtk::Entry::new();
        name.set_hexpand(true);
        name.set_valign(gtk::Align::Center);
        name.set_placeholder_text(Some("name, e.g. Work"));
        name.set_text(&sub.name);
        {
            let s = sender.clone();
            name.connect_changed(move |e| {
                s.input(CalendarSettingsInput::SetSubName(i, e.text().to_string()))
            });
        }
        line1.append(&name);

        let color = gtk::Entry::new();
        color.set_valign(gtk::Align::Center);
        color.set_width_request(120);
        color.set_max_length(7);
        color.set_placeholder_text(Some("#RRGGBB"));
        color.set_tooltip_text(Some(
            "Event colour (optional); blank uses the theme default",
        ));
        color.set_text(&sub.color);
        {
            let s = sender.clone();
            color.connect_changed(move |e| {
                s.input(CalendarSettingsInput::SetSubColor(i, e.text().to_string()))
            });
        }
        line1.append(&color);

        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class("flat");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some("Remove"));
        {
            let s = sender.clone();
            remove.connect_clicked(move |_| s.input(CalendarSettingsInput::RemoveSubscription(i)));
        }
        line1.append(&remove);

        card.append(&line1);

        // ── Line 2: the .ics URL ──
        let url = gtk::Entry::new();
        url.set_hexpand(true);
        url.set_valign(gtk::Align::Center);
        url.set_placeholder_text(Some("https://…/basic.ics"));
        url.set_text(&sub.url);
        {
            let s = sender.clone();
            url.connect_changed(move |e| {
                s.input(CalendarSettingsInput::SetSubUrl(i, e.text().to_string()))
            });
        }
        card.append(&url);

        subs_box.append(&card);
    }
}

#[relm4::component(pub)]
impl Component for CalendarSettingsModel {
    type CommandOutput = CalendarSettingsCommandOutput;
    type Input = CalendarSettingsInput;
    type Output = CalendarSettingsOutput;
    type Init = CalendarSettingsInit;

    view! {
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("x-office-calendar-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Calendar", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Sources for the clock-menu agenda and dashboard calendar. Add a local folder of .ics files and remote iCal subscriptions. Changes apply on the next refresh.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ════════ Local calendars ════════
                gtk::Label { add_css_class: "label-large-bold", set_label: "Local calendars", set_halign: gtk::Align::Start },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_label: "A folder of .ics files. Each file is one calendar; a sub-directory groups several files into one. Blank uses ~/.config/margo/calendars.",
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Folder" },
                        #[local_ref]
                        local_entry -> gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            set_placeholder_text: Some("~/.config/margo/calendars"),
                            connect_changed[sender] => move |e| sender.input(CalendarSettingsInput::SetLocalDir(e.text().to_string())),
                        },
                    },
                    #[template] Row {
                        #[template_child] title { set_label: "Refresh every" },
                        #[local_ref]
                        refresh_spin -> gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_digits: 0,
                            set_tooltip_text: Some("Minutes between background re-fetches of remote subscriptions"),
                            connect_value_changed[sender] => move |sp| sender.input(CalendarSettingsInput::SetRefreshMins(sp.value().max(1.0) as u64)),
                        },
                    },
                },

                // ════════ Subscriptions ════════
                gtk::Label { add_css_class: "label-large-bold", set_label: "Subscriptions", set_halign: gtk::Align::Start, set_margin_top: 8 },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_label: "Remote iCal (.ics) URLs — read-only, no sign-in. Paste a public or \"secret address in iCal format\" link (Google, Outlook, any CalDAV server that exposes one). The colour is optional.",
                },

                #[local_ref]
                subs_box -> gtk::Box {
                    add_css_class: "settings-boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                },

                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "ok-button-primary",
                    set_label: "Add subscription",
                    connect_clicked[sender] => move |_| sender.input(CalendarSettingsInput::AddSubscription),
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cm = config_manager();
        let local_dir = cm.config().calendars().local_dir().get_untracked();
        let refresh_secs = cm.config().calendars().refresh_secs().get_untracked();

        let local_entry = gtk::Entry::new();
        local_entry.set_text(&local_dir);

        let refresh_spin = gtk::SpinButton::with_range(1.0, 1440.0, 1.0);
        refresh_spin.set_value((refresh_secs / 60).max(1) as f64);

        let model = CalendarSettingsModel {
            subs: read_subscriptions(),
            subs_box: gtk::Box::new(gtk::Orientation::Vertical, 6),
        };
        let subs_box = model.subs_box.clone();
        let widgets = view_output!();

        rebuild_subs(&model.subs_box, &model.subs, &sender);
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            CalendarSettingsInput::SetLocalDir(dir) => {
                config_manager().update_config(|config| {
                    config.calendars.local_dir = dir.trim().to_string();
                });
            }
            CalendarSettingsInput::SetRefreshMins(mins) => {
                let secs = mins.max(1).saturating_mul(60);
                config_manager().update_config(|config| {
                    config.calendars.refresh_secs = secs;
                });
            }
            CalendarSettingsInput::AddSubscription => {
                config_manager().update_config(|config| {
                    config
                        .calendars
                        .subscriptions
                        .push(CalendarSubscription::default());
                });
                self.subs = read_subscriptions();
                rebuild_subs(&self.subs_box, &self.subs, &sender);
            }
            CalendarSettingsInput::RemoveSubscription(idx) => {
                config_manager().update_config(|config| {
                    if idx < config.calendars.subscriptions.len() {
                        config.calendars.subscriptions.remove(idx);
                    }
                });
                self.subs = read_subscriptions();
                rebuild_subs(&self.subs_box, &self.subs, &sender);
            }
            CalendarSettingsInput::SetSubName(idx, name) => {
                update_sub(idx, |s| s.name = name);
            }
            CalendarSettingsInput::SetSubUrl(idx, url) => {
                update_sub(idx, |s| s.url = url.trim().to_string());
            }
            CalendarSettingsInput::SetSubColor(idx, color) => {
                update_sub(idx, |s| s.color = color.trim().to_string());
            }
        }
    }
}
