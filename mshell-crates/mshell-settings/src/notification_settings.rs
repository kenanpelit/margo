use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, NotificationsStoreFields};
use mshell_config::schema::position::NotificationPosition;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct NotificationSettingsModel {
    position: NotificationPosition,
    blocklist: Vec<String>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum NotificationSettingsInput {
    PositionChanged(NotificationPosition),
    PositionEffect(NotificationPosition),
    BlocklistAdd(String),
    BlocklistRemove(String),
    BlocklistEffect(Vec<String>),
}

#[derive(Debug)]
pub(crate) enum NotificationSettingsOutput {}

pub(crate) struct NotificationSettingsInit {}

#[derive(Debug)]
pub(crate) enum NotificationSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for NotificationSettingsModel {
    type CommandOutput = NotificationSettingsCommandOutput;
    type Input = NotificationSettingsInput;
    type Output = NotificationSettingsOutput;
    type Init = NotificationSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("dialog-information-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Notifications",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Toast geometry, history retention, urgency bar, do-not-disturb.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Notifications",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Position",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Where popup notifications should be positioned.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 150,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&gtk::StringList::new(&NotificationPosition::display_names())),
                        #[watch]
                        #[block_signal(handler)]
                        set_selected: model.position.to_index(),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(NotificationSettingsInput::PositionChanged(
                                NotificationPosition::from_index(dd.selected())
                            ));
                        } @handler,
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Muted apps",
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_label: "Notifications whose app name contains one of these entries (case-insensitive) are silently dropped. Type an app name and press Enter or Add.",
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,

                    #[name = "blocklist_entry"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("App name (e.g. Spotify)"),
                    },

                    #[name = "blocklist_add"]
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_valign: gtk::Align::Center,
                        set_label: "Add",
                    },
                },

                #[name = "blocklist_list"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.notifications().notification_position().get();
            sender_clone.input(NotificationSettingsInput::PositionEffect(value));
        });

        // Mirror external blocklist edits (e.g. hand-edited YAML) back
        // into the UI.
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let list = config_manager().config().notifications().blocklist().get();
            sender_clone.input(NotificationSettingsInput::BlocklistEffect(list));
        });

        let model = NotificationSettingsModel {
            position: config_manager()
                .config()
                .notifications()
                .notification_position()
                .get_untracked(),
            blocklist: config_manager()
                .config()
                .notifications()
                .blocklist()
                .get_untracked(),
            _effects: effects,
        };

        let widgets = view_output!();

        // Wire the add entry + button, and paint the initial rows.
        let entry = widgets.blocklist_entry.clone();
        let sender_clone = sender.clone();
        let submit = move |entry: &gtk::Entry, sender: &ComponentSender<NotificationSettingsModel>| {
            let name = entry.text().trim().to_string();
            if !name.is_empty() {
                sender.input(NotificationSettingsInput::BlocklistAdd(name));
                entry.set_text("");
            }
        };
        {
            let entry = entry.clone();
            let sender = sender_clone.clone();
            let submit = submit.clone();
            widgets.blocklist_add.connect_clicked(move |_| submit(&entry, &sender));
        }
        {
            let sender = sender_clone.clone();
            widgets
                .blocklist_entry
                .connect_activate(move |e| submit(e, &sender));
        }
        rebuild_blocklist_rows(&widgets.blocklist_list, &model.blocklist, &sender);

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NotificationSettingsInput::PositionChanged(position) => {
                self.position = position.clone();
                config_manager().update_config(|config| {
                    config.notifications.notification_position = position;
                });
            }
            NotificationSettingsInput::PositionEffect(position) => {
                self.position = position;
            }
            NotificationSettingsInput::BlocklistAdd(name) => {
                let exists = self
                    .blocklist
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(&name));
                if !exists {
                    self.blocklist.push(name);
                    let list = self.blocklist.clone();
                    config_manager().update_config(move |config| {
                        config.notifications.blocklist = list;
                    });
                    rebuild_blocklist_rows(&widgets.blocklist_list, &self.blocklist, &sender);
                }
            }
            NotificationSettingsInput::BlocklistRemove(name) => {
                self.blocklist.retain(|e| e != &name);
                let list = self.blocklist.clone();
                config_manager().update_config(move |config| {
                    config.notifications.blocklist = list;
                });
                rebuild_blocklist_rows(&widgets.blocklist_list, &self.blocklist, &sender);
            }
            NotificationSettingsInput::BlocklistEffect(list) => {
                if list != self.blocklist {
                    self.blocklist = list;
                    rebuild_blocklist_rows(&widgets.blocklist_list, &self.blocklist, &sender);
                }
            }
        }

        self.update_view(widgets, sender);
    }
}

/// Repaint the muted-apps list: one row per entry with a remove ✕.
fn rebuild_blocklist_rows(
    list: &gtk::Box,
    items: &[String],
    sender: &ComponentSender<NotificationSettingsModel>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for name in items {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("notification-mute-row");

        let label = gtk::Label::new(Some(name));
        label.add_css_class("label-medium");
        label.set_halign(gtk::Align::Start);
        label.set_hexpand(true);
        label.set_xalign(0.0);
        row.append(&label);

        let remove = gtk::Button::new();
        remove.add_css_class("ok-button-surface");
        remove.set_valign(gtk::Align::Center);
        remove.set_child(Some(&gtk::Image::from_icon_name("user-trash-symbolic")));
        let name = name.clone();
        let sender = sender.clone();
        remove.connect_clicked(move |_| {
            sender.input(NotificationSettingsInput::BlocklistRemove(name.clone()));
        });
        row.append(&remove);

        list.append(&row);
    }
}
