//! SSH Sessions — bar pill showing the live SSH-connection count.
//!
//! A terminal glyph; when one or more `ssh` clients are running the
//! icon tints `--primary` (DESIGN.md §3) and the count is shown beside
//! it. Left click opens the host panel; right click forces an
//! immediate re-poll. The active set comes from [`crate::ssh`].

use crate::ssh;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(10);
const STARTUP_DELAY: Duration = Duration::from_millis(300);

pub(crate) struct SshSessionsModel {
    active: usize,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum SshSessionsInput {
    /// Left click — open the host panel.
    Clicked,
    /// Right click — re-poll active sessions now.
    Refresh,
}

#[derive(Debug)]
pub(crate) enum SshSessionsOutput {
    Clicked,
}

pub(crate) struct SshSessionsInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum SshSessionsCommandOutput {
    Count(usize),
}

#[relm4::component(pub)]
impl Component for SshSessionsModel {
    type CommandOutput = SshSessionsCommandOutput;
    type Input = SshSessionsInput;
    type Output = SshSessionsOutput;
    type Init = SshSessionsInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            #[watch]
            set_css_classes: if model.active > 0 {
                &["ssh-sessions-bar-widget", "active"]
            } else {
                &["ssh-sessions-bar-widget"]
            },
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(model.active)),

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(SshSessionsInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        set_icon_name: Some("utilities-terminal-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "ssh-sessions-bar-label",
                        #[watch]
                        set_label: &model.active.to_string(),
                        #[watch]
                        set_visible: model.active > 0,
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut first = true;
            loop {
                let delay = if first { STARTUP_DELAY } else { POLL };
                first = false;
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                let n = ssh::active_targets().await.len();
                let _ = out.send(SshSessionsCommandOutput::Count(n));
            }
        });

        let model = SshSessionsModel {
            active: 0,
            _orientation: params.orientation,
        };
        let widgets = view_output!();

        // Right-click → immediate re-poll.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let refresh_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            refresh_sender.input(SshSessionsInput::Refresh);
        });
        widgets.root.add_controller(gesture);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            SshSessionsInput::Clicked => {
                let _ = sender.output(SshSessionsOutput::Clicked);
            }
            SshSessionsInput::Refresh => {
                sender.command(|out, _shutdown| async move {
                    let n = ssh::active_targets().await.len();
                    let _ = out.send(SshSessionsCommandOutput::Count(n));
                });
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SshSessionsCommandOutput::Count(n) => {
                self.active = n;
            }
        }
    }
}

fn tooltip(active: usize) -> String {
    let head = match active {
        0 => "SSH: no active sessions".to_string(),
        1 => "SSH: 1 active session".to_string(),
        n => format!("SSH: {n} active sessions"),
    };
    format!("{head}\n\nClick: hosts  ·  Right-click: refresh")
}
