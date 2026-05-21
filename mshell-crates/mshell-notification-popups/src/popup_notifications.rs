use gtk4_layer_shell::{Edge, Layer, LayerShell};
use mshell_common::dynamic_box::dynamic_box::{
    DynamicBoxFactory, DynamicBoxInit, DynamicBoxInput, DynamicBoxModel,
};
use mshell_common::dynamic_box::generic_widget_controller::GenericWidgetController;
use mshell_common::notification::{NotificationInit, NotificationModel};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, NotificationsStoreFields};
use mshell_config::schema::position::NotificationPosition;
use mshell_services::notification_service;
use mshell_utils::notifications::spawn_notification_popups_watcher;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{GtkWindowExt, OrientableExt, WidgetExt};
use relm4::gtk::{RevealerTransitionType, gdk};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::sync::Arc;
use tracing::debug;
use wayle_notification::core::notification::Notification;

pub struct PopupNotificationsModel {
    dynamic_box_controller: Controller<DynamicBoxModel<Arc<Notification>, u32>>,
    popup_width: i32,
    _effects: EffectScope,
}

#[derive(Debug)]
pub enum PopupNotificationsInput {
    PositionChanged(NotificationPosition),
    WidthChanged(i32),
}

#[derive(Debug)]
pub enum PopupNotificationsOutput {}

pub struct PopupNotificationsInit {
    pub monitor: gdk::Monitor,
}

#[derive(Debug)]
pub enum PopupNotificationsCommandOutput {
    NotificationsChanged,
}

#[relm4::component(pub)]
impl Component for PopupNotificationsModel {
    type CommandOutput = PopupNotificationsCommandOutput;
    type Input = PopupNotificationsInput;
    type Output = PopupNotificationsOutput;
    type Init = PopupNotificationsInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Window {
            set_css_classes: &["popup-notifications-window", "window-opacity"],
            set_decorated: false,
            // Start hidden and toggle with the popup count (see
            // NotificationsChanged). An always-visible layer-shell overlay
            // keeps showing its last committed frame after the toasts are
            // removed — leaving a half-collapsed remnant ("stuck View
            // button") on screen. Hiding the surface when empty makes the
            // compositor drop it. Same lifecycle mshell-osd uses.
            set_visible: false,
            set_default_height: 1,

            #[name = "content_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                // Initial width; live changes arrive via WidthChanged.
                set_width_request: model.popup_width,

                model.dynamic_box_controller.widget().clone() {},
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let config = config_manager().config();

        let position = config
            .notifications()
            .notification_position()
            .get_untracked();

        let popup_width = config_manager()
            .config()
            .notifications()
            .popup_width()
            .get_untracked();

        root.init_layer_shell();
        root.set_monitor(Some(&params.monitor));
        root.set_namespace(Some("mshell-notifications"));
        root.set_layer(Layer::Overlay);
        root.set_exclusive_zone(0);
        set_position(position.clone(), &root);

        debug!(
            position = ?position,
            "popup_notifications: layer surface initialized"
        );

        spawn_notification_popups_watcher(&sender, || {
            PopupNotificationsCommandOutput::NotificationsChanged
        });

        let notifications_dynamic_box_factory = DynamicBoxFactory::<Arc<Notification>, u32> {
            id: Box::new(|item| item.id),
            create: Box::new(move |item| {
                let notification = item.clone();
                let notification_controller = NotificationModel::builder()
                    .launch(NotificationInit { notification })
                    .detach();

                Box::new(notification_controller) as Box<dyn GenericWidgetController>
            }),
            update: None,
        };

        let notifications_dynamic_box_controller: Controller<
            DynamicBoxModel<Arc<Notification>, u32>,
        > = DynamicBoxModel::builder()
            .launch(DynamicBoxInit {
                factory: notifications_dynamic_box_factory,
                orientation: gtk::Orientation::Vertical,
                spacing: 10,
                transition_type: RevealerTransitionType::SlideDown,
                transition_duration_ms: 200,
                reverse: false,
                retain_entries: false,
                allow_drag_and_drop: false,
            })
            .detach();

        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let position = config.notifications().notification_position().get();
            sender_clone.input(PopupNotificationsInput::PositionChanged(position))
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let width = config.notifications().popup_width().get();
            sender_clone.input(PopupNotificationsInput::WidthChanged(width))
        });

        let model = PopupNotificationsModel {
            dynamic_box_controller: notifications_dynamic_box_controller,
            popup_width,
            _effects: effects,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            PopupNotificationsInput::PositionChanged(pos) => {
                set_position(pos, root);
            }
            PopupNotificationsInput::WidthChanged(width) => {
                self.popup_width = width;
                widgets.content_box.set_width_request(width);
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            PopupNotificationsCommandOutput::NotificationsChanged => {
                let notifications = notification_service().popups.get();
                debug!(
                    count = notifications.len(),
                    "popup_notifications: NotificationsChanged → SetItems"
                );
                // Show the overlay surface only while there are toasts.
                // Hiding it when the list empties forces the compositor to
                // drop the surface, so a removed toast can't linger as a
                // stale / half-collapsed frame.
                root.set_visible(!notifications.is_empty());
                self.dynamic_box_controller
                    .emit(DynamicBoxInput::SetItems(notifications));
            }
        }

        self.update_view(widgets, sender);
    }
}

fn set_position(position: NotificationPosition, root: &gtk::Window) {
    match position {
        NotificationPosition::Left => {
            root.set_anchor(Edge::Top, true);
            root.set_anchor(Edge::Bottom, false);
            root.set_anchor(Edge::Left, true);
            root.set_anchor(Edge::Right, false);
        }
        NotificationPosition::Right => {
            root.set_anchor(Edge::Top, true);
            root.set_anchor(Edge::Bottom, false);
            root.set_anchor(Edge::Left, false);
            root.set_anchor(Edge::Right, true);
        }
        NotificationPosition::Center => {
            root.set_anchor(Edge::Top, true);
            root.set_anchor(Edge::Bottom, false);
            root.set_anchor(Edge::Left, false);
            root.set_anchor(Edge::Right, false);
        }
    }
}
