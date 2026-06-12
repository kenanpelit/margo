use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use mshell_common::dynamic_box::dynamic_box::{
    DynamicBoxFactory, DynamicBoxInit, DynamicBoxInput, DynamicBoxModel,
};
use mshell_common::dynamic_box::generic_widget_controller::{
    GenericWidgetController, GenericWidgetControllerExtSafe,
};
use mshell_common::notification::{NotificationInit, NotificationInput, NotificationModel};
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
use std::rc::Rc;
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
        // On-demand keyboard: the surface never grabs by itself, but a
        // click into an inline-reply entry can request focus to type.
        root.set_keyboard_mode(KeyboardMode::OnDemand);
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
                // A fresh toast id — the moment a sound (if configured)
                // belongs. Replaces of the same id reuse the widget via
                // `update` below and stay silent.
                maybe_play_sound(item);

                let notification = item.clone();
                let id = notification.id;
                let svc = notification_service();

                // Effective on-screen time = the configured popup
                // duration, capped by any (shorter) app expire_timeout —
                // matches wayle's own timer so the bar stays in sync.
                let show_bar = config_manager()
                    .config()
                    .notifications()
                    .show_timeout_bar()
                    .get_untracked();
                let timeout_ms = if show_bar {
                    let base = svc.popup_duration.get();
                    let effective = match notification.expire_timeout.get() {
                        Some(ttl) if ttl > 0 => base.min(ttl),
                        _ => base,
                    };
                    Some(effective)
                } else {
                    None
                };

                // Hover pauses the real auto-dismiss timer (and the bar).
                let (on_hover_enter, on_hover_leave): (Option<Rc<dyn Fn()>>, Option<Rc<dyn Fn()>>) =
                    if show_bar {
                        let svc_enter = svc.clone();
                        let svc_leave = svc.clone();
                        (
                            Some(Rc::new(move || svc_enter.inhibit_popup(id))),
                            Some(Rc::new(move || svc_leave.release_popup(id))),
                        )
                    } else {
                        (None, None)
                    };

                let notification_controller = NotificationModel::builder()
                    .launch(NotificationInit {
                        notification,
                        timeout_ms,
                        on_hover_enter,
                        on_hover_leave,
                    })
                    .detach();

                Box::new(notification_controller) as Box<dyn GenericWidgetController>
            }),
            // `replaces_id` re-sends arrive as a NEW Arc under the same id
            // (wayle swaps the list entry). Route the fresh snapshot into
            // the existing card so progress bars / body text update live
            // instead of showing the first frame forever.
            update: Some(Box::new(|controller, item| {
                if let Some(c) =
                    (**controller).downcast_ref::<relm4::Controller<NotificationModel>>()
                {
                    c.emit(NotificationInput::Replaced(item.clone()));
                }
            })),
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

/// Play the notification sound for a freshly-shown toast, honouring the
/// whole decision ladder: master toggle → spec `suppress-sound` hint →
/// quiet hours → per-urgency toggles → client `sound-file` hint (when
/// allowed) or the built-in chime. DND never reaches here — wayle drops
/// the popup itself.
fn maybe_play_sound(n: &Notification) {
    let cfg = config_manager().config();
    if !cfg.clone().notifications().sound_enabled().get_untracked() {
        return;
    }
    if n.suppress_sound.get() {
        return;
    }
    if cfg
        .clone()
        .notifications()
        .quiet_hours_enabled()
        .get_untracked()
    {
        let start = cfg
            .clone()
            .notifications()
            .quiet_hours_start()
            .get_untracked();
        let end = cfg
            .clone()
            .notifications()
            .quiet_hours_end()
            .get_untracked();
        if in_quiet_hours(&start, &end) {
            return;
        }
    }
    use wayle_notification::types::Urgency;
    let urgency = n.urgency.get();
    let allowed = match urgency {
        Urgency::Low => cfg.clone().notifications().sound_low().get_untracked(),
        Urgency::Normal => cfg.clone().notifications().sound_normal().get_untracked(),
        Urgency::Critical => cfg.clone().notifications().sound_critical().get_untracked(),
    };
    if !allowed {
        return;
    }
    let from_client = cfg
        .clone()
        .notifications()
        .sound_from_client()
        .get_untracked();
    if from_client && let Some(file) = n.sound_file.get() {
        let file = file.trim().to_string();
        if !file.is_empty() {
            mshell_sounds::play_notification_file(&file);
            return;
        }
    }
    if matches!(urgency, Urgency::Critical) {
        mshell_sounds::play_notification_critical();
    } else {
        mshell_sounds::play_notification();
    }
}

/// Whether the local wall-clock time falls inside the `HH:MM`–`HH:MM`
/// window. An end before the start wraps past midnight (22:00–08:00).
/// Malformed strings disable the window (sounds keep playing).
fn in_quiet_hours(start: &str, end: &str) -> bool {
    fn mins(s: &str) -> Option<i32> {
        let (h, m) = s.split_once(':')?;
        let h: i32 = h.trim().parse().ok()?;
        let m: i32 = m.trim().parse().ok()?;
        if (0..24).contains(&h) && (0..60).contains(&m) {
            Some(h * 60 + m)
        } else {
            None
        }
    }
    let (Some(s), Some(e)) = (mins(start), mins(end)) else {
        return false;
    };
    let Ok(now) = relm4::gtk::glib::DateTime::now_local() else {
        return false;
    };
    let n = now.hour() * 60 + now.minute();
    if s <= e {
        n >= s && n < e
    } else {
        n >= s || n < e
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

#[cfg(test)]
mod quiet_hours_tests {
    use super::in_quiet_hours;

    // `in_quiet_hours` reads the live clock; the parse/shape cases are
    // what's deterministic to test.
    #[test]
    fn malformed_windows_never_mute() {
        assert!(!in_quiet_hours("", ""));
        assert!(!in_quiet_hours("22", "08:00"));
        assert!(!in_quiet_hours("25:00", "08:00"));
        assert!(!in_quiet_hours("22:00", "08:61"));
    }

    #[test]
    fn degenerate_equal_window_is_empty() {
        // start == end → `n >= s && n < e` can never hold, whatever the
        // wall clock says.
        assert!(!in_quiet_hours("10:00", "10:00"));
    }
}
