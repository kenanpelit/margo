use mshell_common::notification::{NotificationInit, NotificationModel, NotificationOutput};
use mshell_services::notification_service;
use mshell_utils::notifications::{spawn_dnd_watcher, spawn_notifications_watcher};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::collections::HashMap;
use std::sync::Arc;
use wayle_notification::core::notification::Notification;

pub(crate) struct NotificationsModel {
    /// Live notification widget controllers, kept alive while their
    /// widgets are parented in the (re)grouped list. Rebuilt on every
    /// change so per-app grouping stays correct.
    notif_controllers: Vec<Controller<NotificationModel>>,
    empty_label_visible: bool,
    dnd: bool,
}

#[derive(Debug)]
pub(crate) enum NotificationsInput {
    ClearAllClicked,
    DndClicked,
}

#[derive(Debug)]
pub(crate) enum NotificationsOutput {
    CloseMenu,
}

pub(crate) struct NotificationsInit {}

#[derive(Debug)]
pub(crate) enum NotificationsCommandOutput {
    NotificationsChanged,
    DndChanged,
}

#[relm4::component(pub)]
impl Component for NotificationsModel {
    type CommandOutput = NotificationsCommandOutput;
    type Input = NotificationsInput;
    type Output = NotificationsOutput;
    type Init = NotificationsInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "notifications-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input(NotificationsInput::DndClicked);
                    },
                    set_margin_end: 4,

                    gtk::Image {
                        #[watch]
                        set_icon_name: if model.dnd {
                            Some("notification-disabled-symbolic")
                        } else {
                            Some("notification-symbolic")
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_halign: gtk::Align::Start,
                    set_label: "Notification History",
                    set_hexpand: true,
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input(NotificationsInput::ClearAllClicked);
                    },

                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Clear all",
                    },
                },
            },

            gtk::Label {
                add_css_class: "label-medium",
                #[watch]
                set_visible: model.empty_label_visible,
                set_label: "Empty",
            },

            gtk::ScrolledWindow {
                set_vscrollbar_policy: gtk::PolicyType::Automatic,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,
                set_propagate_natural_width: false,

                #[name = "list"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 10,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_notifications_watcher(&sender, || NotificationsCommandOutput::NotificationsChanged);

        spawn_dnd_watcher(&sender, || NotificationsCommandOutput::DndChanged);

        let model = NotificationsModel {
            notif_controllers: Vec::new(),
            empty_label_visible: true,
            dnd: false,
        };

        let widgets = view_output!();

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
            NotificationsInput::ClearAllClicked => {
                tokio::spawn(async move {
                    let _ = notification_service().dismiss_all().await;
                });
                let _ = sender.output(NotificationsOutput::CloseMenu);
            }
            NotificationsInput::DndClicked => {
                let service = notification_service();
                let dnd = service.dnd.get();

                service.set_dnd(!dnd);
            }
        }

        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NotificationsCommandOutput::NotificationsChanged => {
                let notifications = notification_service().notifications.get();
                self.empty_label_visible = notifications.is_empty();
                self.rebuild_list(&widgets.list, &notifications, &sender);
            }
            NotificationsCommandOutput::DndChanged => {
                let service = notification_service();
                self.dnd = service.dnd.get();
            }
        }

        self.update_view(widgets, sender);
    }
}

impl NotificationsModel {
    /// Rebuild the history list, grouping notifications by app name.
    /// A single notification from an app renders directly; two or more
    /// collapse into an expandable group header ("App (N)"). New
    /// controllers replace the old ones so their widgets stay alive.
    fn rebuild_list(
        &mut self,
        list: &gtk::Box,
        notifications: &[Arc<Notification>],
        sender: &ComponentSender<Self>,
    ) {
        // Drop the old widgets, then the old controllers.
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        self.notif_controllers.clear();

        // Group by app name, preserving first-seen order.
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, Vec<Arc<Notification>>> = HashMap::new();
        for n in notifications {
            let app = n.app_name.get().unwrap_or_default();
            if !groups.contains_key(&app) {
                order.push(app.clone());
            }
            groups.entry(app).or_default().push(n.clone());
        }

        for app in order {
            let items = groups.remove(&app).unwrap_or_default();
            let build = |n: &Arc<Notification>, this: &mut Self| -> gtk::Box {
                let controller = NotificationModel::builder()
                    .launch(NotificationInit {
                        notification: n.clone(),
                    })
                    .forward(sender.output_sender(), |msg| match msg {
                        NotificationOutput::ActionActivated => NotificationsOutput::CloseMenu,
                    });
                let widget = controller.widget().clone();
                this.notif_controllers.push(controller);
                widget
            };

            if items.len() == 1 {
                let w = build(&items[0], self);
                list.append(&w);
            } else {
                let inner = gtk::Box::new(gtk::Orientation::Vertical, 10);
                for n in &items {
                    let w = build(n, self);
                    inner.append(&w);
                }
                let expander = gtk::Expander::new(Some(&format!("{app}  ({})", items.len())));
                expander.add_css_class("notification-group");
                expander.set_expanded(false);
                expander.set_child(Some(&inner));
                list.append(&expander);
            }
        }
    }
}
