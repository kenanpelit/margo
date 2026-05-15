//! KeepAwake — bar toggle for the system idle inhibitor.
//!
//! Click flips the inhibitor on / off. The widget reflects the
//! global inhibitor state — so flipping it from `mctl` or another
//! shell process still updates the icon.
//!
//! Backend: `mshell_idle::IdleInhibitor::global()`. The inhibitor
//! holds an FD against `org.freedesktop.login1`'s `Inhibit` call
//! with `what=idle`; while active, the compositor's idle timer
//! never fires (so no auto-lock / suspend / dim).

use futures::StreamExt;
use mshell_idle::inhibitor::IdleInhibitor;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use tracing::warn;

pub(crate) struct KeepAwakeModel {
    active: bool,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum KeepAwakeInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum KeepAwakeOutput {}

pub(crate) struct KeepAwakeInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum KeepAwakeCommandOutput {
    StateChanged(bool),
}

#[relm4::component(pub)]
impl Component for KeepAwakeModel {
    type CommandOutput = KeepAwakeCommandOutput;
    type Input = KeepAwakeInput;
    type Output = KeepAwakeOutput;
    type Init = KeepAwakeInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "keep-awake-bar-widget",
            #[watch]
            set_css_classes: if model.active {
                &["keep-awake-bar-widget", "active"]
            } else {
                &["keep-awake-bar-widget"]
            },
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            gtk::Button {
                #[watch]
                set_css_classes: if model.active {
                    &["ok-button-surface", "ok-bar-widget", "selected"]
                } else {
                    &["ok-button-surface", "ok-bar-widget"]
                },
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(KeepAwakeInput::Clicked);
                },

                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_icon_name: Some(if model.active {
                        "eye-symbolic"
                    } else {
                        "eye-off-symbolic"
                    }),
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Subscribe to inhibitor state changes so toggles from
        // mctl / external clients still update the pill.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut stream = IdleInhibitor::global().watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    Some(state) = stream.next() => {
                        let _ = out.send(KeepAwakeCommandOutput::StateChanged(state));
                    }
                }
            }
        });

        let model = KeepAwakeModel {
            active: IdleInhibitor::global().get(),
            _orientation: params.orientation,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            KeepAwakeInput::Clicked => {
                // Toggle is async (zbus call). Spawn off the main
                // loop; the watch stream will land the resulting
                // state via `StateChanged`.
                relm4::spawn(async move {
                    if let Err(e) = IdleInhibitor::global().toggle().await {
                        warn!(error = %e, "keep_awake: toggle failed");
                    }
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
            KeepAwakeCommandOutput::StateChanged(state) => {
                self.active = state;
            }
        }
    }
}
