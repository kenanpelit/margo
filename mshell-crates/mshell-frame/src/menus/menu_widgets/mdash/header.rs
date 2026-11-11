//! mdash's header: the greeting, the date and time, Settings, and ONE
//! power button whose popover holds Lock / Suspend / Log out / Reboot /
//! Shut down. The destructive three ask once more inside the popover; the
//! actions run through the Session menu's `run_session_action`, so the
//! `[session]` commands the user configured apply here too.

use chrono::NaiveDateTime;
use mshell_utils::session::SessionAction;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, PopoverExt, WidgetExt};
use std::rc::Rc;

/// What the header asks the dashboard to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderEvent {
    Settings,
    Session(SessionAction),
}

/// The popover's order: the safe actions first, the destructive last.
pub(crate) const POWER_ACTIONS: [SessionAction; 5] = [
    SessionAction::Lock,
    SessionAction::Suspend,
    SessionAction::Logout,
    SessionAction::Reboot,
    SessionAction::Shutdown,
];

/// Actions that end the session or the machine: confirmed first.
pub(crate) fn needs_confirm(action: SessionAction) -> bool {
    matches!(
        action,
        SessionAction::Logout | SessionAction::Reboot | SessionAction::Shutdown
    )
}

pub(crate) fn action_label(action: SessionAction) -> &'static str {
    match action {
        SessionAction::Lock => "Lock",
        SessionAction::Suspend => "Suspend",
        SessionAction::Logout => "Log out",
        SessionAction::Reboot => "Reboot",
        SessionAction::Shutdown => "Shut down",
    }
}

fn confirm_question(action: SessionAction) -> &'static str {
    match action {
        SessionAction::Logout => "Log out now?",
        SessionAction::Reboot => "Reboot now?",
        _ => "Shut down now?",
    }
}

/// "Tuesday 10 November · 20:23" (or "· 8:23 PM" with a 12-hour clock).
pub(crate) fn datetime_line(now: NaiveDateTime, h24: bool) -> String {
    let date = now.format("%A %-d %B");
    let time = if h24 {
        now.format("%H:%M").to_string()
    } else {
        now.format("%-I:%M %p").to_string()
    };
    format!("{date} · {time}")
}

pub(crate) struct Header {
    pub(crate) root: gtk::Box,
    greeting: gtk::Label,
    clock: gtk::Label,
}

impl Header {
    pub(crate) fn new(on: impl Fn(HeaderEvent) + 'static) -> Self {
        let on: Rc<dyn Fn(HeaderEvent)> = Rc::new(on);
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("mdash-header");

        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let greeting = gtk::Label::new(None);
        greeting.add_css_class("mdash-greeting");
        greeting.set_xalign(0.0);
        greeting.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&greeting);
        let clock = gtk::Label::new(None);
        clock.add_css_class("mdash-datetime");
        clock.set_xalign(0.0);
        text.append(&clock);
        root.append(&text);

        let settings = gtk::Button::from_icon_name("settings-symbolic");
        settings.add_css_class("panel-action-btn");
        settings.set_valign(gtk::Align::Center);
        mshell_common::a11y::name(&settings, "Settings");
        {
            let on = on.clone();
            settings.connect_clicked(move |_| on(HeaderEvent::Settings));
        }
        root.append(&settings);

        let power = gtk::MenuButton::new();
        power.set_icon_name("system-shutdown-symbolic");
        power.add_css_class("panel-action-btn");
        power.add_css_class("mdash-power-button");
        power.set_valign(gtk::Align::Center);
        mshell_common::a11y::name(&power, "Power");
        let popover = gtk::Popover::new();
        popover.add_css_class("mdash-power-popover");
        popover.set_child(Some(&power_page(&popover, on)));
        power.set_popover(Some(&popover));
        root.append(&power);

        Self {
            root,
            greeting,
            clock,
        }
    }

    pub(crate) fn set_time(&self, greeting: &str, datetime: &str) {
        if self.greeting.label() != greeting {
            self.greeting.set_label(greeting);
        }
        if self.clock.label() != datetime {
            self.clock.set_label(datetime);
        }
    }
}

/// The popover's content: the action list, and an inline confirmation
/// page for the destructive actions (back to the list on close).
fn power_page(popover: &gtk::Popover, on: Rc<dyn Fn(HeaderEvent)>) -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_vhomogeneous(false);
    stack.set_hhomogeneous(true);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    list.add_css_class("mdash-power-list");

    let confirm = gtk::Box::new(gtk::Orientation::Vertical, 10);
    confirm.add_css_class("mdash-power-confirm");
    let question = gtk::Label::new(None);
    question.add_css_class("mdash-power-question");
    question.set_xalign(0.0);
    confirm.append(&question);
    let hint = gtk::Label::new(Some("Unsaved work in open apps is lost."));
    hint.add_css_class("mdash-power-hint");
    hint.set_xalign(0.0);
    confirm.append(&hint);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_homogeneous(true);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("ok-button-surface");
    cancel.add_css_class("ok-button-cell");
    let go = gtk::Button::new();
    go.add_css_class("ok-button-error");
    go.add_css_class("ok-button-cell");
    buttons.append(&cancel);
    buttons.append(&go);
    confirm.append(&buttons);

    let pending: Rc<std::cell::Cell<Option<SessionAction>>> = Rc::default();
    for action in POWER_ACTIONS {
        let row = gtk::Button::new();
        row.add_css_class("mdash-power-row");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.append(&gtk::Image::from_icon_name(action.icon()));
        let label = gtk::Label::new(Some(action_label(action)));
        label.set_xalign(0.0);
        content.append(&label);
        row.set_child(Some(&content));
        if needs_confirm(action) {
            row.add_css_class("destructive");
        }
        let (on, popover, stack, question, go, pending) = (
            on.clone(),
            popover.clone(),
            stack.clone(),
            question.clone(),
            go.clone(),
            pending.clone(),
        );
        row.connect_clicked(move |_| {
            if needs_confirm(action) {
                pending.set(Some(action));
                question.set_label(confirm_question(action));
                go.set_label(action_label(action));
                stack.set_visible_child_name("confirm");
                go.grab_focus();
            } else {
                popover.popdown();
                on(HeaderEvent::Session(action));
            }
        });
        list.append(&row);
    }

    {
        let (popover, pending) = (popover.clone(), pending.clone());
        go.connect_clicked(move |_| {
            if let Some(action) = pending.take() {
                popover.popdown();
                on(HeaderEvent::Session(action));
            }
        });
    }
    {
        let (stack, pending) = (stack.clone(), pending.clone());
        cancel.connect_clicked(move |_| {
            pending.set(None);
            stack.set_visible_child_name("list");
        });
    }
    {
        let stack = stack.clone();
        popover.connect_closed(move |_| {
            pending.set(None);
            stack.set_visible_child_name("list");
        });
    }

    stack.add_named(&list, Some("list"));
    stack.add_named(&confirm, Some("confirm"));
    stack.set_visible_child_name("list");
    stack
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn only_session_ending_actions_ask_first() {
        let asked: Vec<_> = POWER_ACTIONS
            .into_iter()
            .filter(|a| needs_confirm(*a))
            .collect();
        assert_eq!(
            asked,
            [
                SessionAction::Logout,
                SessionAction::Reboot,
                SessionAction::Shutdown
            ]
        );
        // Every session action is reachable from the one power button.
        assert_eq!(POWER_ACTIONS.len(), SessionAction::ALL.len());
    }

    #[test]
    fn datetime_line_in_both_clock_formats() {
        let t = NaiveDate::from_ymd_opt(2026, 11, 10)
            .and_then(|d| d.and_hms_opt(20, 23, 0))
            .expect("valid");
        assert_eq!(datetime_line(t, true), "Tuesday 10 November · 20:23");
        assert_eq!(datetime_line(t, false), "Tuesday 10 November · 8:23 PM");
    }
}
