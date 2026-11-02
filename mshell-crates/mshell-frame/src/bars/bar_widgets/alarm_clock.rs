//! Alarm Clock — bar pill that opens the Alarm Clock menu.
//!
//! An alarm-bell glyph. While the stopwatch runs it shows the live
//! elapsed time inline; while the tone is ringing it pulses (the
//! `ringing` CSS class). Click toggles the menu. The pill sleeps until the
//! stopwatch or the alarm changes state; while the stopwatch runs it wakes
//! once per displayed second, so a quiet pill costs nothing.

use crate::stopwatch::{self, StopwatchState};
use mshell_sounds::{alarm_is_ringing, stop_alarm};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, WidgetExt};
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
    /// Right-click — silence the ringing alarm without opening the menu.
    StopRinging,
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

        // Right-click → silence the ringing alarm (no menu).
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let stop_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            stop_sender.input(AlarmClockInput::StopRinging);
        });
        widgets.button_widget.add_controller(gesture);

        refresh(&model);

        // Event-driven heartbeat: wait for a stopwatch / alarm state change,
        // and while the stopwatch runs also wake on each whole displayed
        // second (the readout is `MM:SS`). The ringing pulse is a CSS
        // animation, so ringing alone needs no ticks.
        mshell_sounds::set_alarm_state_listener(stopwatch::notify_changed);
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut changes = stopwatch::changes();
            loop {
                let running = stopwatch::state() == StopwatchState::Running;
                let next_second = next_readout_change(running, stopwatch::elapsed());
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = changes.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    () = sleep_or_forever(next_second) => {}
                }
                let _ = out.send(());
            }
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AlarmClockInput::Clicked => {
                let _ = sender.output(AlarmClockOutput::Clicked);
            }
            AlarmClockInput::StopRinging => {
                if alarm_is_ringing() {
                    stop_alarm();
                }
                refresh(self);
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
        model
            .button
            .set_tooltip_text(Some("Ringing — right-click to stop"));
    } else {
        model.button.remove_css_class("ringing");
        model.button.set_tooltip_text(Some("Alarm Clock"));
    }
}

/// How long until the `MM:SS` readout next changes: the rest of the current
/// second while the stopwatch runs, `None` (nothing to tick) otherwise.
fn next_readout_change(running: bool, elapsed: Duration) -> Option<Duration> {
    running.then(|| {
        let into_second = Duration::from_nanos(u64::from(elapsed.subsec_nanos()));
        // A hair past the boundary so the readout has definitely rolled.
        Duration::from_secs(1) - into_second + Duration::from_millis(5)
    })
}

async fn sleep_or_forever(delay: Option<Duration>) {
    match delay {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readout_ticks_only_while_running_and_on_second_boundaries() {
        assert_eq!(
            next_readout_change(false, Duration::from_millis(1234)),
            None
        );
        assert_eq!(
            next_readout_change(true, Duration::from_millis(1234)),
            Some(Duration::from_millis(771))
        );
        assert_eq!(
            next_readout_change(true, Duration::from_secs(7)),
            Some(Duration::from_millis(1005))
        );
    }
}
