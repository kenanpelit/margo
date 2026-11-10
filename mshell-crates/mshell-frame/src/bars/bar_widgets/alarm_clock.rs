//! Alarm Clock — bar pill that opens the Alarm Clock menu.
//!
//! An alarm-bell glyph. While the stopwatch runs it shows the live
//! elapsed time inline; otherwise, while a one-shot timer / reminder is
//! pending (and `alarm.show_timer_on_pill` is on), the soonest one's time
//! left (`09:59`, `1h 05m`). While the tone is ringing it pulses (the
//! `ringing` CSS class). Click toggles the menu — on its Timers tab when a
//! timer is shown. The pill sleeps until the stopwatch, the alarm or the
//! timers change; it wakes once per displayed change of a readout, so a
//! quiet pill costs nothing.

use crate::stopwatch::{self, StopwatchState};
use crate::timers::{self, Timer, format};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{AlarmConfigStoreFields, ConfigStoreFields};
use mshell_sounds::{alarm_is_ringing, stop_alarm};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tokio::sync::watch;

pub(crate) struct AlarmClockModel {
    _orientation: Orientation,
    button: gtk::Button,
    label: gtk::Label,
    /// `alarm.show_timer_on_pill`, mirrored for the off-thread heartbeat.
    show_timer_tx: watch::Sender<bool>,
    /// A timer readout is on the pill (a click opens the Timers tab).
    showing_timer: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum AlarmClockInput {
    Clicked,
    /// Right-click — silence the ringing alarm without opening the menu.
    StopRinging,
    /// `alarm.show_timer_on_pill` changed.
    ShowTimerChanged(bool),
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

        let mut effects = EffectScope::new();
        let effect_sender = sender.clone();
        effects.push(move |_| {
            let show = config_manager().config().alarm().show_timer_on_pill().get();
            effect_sender.input(AlarmClockInput::ShowTimerChanged(show));
        });
        let model = AlarmClockModel {
            _orientation: params.orientation,
            button: button_widget.clone(),
            label: label_widget.clone(),
            show_timer_tx: watch::channel(
                config_manager()
                    .config()
                    .alarm()
                    .show_timer_on_pill()
                    .get_untracked(),
            )
            .0,
            showing_timer: false,
            _effects: effects,
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

        let mut model = model;
        refresh(&mut model);

        // Event-driven heartbeat: wait for a stopwatch / alarm / timer
        // change, and while a readout shows also wake when it next changes
        // (each second of the stopwatch's `MM:SS`; a timer's `09:59` each
        // second, its `1h 05m` each minute). The ringing pulse is a CSS
        // animation, so ringing alone needs no ticks.
        mshell_sounds::set_alarm_state_listener(stopwatch::notify_changed);
        let mut show_timer_rx = model.show_timer_tx.subscribe();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut changes = stopwatch::changes();
            let mut timer_changes = timers::changes();
            loop {
                let running = stopwatch::state() == StopwatchState::Running;
                timer_changes.borrow_and_update();
                let timer_tick = if !running && *show_timer_rx.borrow_and_update() {
                    timers::next_readout_change()
                } else {
                    None
                };
                let next = next_readout_change(running, stopwatch::elapsed()).or(timer_tick);
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = changes.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    changed = timer_changes.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    changed = show_timer_rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    () = sleep_or_forever(next) => {}
                }
                let _ = out.send(());
            }
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AlarmClockInput::Clicked => {
                if self.showing_timer {
                    timers::request_tab();
                }
                let _ = sender.output(AlarmClockOutput::Clicked);
            }
            AlarmClockInput::ShowTimerChanged(show) => {
                self.show_timer_tx.send_replace(show);
                refresh(self);
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

/// What the pill's inline label shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Readout {
    None,
    Stopwatch(String),
    /// A timer's time left, and its tooltip.
    Timer(String, String),
}

/// A running stopwatch wins (it is being watched); then the soonest timer
/// when the switch is on; then a paused stopwatch. With no timer pending
/// the pill is exactly what it was before timers existed.
fn readout(
    stopwatch: StopwatchState,
    elapsed: Duration,
    timer: Option<&Timer>,
    show_timer: bool,
    now: i64,
) -> Readout {
    let stopwatch_text = || Readout::Stopwatch(format_coarse(elapsed));
    match (stopwatch, timer.filter(|_| show_timer)) {
        (StopwatchState::Running, _) => stopwatch_text(),
        (_, Some(t)) => {
            let rings = format::rings_phrase(&timers::due_clock(t, now));
            let tip = match t.label.as_str() {
                "" => format!("Timer — {rings}"),
                label => format!("{label} — {rings}"),
            };
            Readout::Timer(format::pill_text(t.remaining(now)), tip)
        }
        (StopwatchState::Paused, None) => stopwatch_text(),
        (StopwatchState::Stopped, None) => Readout::None,
    }
}

/// Sync the inline readout + ringing pulse from the globals.
fn refresh(model: &mut AlarmClockModel) {
    let show_timer = *model.show_timer_tx.borrow();
    let next = timers::next();
    let shown = readout(
        stopwatch::state(),
        stopwatch::elapsed(),
        next.as_ref(),
        show_timer,
        timers::now_unix(),
    );
    model.showing_timer = matches!(shown, Readout::Timer(..));
    let tooltip = match &shown {
        Readout::None => {
            model.label.set_visible(false);
            "Alarm Clock".to_string()
        }
        Readout::Stopwatch(text) => {
            model.label.set_visible(true);
            model.label.set_label(text);
            "Alarm Clock".to_string()
        }
        Readout::Timer(text, tip) => {
            model.label.set_visible(true);
            model.label.set_label(text);
            tip.clone()
        }
    };

    if alarm_is_ringing() {
        model.button.add_css_class("ringing");
        model
            .button
            .set_tooltip_text(Some("Ringing — right-click to stop"));
    } else {
        model.button.remove_css_class("ringing");
        model.button.set_tooltip_text(Some(&tooltip));
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
    use crate::timers::TimerKind;

    fn timer(left: i64, label: &str) -> Timer {
        Timer {
            id: 1,
            due: 1_000 + left,
            created: 1_000,
            kind: TimerKind::Timer,
            label: label.to_string(),
        }
    }

    #[test]
    fn with_no_timer_the_pill_is_unchanged() {
        let e = Duration::from_secs(75);
        assert_eq!(
            readout(StopwatchState::Stopped, e, None, true, 1_000),
            Readout::None
        );
        assert_eq!(
            readout(StopwatchState::Running, e, None, true, 1_000),
            Readout::Stopwatch("01:15".into())
        );
        assert_eq!(
            readout(StopwatchState::Paused, e, None, true, 1_000),
            Readout::Stopwatch("01:15".into())
        );
    }

    #[test]
    fn a_pending_timer_shows_its_time_left_unless_switched_off() {
        let t = timer(599, "tea");
        let e = Duration::ZERO;
        match readout(StopwatchState::Stopped, e, Some(&t), true, 1_000) {
            Readout::Timer(text, tip) => {
                assert_eq!(text, "09:59");
                assert!(tip.starts_with("tea — rings "), "{tip}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            readout(StopwatchState::Stopped, e, Some(&t), false, 1_000),
            Readout::None
        );
        // A running stopwatch keeps the pill; a paused one yields to the timer.
        assert!(matches!(
            readout(StopwatchState::Running, e, Some(&t), true, 1_000),
            Readout::Stopwatch(_)
        ));
        assert!(matches!(
            readout(StopwatchState::Paused, e, Some(&t), true, 1_000),
            Readout::Timer(..)
        ));
    }

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
