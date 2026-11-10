use mshell_services::notification_service;
use mshell_utils::notifications::spawn_notifications_watcher;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// Bar bell with a read/unread corner dot.
///
/// `total` is the size of the notification history; `unread` is what
/// arrived since the user last opened the centre — the shared read line
/// in [`crate::notification_unread`], so the bell, mdash's chip and the
/// Overview card always agree.
/// The dot encodes three states:
///   - unread (`total > seen`) → solid accent dot ("new, look at me")
///   - read   (`total > 0`, all seen) → faint muted dot ("history here")
///   - empty  (`total == 0`) → no dot
#[derive(Debug, Clone)]
pub(crate) struct NotificationsModel {
    orientation: Orientation,
    total: usize,
    unread: usize,
}

#[derive(Debug)]
pub(crate) enum NotificationsInput {
    /// The user opened the centre — mark everything currently in
    /// history as read.
    Opened,
}

#[derive(Debug)]
pub(crate) enum NotificationsOutput {
    Clicked,
}

pub(crate) struct NotificationsInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum NotificationsCommandOutput {
    NotificationsChanged,
}

impl NotificationsModel {
    fn unread(&self) -> usize {
        self.unread
    }

    fn refresh(&mut self) {
        self.total = notification_service()
            .map(|s| s.notifications.get().len())
            .unwrap_or(0);
        self.unread = crate::notification_unread::unread();
    }
}

/// CSS classes for the corner dot given the current counts.
fn dot_classes(unread: usize, total: usize) -> &'static [&'static str] {
    if unread > 0 {
        &["notif-dot", "unread"]
    } else if total > 0 {
        &["notif-dot", "read"]
    } else {
        &["notif-dot"]
    }
}

#[relm4::component(pub)]
impl Component for NotificationsModel {
    type Input = NotificationsInput;
    type Output = NotificationsOutput;
    type Init = NotificationsInit;
    type CommandOutput = NotificationsCommandOutput;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "notifications-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(NotificationsInput::Opened);
                    sender.output(NotificationsOutput::Clicked).unwrap_or_default();
                },

                gtk::Overlay {
                    set_hexpand: true,
                    set_vexpand: true,

                    #[name="image"]
                    gtk::Image {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        // Bell carries an extra cue for the unread state
                        // (filled alert glyph); the dot does the precise
                        // read/unread/empty distinction.
                        #[watch]
                        set_icon_name: Some(if model.unread() > 0 {
                            "notification-alert-symbolic"
                        } else {
                            "notification-symbolic"
                        }),
                    },

                    // Corner dot — purely decorative, never a click target
                    // (the whole bell handles the click).
                    add_overlay = &gtk::Box {
                        set_halign: gtk::Align::End,
                        set_valign: gtk::Align::Start,
                        set_can_target: false,
                        #[watch]
                        set_visible: model.total > 0,
                        #[watch]
                        set_css_classes: dot_classes(model.unread(), model.total),
                    },
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_notifications_watcher(&sender, || NotificationsCommandOutput::NotificationsChanged);
        // The read line also moves when the centre is opened elsewhere.
        sender.command(|out, shutdown| {
            shutdown
                .register(async move {
                    let mut rx = crate::notification_unread::changes();
                    while rx.changed().await.is_ok() {
                        if out
                            .send(NotificationsCommandOutput::NotificationsChanged)
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .drop_on_shutdown()
        });

        // The history present at startup counts as read (the shared line's
        // first observation), so a fresh login doesn't light up old ones.
        let mut model = NotificationsModel {
            orientation: params.orientation,
            total: 0,
            unread: 0,
        };
        model.refresh();

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
            NotificationsInput::Opened => {
                // Opening the centre acknowledges everything in history.
                crate::notification_unread::mark_all_read();
                self.refresh();
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
            NotificationsCommandOutput::NotificationsChanged => self.refresh(),
        }

        self.update_view(widgets, sender);
    }
}
