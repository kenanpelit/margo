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
    show_close_button: bool,
    show_action_buttons: bool,
    group_notifications: bool,
    popup_width: i32,
    blocklist: Vec<String>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum NotificationSettingsInput {
    PositionChanged(NotificationPosition),
    PositionEffect(NotificationPosition),
    ShowCloseChanged(bool),
    ShowCloseEffect(bool),
    ShowActionsChanged(bool),
    ShowActionsEffect(bool),
    GroupChanged(bool),
    GroupEffect(bool),
    PopupWidthChanged(i32),
    PopupWidthEffect(i32),
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
                    set_label: "Toast content",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Close button",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Show the small ✕ button on each notification (swipe also dismisses).",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "show_close_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(close_handler)]
                        set_active: model.show_close_button,
                        connect_active_notify[sender] => move |s| {
                            sender.input(NotificationSettingsInput::ShowCloseChanged(s.is_active()));
                        } @close_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Action buttons",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Show app-provided buttons (View / Open / Reply / …). Off keeps toasts clean.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "show_actions_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(actions_handler)]
                        set_active: model.show_action_buttons,
                        connect_active_notify[sender] => move |s| {
                            sender.input(NotificationSettingsInput::ShowActionsChanged(s.is_active()));
                        } @actions_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Popup width",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Width (px) of the corner popup toasts. Separate from the history menu width in Widgets → Notifications.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "popup_width_spin"]
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (200.0, 1200.0),
                        set_increments: (10.0, 50.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(popup_width_handler)]
                        set_value: model.popup_width as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(NotificationSettingsInput::PopupWidthChanged(s.value() as i32));
                        } @popup_width_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Group by app",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Collapse 2+ notifications from the same app into an expandable group in the history. Off = flat list.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    #[name = "group_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(group_handler)]
                        set_active: model.group_notifications,
                        connect_active_notify[sender] => move |s| {
                            sender.input(NotificationSettingsInput::GroupChanged(s.is_active()));
                        } @group_handler,
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

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .notifications()
                .show_close_button()
                .get();
            sender_clone.input(NotificationSettingsInput::ShowCloseEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .notifications()
                .show_action_buttons()
                .get();
            sender_clone.input(NotificationSettingsInput::ShowActionsEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .notifications()
                .group_notifications()
                .get();
            sender_clone.input(NotificationSettingsInput::GroupEffect(v));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .notifications()
                .popup_width()
                .get();
            sender_clone.input(NotificationSettingsInput::PopupWidthEffect(v));
        });

        let model = NotificationSettingsModel {
            position: config_manager()
                .config()
                .notifications()
                .notification_position()
                .get_untracked(),
            show_close_button: config_manager()
                .config()
                .notifications()
                .show_close_button()
                .get_untracked(),
            show_action_buttons: config_manager()
                .config()
                .notifications()
                .show_action_buttons()
                .get_untracked(),
            group_notifications: config_manager()
                .config()
                .notifications()
                .group_notifications()
                .get_untracked(),
            popup_width: config_manager()
                .config()
                .notifications()
                .popup_width()
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
            let submit = submit;
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
            NotificationSettingsInput::ShowCloseChanged(v) => {
                self.show_close_button = v;
                config_manager().update_config(move |config| {
                    config.notifications.show_close_button = v;
                });
            }
            NotificationSettingsInput::ShowCloseEffect(v) => {
                self.show_close_button = v;
            }
            NotificationSettingsInput::ShowActionsChanged(v) => {
                self.show_action_buttons = v;
                config_manager().update_config(move |config| {
                    config.notifications.show_action_buttons = v;
                });
            }
            NotificationSettingsInput::ShowActionsEffect(v) => {
                self.show_action_buttons = v;
            }
            NotificationSettingsInput::GroupChanged(v) => {
                self.group_notifications = v;
                config_manager().update_config(move |config| {
                    config.notifications.group_notifications = v;
                });
            }
            NotificationSettingsInput::GroupEffect(v) => {
                self.group_notifications = v;
            }
            NotificationSettingsInput::PopupWidthChanged(w) => {
                self.popup_width = w;
                config_manager().update_config(move |config| {
                    config.notifications.popup_width = w;
                });
            }
            NotificationSettingsInput::PopupWidthEffect(w) => {
                self.popup_width = w;
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
