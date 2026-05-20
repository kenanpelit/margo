//! KeepAwake — bar pill for the timed idle inhibitor.
//!
//! Left click opens the Keep Awake panel (duration grid + countdown);
//! right click turns a running session off. The pill reflects the
//! global inhibitor state plus the [`KeepAwakeSession`] deadline, so
//! it shows a live countdown while a timed session runs and stays in
//! sync with `mctl` / external toggles.

use crate::keep_awake::{KeepAwakeSession, format_remaining};
use futures::StreamExt;
use mshell_idle::inhibitor::IdleInhibitor;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

pub(crate) struct KeepAwakeModel {
    active: bool,
    remaining: Option<Duration>,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum KeepAwakeInput {
    /// Left click — open the panel.
    Clicked,
    /// Right click — turn a running session off.
    TurnOff,
}

#[derive(Debug)]
pub(crate) enum KeepAwakeOutput {
    Clicked,
}

pub(crate) struct KeepAwakeInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum KeepAwakeCommandOutput {
    /// Inhibitor active flag + session remaining time.
    Refresh(bool, Option<Duration>),
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
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(model.active, model.remaining)),

            gtk::Button {
                // Always the plain bar-pill surface — active state is a
                // primary icon tint (DESIGN.md §3), not the filled
                // `selected` capsule (which forces on-primary text that
                // vanishes against the transparent bar background).
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(KeepAwakeInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        #[watch]
                        set_icon_name: Some(if model.active {
                            "eye-symbolic"
                        } else {
                            "eye-off-symbolic"
                        }),
                    },
                    gtk::Label {
                        add_css_class: "keep-awake-bar-label",
                        #[watch]
                        set_label: &countdown_text(model.active, model.remaining),
                        #[watch]
                        set_visible: !countdown_text(model.active, model.remaining).is_empty(),
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
        // Combined refresh loop: inhibitor flips, session deadline
        // changes, and a 1 s heartbeat for the countdown.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut inhib = IdleInhibitor::global().watch();
            let mut sess = KeepAwakeSession::global().watch();
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    Some(_) = inhib.next() => {}
                    Ok(_) = sess.changed() => {}
                    _ = tick.tick() => {}
                }
                let active = IdleInhibitor::global().get();
                let remaining = KeepAwakeSession::global().remaining();
                let _ = out.send(KeepAwakeCommandOutput::Refresh(active, remaining));
            }
        });

        let model = KeepAwakeModel {
            active: IdleInhibitor::global().get(),
            remaining: KeepAwakeSession::global().remaining(),
            _orientation: params.orientation,
        };

        let widgets = view_output!();

        // Right-click → quick turn-off (without opening the panel).
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let off_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            off_sender.input(KeepAwakeInput::TurnOff);
        });
        widgets.root.add_controller(gesture);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            KeepAwakeInput::Clicked => {
                let _ = sender.output(KeepAwakeOutput::Clicked);
            }
            KeepAwakeInput::TurnOff => {
                KeepAwakeSession::global().deactivate();
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
            KeepAwakeCommandOutput::Refresh(active, remaining) => {
                self.active = active;
                self.remaining = remaining;
            }
        }
    }
}

fn countdown_text(active: bool, remaining: Option<Duration>) -> String {
    match (active, remaining) {
        (true, Some(d)) => format_remaining(d),
        _ => String::new(),
    }
}

fn tooltip(active: bool, remaining: Option<Duration>) -> String {
    let footer = "\n\nClick: durations  ·  Right-click: off";
    let head = match (active, remaining) {
        (false, _) => "Keep Awake: off".to_string(),
        (true, Some(d)) => format!("Keep Awake: {} left", format_remaining(d)),
        (true, None) => "Keep Awake: on (no limit)".to_string(),
    };
    format!("{head}{footer}")
}
