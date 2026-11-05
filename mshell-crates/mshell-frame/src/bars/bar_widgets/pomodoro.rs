//! Pomodoro — bar pill for the shell's Pomodoro timer
//! ([`crate::pomodoro`]).
//!
//! Shows the phase glyph + the time left (`24:13`; vertical bars show just
//! the minutes). The root carries `running` / `on-break` state classes:
//! a running focus session tints primary (DESIGN §3), a break tints the
//! calm success colour, a paused or ready timer stays neutral.
//! Left-click opens the Alarm Clock menu on its Pomodoro tab; right-click
//! starts / pauses. The pill sleeps until the timer changes and, while a
//! phase runs, wakes once per displayed second only.

use crate::bars::pill_classes::PillClasses;
use crate::pomodoro::engine::RunState;
use crate::pomodoro::{self, Command, Phase, Snapshot};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Instant;

pub(crate) struct PomodoroModel {
    orientation: Orientation,
    root: gtk::Box,
    icon: gtk::Image,
    label: gtk::Label,
}

#[derive(Debug)]
pub(crate) enum PomodoroInput {
    Clicked,
    ToggleRun,
}

#[derive(Debug)]
pub(crate) enum PomodoroOutput {
    Clicked,
}

pub(crate) struct PomodoroInit {
    pub(crate) orientation: Orientation,
}

const STATE_CLASSES: &[&str] = &["running", "on-break"];

#[relm4::component(pub)]
impl Component for PomodoroModel {
    type CommandOutput = ();
    type Input = PomodoroInput;
    type Output = PomodoroOutput;
    type Init = PomodoroInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["pomodoro-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,

            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| sender.input(PomodoroInput::Clicked),

                gtk::Box {
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    #[local_ref]
                    icon_widget -> gtk::Image {},
                    #[local_ref]
                    label_widget -> gtk::Label {
                        add_css_class: "pomodoro-bar-time",
                    },
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let icon_widget = gtk::Image::new();
        let label_widget = gtk::Label::new(None);
        let model = PomodoroModel {
            orientation: params.orientation,
            root: root.clone(),
            icon: icon_widget.clone(),
            label: label_widget.clone(),
        };
        let widgets = view_output!();

        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let toggle_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| toggle_sender.input(PomodoroInput::ToggleRun));
        widgets.button.add_controller(gesture);

        refresh(&model);

        // Sleep until the timer changes; while a phase runs, also wake on
        // each displayed second.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut changes = pomodoro::changes();
            loop {
                let deadline = changes.borrow_and_update().deadline;
                let tick = pomodoro::next_readout_change(deadline, Instant::now());
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = changes.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    () = sleep_or_forever(tick) => {}
                }
                let _ = out.send(());
            }
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            PomodoroInput::Clicked => {
                pomodoro::request_tab();
                let _ = sender.output(PomodoroOutput::Clicked);
            }
            PomodoroInput::ToggleRun => {
                pomodoro::run(Command::Toggle);
                refresh(self);
            }
        }
    }

    fn update_cmd(&mut self, _: Self::CommandOutput, _: ComponentSender<Self>, _: &Self::Root) {
        refresh(self);
    }
}

fn refresh(model: &PomodoroModel) {
    let snap = pomodoro::snapshot();
    let vertical = model.orientation == Orientation::Vertical;
    model.label.set_label(&pill_text(&snap, vertical));
    model.icon.set_icon_name(Some(phase_icon(snap.phase)));
    model
        .root
        .set_state_classes(STATE_CLASSES, state_classes(&snap));
    model.root.set_tooltip_text(Some(&tooltip(&snap)));
}

/// The inline readout: `MM:SS` (horizontal) or whole minutes left (vertical).
fn pill_text(snap: &Snapshot, vertical: bool) -> String {
    if vertical {
        let mins = snap.remaining.as_secs().div_ceil(60);
        format!("{mins}m")
    } else {
        pomodoro::format_remaining(snap.remaining)
    }
}

fn phase_icon(phase: Phase) -> &'static str {
    match phase {
        Phase::Work => "timer-symbolic",
        Phase::ShortBreak | Phase::LongBreak => "coffee-symbolic",
    }
}

/// `running` while a phase counts down, plus `on-break` during a break.
fn state_classes(snap: &Snapshot) -> &'static [&'static str] {
    match (snap.run_state == RunState::Running, snap.phase.is_break()) {
        (true, false) => &["running"],
        (true, true) => &["running", "on-break"],
        (false, true) => &["on-break"],
        (false, false) => &[],
    }
}

fn tooltip(snap: &Snapshot) -> String {
    let state = match snap.run_state {
        RunState::Ready => "ready",
        RunState::Running => "running",
        RunState::Paused => "paused",
    };
    format!(
        "Pomodoro · {} {} · {} left\nSession {} of {} · {} done\nClick: open the Pomodoro tab  ·  Right-click: start / pause",
        snap.phase.label(),
        state,
        pomodoro::format_remaining(snap.remaining),
        snap.session,
        snap.cycles,
        snap.completed,
    )
}

async fn sleep_or_forever(delay: Option<std::time::Duration>) {
    match delay {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn snap(phase: Phase, run_state: RunState, secs: u64) -> Snapshot {
        Snapshot {
            phase,
            run_state,
            remaining: Duration::from_secs(secs),
            progress: 0.0,
            session: 1,
            cycles: 4,
            completed: 0,
        }
    }

    #[test]
    fn the_pill_shows_mm_ss_or_minutes_on_a_vertical_bar() {
        let s = snap(Phase::Work, RunState::Running, 24 * 60 + 13);
        assert_eq!(pill_text(&s, false), "24:13");
        assert_eq!(pill_text(&s, true), "25m", "partial minutes round up");
    }

    #[test]
    fn state_classes_tint_running_focus_and_breaks() {
        assert_eq!(
            state_classes(&snap(Phase::Work, RunState::Running, 1)),
            &["running"]
        );
        assert_eq!(
            state_classes(&snap(Phase::ShortBreak, RunState::Running, 1)),
            &["running", "on-break"]
        );
        assert!(state_classes(&snap(Phase::Work, RunState::Paused, 1)).is_empty());
        assert!(state_classes(&snap(Phase::Work, RunState::Ready, 1)).is_empty());
    }
}
