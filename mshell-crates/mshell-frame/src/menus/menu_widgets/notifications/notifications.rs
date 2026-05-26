use mshell_common::notification::{NotificationInit, NotificationModel, NotificationOutput};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, NotificationsStoreFields};
use mshell_services::notification_service;
use mshell_utils::notifications::{spawn_dnd_watcher, spawn_notifications_watcher};
use reactive_graph::traits::{Get, GetUntracked};
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
    /// Whether this menu is currently revealed. Every monitor hosts a
    /// notifications menu, so an un-gated rebuild-on-notification was N
    /// full list rebuilds per incoming toast — and toasts arrive while
    /// the menu is closed. We rebuild once, on the next reveal, if dirty.
    revealed: bool,
    /// A notification (or grouping) change landed while hidden — rebuild
    /// on next reveal.
    dirty: bool,
    /// Re-runs `rebuild_list` when the `group_notifications` toggle
    /// changes, so the Settings switch applies to an open menu live.
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum NotificationsInput {
    ClearAllClicked,
    DndClicked,
    /// The `group_notifications` config toggle flipped — rebuild the list.
    GroupingChanged,
    /// The frame's notifications menu was revealed (`true`) or hidden
    /// (`false`). Rebuilds happen only while revealed; a notification
    /// arriving while hidden just flips `dirty` and the list is rebuilt
    /// once, on the next reveal.
    ParentRevealChanged(bool),
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

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Do Not Disturb"),
                    connect_clicked[sender] => move |_| {
                        sender.input(NotificationsInput::DndClicked);
                    },

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
                    add_css_class: "panel-title",
                    set_halign: gtk::Align::Start,
                    set_label: "Notification History",
                    set_hexpand: true,
                },

                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Clear all"),
                    connect_clicked[sender] => move |_| {
                        sender.input(NotificationsInput::ClearAllClicked);
                    },

                    gtk::Image {
                        set_icon_name: Some("edit-clear-all-symbolic"),
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

        // Re-render the history live when the grouping toggle flips.
        let mut effects = EffectScope::new();
        let eff_sender = sender.clone();
        effects.push(move |_| {
            let _ = config_manager()
                .config()
                .notifications()
                .group_notifications()
                .get();
            eff_sender.input(NotificationsInput::GroupingChanged);
        });

        let model = NotificationsModel {
            notif_controllers: Vec::new(),
            empty_label_visible: true,
            dnd: false,
            revealed: false,
            dirty: false,
            _effects: effects,
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
            NotificationsInput::GroupingChanged => {
                // Only the visible menu rebuilds; a hidden one defers.
                if self.revealed {
                    self.refresh(&widgets.list, &sender);
                } else {
                    self.dirty = true;
                }
            }
            NotificationsInput::ParentRevealChanged(revealed) => {
                self.revealed = revealed;
                if revealed {
                    // Rebuild on open to reflect current history, clearing
                    // any deferral accumulated while hidden.
                    self.dirty = false;
                    self.refresh(&widgets.list, &sender);
                }
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
                // The hot path: a notification arrived/cleared. Rebuild
                // only the visible menu; hidden ones defer to next reveal
                // so a single toast doesn't rebuild every monitor's panel.
                if self.revealed {
                    self.refresh(&widgets.list, &sender);
                } else {
                    self.dirty = true;
                }
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
    /// Pull current history from the service, refresh the empty-state
    /// flag, and rebuild the list. The single rebuild entry point for
    /// the visible menu (notification change, grouping toggle, reveal).
    fn refresh(&mut self, list: &gtk::Box, sender: &ComponentSender<Self>) {
        let notifications = notification_service().notifications.get();
        self.empty_label_visible = notifications.is_empty();
        self.rebuild_list(list, &notifications, sender);
    }

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

        // Grouping off → flat, chronological list, one row per notification.
        let group = config_manager()
            .config()
            .notifications()
            .group_notifications()
            .get_untracked();
        if !group {
            for n in notifications {
                let w = build(n, self);
                list.append(&w);
            }
            return;
        }

        // Group by app name, preserving first-seen order. A single
        // notification renders directly; two or more collapse into an
        // expandable "App (N)" header.
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
