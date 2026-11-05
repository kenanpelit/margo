//! The Alarm Clock menu's Pomodoro tab: phase hero, time left, progress,
//! session count and the Start/Pause · Skip · Reset controls for the
//! shell-wide [`crate::pomodoro`] timer. Buttons drive the timer directly;
//! the menu repaints from the timer's change channel, so the bar pill,
//! `mshellctl pomodoro` and this tab always agree.

use crate::pomodoro::engine::RunState;
use crate::pomodoro::{self, Command, Phase, Snapshot};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};

pub(super) struct PomodoroTab {
    pub(super) root: gtk::Box,
    phase: gtk::Label,
    time: gtk::Label,
    progress: gtk::ProgressBar,
    session: gtk::Label,
    primary: gtk::Button,
    reset: gtk::Button,
}

impl PomodoroTab {
    pub(super) fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("alarm-pomodoro");
        root.set_valign(gtk::Align::Center);

        let phase = gtk::Label::new(None);
        phase.add_css_class("alarm-pomodoro-phase");
        phase.set_halign(gtk::Align::Center);
        root.append(&phase);

        let time = gtk::Label::new(None);
        time.add_css_class("alarm-stopwatch-time");
        time.set_halign(gtk::Align::Center);
        root.append(&time);

        let progress = gtk::ProgressBar::new();
        progress.add_css_class("alarm-pomodoro-progress");
        root.append(&progress);

        let session = gtk::Label::new(None);
        session.add_css_class("label-small");
        session.set_halign(gtk::Align::Center);
        root.append(&session);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_homogeneous(true);
        let button = |label: &str, classes: &[&str], cmd: fn(&Snapshot) -> Command| {
            let b = gtk::Button::with_label(label);
            for c in classes {
                b.add_css_class(c);
            }
            b.connect_clicked(move |_| {
                pomodoro::run(cmd(&pomodoro::snapshot()));
            });
            buttons.append(&b);
            b
        };
        let primary = button("Start", &["ok-button-primary", "ok-button-cell"], |_| {
            Command::Toggle
        });
        button("Skip", &["ok-button-surface", "ok-button-cell"], |_| {
            Command::Skip
        });
        let reset = button(
            "Reset",
            &["ok-button-surface", "ok-button-cell"],
            reset_command,
        );
        root.append(&buttons);

        let hint = gtk::Label::new(Some(
            "Lengths and auto-start: Settings → Widgets → Alarm Clock",
        ));
        hint.add_css_class("label-small");
        hint.add_css_class("alarm-pomodoro-hint");
        hint.set_wrap(true);
        hint.set_justify(gtk::Justification::Center);
        root.append(&hint);

        let tab = Self {
            root,
            phase,
            time,
            progress,
            session,
            primary,
            reset,
        };
        tab.sync();
        tab
    }

    /// Repaint from the timer.
    pub(super) fn sync(&self) {
        let snap = pomodoro::snapshot();
        self.phase.set_label(snap.phase.label());
        self.time
            .set_label(&pomodoro::format_remaining(snap.remaining));
        self.progress.set_fraction(snap.progress);
        self.session.set_label(&session_text(&snap));
        self.primary.set_label(primary_label(snap.run_state));
        self.reset.set_label(reset_label(&snap));
        self.reset.set_sensitive(!is_pristine(&snap));
        // Active = primary tint on the hero while a phase counts down.
        for (class, on) in [
            ("running", snap.run_state == RunState::Running),
            ("on-break", snap.phase.is_break()),
        ] {
            if on {
                self.root.add_css_class(class);
            } else {
                self.root.remove_css_class(class);
            }
        }
    }
}

fn primary_label(state: RunState) -> &'static str {
    match state {
        RunState::Ready => "Start",
        RunState::Running => "Pause",
        RunState::Paused => "Resume",
    }
}

/// Reset refills a started phase; on an untouched phase it starts a new
/// round instead (so the button is never a no-op).
fn reset_command(snap: &Snapshot) -> Command {
    if snap.run_state == RunState::Ready {
        Command::ResetAll
    } else {
        Command::Reset
    }
}

fn reset_label(snap: &Snapshot) -> &'static str {
    match reset_command(snap) {
        Command::ResetAll => "New round",
        _ => "Reset",
    }
}

/// The very first state: nothing to reset.
fn is_pristine(snap: &Snapshot) -> bool {
    snap.run_state == RunState::Ready
        && snap.phase == Phase::Work
        && snap.session == 1
        && snap.completed == 0
}

fn session_text(snap: &Snapshot) -> String {
    let done = match snap.completed {
        0 => String::new(),
        1 => " · 1 pomodoro done".to_string(),
        n => format!(" · {n} pomodoros done"),
    };
    format!("Session {} of {}{done}", snap.session, snap.cycles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn snap(phase: Phase, run_state: RunState, session: u32, completed: u32) -> Snapshot {
        Snapshot {
            phase,
            run_state,
            remaining: Duration::from_secs(60),
            progress: 0.0,
            session,
            cycles: 4,
            completed,
        }
    }

    #[test]
    fn the_primary_button_follows_the_run_state() {
        assert_eq!(primary_label(RunState::Ready), "Start");
        assert_eq!(primary_label(RunState::Running), "Pause");
        assert_eq!(primary_label(RunState::Paused), "Resume");
    }

    #[test]
    fn reset_refills_a_started_phase_or_starts_a_new_round() {
        let paused = snap(Phase::Work, RunState::Paused, 2, 1);
        assert_eq!(reset_command(&paused), Command::Reset);
        let ready = snap(Phase::ShortBreak, RunState::Ready, 2, 1);
        assert_eq!(reset_command(&ready), Command::ResetAll);
        assert_eq!(reset_label(&ready), "New round");
        assert!(is_pristine(&snap(Phase::Work, RunState::Ready, 1, 0)));
        assert!(!is_pristine(&ready));
    }

    #[test]
    fn session_text_counts_finished_pomodoros() {
        assert_eq!(
            session_text(&snap(Phase::Work, RunState::Ready, 1, 0)),
            "Session 1 of 4"
        );
        assert_eq!(
            session_text(&snap(Phase::Work, RunState::Ready, 2, 1)),
            "Session 2 of 4 · 1 pomodoro done"
        );
        assert_eq!(
            session_text(&snap(Phase::Work, RunState::Ready, 3, 6)),
            "Session 3 of 4 · 6 pomodoros done"
        );
    }
}
