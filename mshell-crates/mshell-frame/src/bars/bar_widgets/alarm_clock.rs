//! Alarm Clock — bar pill that opens the Alarm Clock menu.
//!
//! An alarm-bell glyph. While the stopwatch runs it shows the live
//! elapsed time inline; while the tone is ringing it pulses (the
//! `ringing` CSS class). Click toggles the menu. The heartbeat only
//! fires while there's something to animate, so a quiet pill is free.

use crate::stopwatch::{self, StopwatchState};
use mshell_sounds::alarm_is_ringing;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

pub(crate) struct AlarmClockModel {
    _orientation: Orientation,
    button: gtk::Button,
    label: gtk::Label,
}

#[derive(Debug)]
pub(crate) enum AlarmClockInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum AlarmClockOutput {
    Clicked,
}

pub(crate) struct AlarmClockInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for AlarmClockModel {
    type CommandOutput = ();
    type Input = AlarmClockInput;
    type Output = AlarmClockOutput;
    type Init = AlarmClockInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "alarm-clock-bar-widget",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_tooltip_text: Some("Alarm Clock"),

            #[local_ref]
            button_widget -> gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(AlarmClockInput::Clicked);
                },

                gtk::Box {
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    gtk::Image {
                        set_icon_name: Some("alarm-symbolic"),
                    },
                    #[local_ref]
                    label_widget -> gtk::Label {
                        add_css_class: "alarm-clock-bar-time",
                        set_visible: false,
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
        let button_widget = gtk::Button::new();
        let label_widget = gtk::Label::new(None);

        let model = AlarmClockModel {
            _orientation: params.orientation,
            button: button_widget.clone(),
            label: label_widget.clone(),
        };
        let widgets = view_output!();

        refresh(&model);

        // Heartbeat — only relayouts while the stopwatch runs or the
        // tone rings; one trailing tick settles the readout when that
        // activity ends.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut tick = tokio::time::interval(Duration::from_millis(200));
            let mut was_active = false;
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tick.tick() => {
                        let active =
                            stopwatch::state() == StopwatchState::Running || alarm_is_ringing();
                        if active || was_active {
                            let _ = out.send(());
                        }
                        was_active = active;
                    }
                }
            }
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AlarmClockInput::Clicked => {
                let _ = sender.output(AlarmClockOutput::Clicked);
            }
        }
    }

    fn update_cmd(
        &mut self,
        _message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        refresh(self);
    }
}

/// Sync the inline stopwatch readout + ringing pulse from the globals.
fn refresh(model: &AlarmClockModel) {
    // Show the time whenever the stopwatch isn't fully stopped.
    match stopwatch::state() {
        StopwatchState::Stopped => model.label.set_visible(false),
        _ => {
            model.label.set_visible(true);
            model.label.set_label(&format_coarse(stopwatch::elapsed()));
        }
    }

    if alarm_is_ringing() {
        model.button.add_css_class("ringing");
    } else {
        model.button.remove_css_class("ringing");
    }
}

/// Bar-friendly `MM:SS` (or `H:MM:SS`) — no flickering centiseconds.
fn format_coarse(d: Duration) -> String {
    let secs = d.as_secs();
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}
