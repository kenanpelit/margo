//! Twilight — bar pill for margo's built-in blue-light filter.
//!
//! Left click opens the Twilight panel (toggle + temperature + mode
//! + schedule presets); right click flips the filter on/off. The
//! pill polls `mctl twilight status` so it tracks the geo/schedule
//! phases and any external `mctl twilight` calls, tints its icon
//! `--primary` while filtering (DESIGN.md §3) and — when on — shows
//! the live colour temperature.

use crate::twilight::{self, TwilightStatus};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(5);
const STARTUP_DELAY: Duration = Duration::from_millis(200);
const POST_TOGGLE_DELAY: Duration = Duration::from_millis(150);

pub(crate) struct TwilightModel {
    status: TwilightStatus,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum TwilightInput {
    /// Left click — open the panel.
    Clicked,
    /// Right click — toggle the filter on/off.
    Toggle,
}

#[derive(Debug)]
pub(crate) enum TwilightOutput {
    Clicked,
}

pub(crate) struct TwilightInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum TwilightCommandOutput {
    Refresh(TwilightStatus),
}

#[relm4::component(pub)]
impl Component for TwilightModel {
    type CommandOutput = TwilightCommandOutput;
    type Input = TwilightInput;
    type Output = TwilightOutput;
    type Init = TwilightInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            #[watch]
            set_css_classes: if model.status.enabled {
                &["twilight-bar-widget", "active"]
            } else {
                &["twilight-bar-widget"]
            },
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(&model.status)),

            gtk::Button {
                // Plain bar-pill surface — active state is a primary
                // icon tint (DESIGN.md §3), not a filled capsule.
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(TwilightInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        #[watch]
                        set_icon_name: Some(model.status.icon()),
                    },
                    gtk::Label {
                        add_css_class: "twilight-bar-label",
                        #[watch]
                        set_label: &temp_text(&model.status),
                        #[watch]
                        set_visible: !temp_text(&model.status).is_empty(),
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
        // Poll `mctl twilight status` so the pill tracks the schedule
        // (and external `mctl twilight` calls), not just our clicks.
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
                if let Some(s) = twilight::probe().await {
                    let _ = out.send(TwilightCommandOutput::Refresh(s));
                }
            }
        });

        let model = TwilightModel {
            status: TwilightStatus::default(),
            _orientation: params.orientation,
        };

        let widgets = view_output!();

        // Right-click → quick toggle (without opening the panel).
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let toggle_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            toggle_sender.input(TwilightInput::Toggle);
        });
        widgets.root.add_controller(gesture);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            TwilightInput::Clicked => {
                let _ = sender.output(TwilightOutput::Clicked);
            }
            TwilightInput::Toggle => {
                sender.command(|out, _shutdown| async move {
                    let _ = tokio::process::Command::new("mctl")
                        .args(["twilight", "toggle"])
                        .status()
                        .await;
                    tokio::time::sleep(POST_TOGGLE_DELAY).await;
                    if let Some(s) = twilight::probe().await {
                        let _ = out.send(TwilightCommandOutput::Refresh(s));
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
            TwilightCommandOutput::Refresh(s) => {
                self.status = s;
            }
        }
    }
}

fn temp_text(s: &TwilightStatus) -> String {
    match (s.enabled, s.current_temp_k) {
        (true, Some(k)) => format!("{k}K"),
        _ => String::new(),
    }
}

fn tooltip(s: &TwilightStatus) -> String {
    let footer = "\n\nClick: open  ·  Right-click: toggle";
    let head = if !s.enabled {
        "Twilight: off".to_string()
    } else {
        let temp = s
            .current_temp_k
            .map(|k| format!("{k}K"))
            .unwrap_or_else(|| "on".to_string());
        let phase = s.phase_label();
        if phase.is_empty() {
            format!("Twilight: {temp} · {}", s.mode)
        } else {
            format!("Twilight: {temp} · {phase} · {}", s.mode)
        }
    };
    format!("{head}{footer}")
}
