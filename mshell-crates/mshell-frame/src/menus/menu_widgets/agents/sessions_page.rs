//! The AI agents menu's Sessions page: running Claude Code sessions
//! grouped Needs you / Working / Idle (click jumps to the session's
//! terminal), then recent saved sessions to resume.

use crate::agents::SessionsSnapshot;
use crate::agents::sessions::{self, LiveSession, SavedSession, SessionStatus};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};

/// What a row asks the menu to do.
#[derive(Debug, Clone)]
pub(crate) enum SessionAction {
    Focus(u32),
    Resume { session_id: String, cwd: String },
}

/// Rebuild the page into `container`. `notice` is a one-line result of the
/// last action (e.g. "No window found for that session").
pub(crate) fn rebuild(
    container: &gtk::Box,
    snap: Option<&SessionsSnapshot>,
    notice: Option<&str>,
    now_ms: i64,
    on_action: impl Fn(SessionAction) + Clone + 'static,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    if let Some(n) = notice {
        container.append(&small(n, &["label-small", "dim-label"]));
    }
    let Some(snap) = snap else {
        container.append(&small("Loading…", &["label-small", "dim-label"]));
        return;
    };
    if !snap.available {
        container.append(&small(
            "No Claude Code sessions folder — Claude Code 2.1 or newer writes it",
            &["label-small", "dim-label"],
        ));
    }
    for status in [
        SessionStatus::Waiting,
        SessionStatus::Busy,
        SessionStatus::Idle,
    ] {
        let group: Vec<&LiveSession> = snap.live.iter().filter(|s| s.status == status).collect();
        if group.is_empty() {
            continue;
        }
        container.append(&section(status.label()));
        let list = boxed_list();
        for s in group {
            list.append(&live_row(s, now_ms, on_action.clone()));
        }
        container.append(&list);
    }
    if snap.live.is_empty() && snap.available {
        container.append(&small(
            "No Claude Code session is running",
            &["label-small", "dim-label"],
        ));
    }
    if !snap.saved.is_empty() {
        container.append(&section("Recent"));
        let list = boxed_list();
        for s in &snap.saved {
            list.append(&saved_row(s, now_ms, on_action.clone()));
        }
        container.append(&list);
    }
}

fn small(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(classes);
    l.set_halign(gtk::Align::Start);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l
}

fn section(title: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(title));
    l.add_css_class("section-label-menu-widget");
    l.set_halign(gtk::Align::Start);
    l
}

fn boxed_list() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_css_classes(&["ok-boxed-list", "compact", "agents-session-list"]);
    b
}

fn text_block(title: &str, subtitle: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.set_css_classes(&["label-medium"]);
    t.set_halign(gtk::Align::Start);
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&t);
    let s = gtk::Label::new(Some(subtitle));
    s.set_css_classes(&["label-small", "dim-label"]);
    s.set_halign(gtk::Align::Start);
    s.set_xalign(0.0);
    s.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    b.append(&s);
    b
}

fn since(now_ms: i64, then_ms: i64) -> String {
    crate::agents::limits::span(now_ms - then_ms)
}

fn live_row(
    s: &LiveSession,
    now_ms: i64,
    on_action: impl Fn(SessionAction) + 'static,
) -> gtk::Button {
    let btn = gtk::Button::new();
    btn.set_css_classes(&["agents-session-row"]);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    lamp.set_css_classes(&["agents-lamp", s.status.css_class()]);
    lamp.set_valign(gtk::Align::Center);
    row.append(&lamp);
    let mut sub = format!(
        "{} · {} {}",
        sessions::tilde(&s.cwd),
        s.status.label(),
        since(now_ms, s.since_ms)
    );
    if !s.waiting_for.is_empty() {
        sub.push_str(&format!(" · {}", s.waiting_for));
    }
    row.append(&text_block(&s.name, &sub));
    let go = gtk::Image::from_icon_name("go-next-symbolic");
    go.set_valign(gtk::Align::Center);
    go.add_css_class("agents-session-go");
    row.append(&go);
    btn.set_child(Some(&row));
    btn.set_tooltip_text(Some("Jump to its terminal"));
    let pid = s.pid;
    btn.connect_clicked(move |_| on_action(SessionAction::Focus(pid)));
    btn
}

fn saved_row(
    s: &SavedSession,
    now_ms: i64,
    on_action: impl Fn(SessionAction) + 'static,
) -> gtk::Button {
    let btn = gtk::Button::new();
    btn.set_css_classes(&["agents-session-row"]);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let title = if s.last_prompt.is_empty() {
        s.session_id.as_str()
    } else {
        s.last_prompt.as_str()
    };
    row.append(&text_block(
        title,
        &format!(
            "{} · {} ago · {} prompt{}",
            sessions::tilde(&s.project),
            since(now_ms, s.last_ms),
            s.prompts,
            if s.prompts == 1 { "" } else { "s" }
        ),
    ));
    let play = gtk::Image::from_icon_name("media-playback-start-symbolic");
    play.set_valign(gtk::Align::Center);
    play.add_css_class("agents-session-go");
    row.append(&play);
    btn.set_child(Some(&row));
    btn.set_tooltip_text(Some("Resume in a new terminal"));
    let (id, cwd) = (s.session_id.clone(), s.project.clone());
    btn.connect_clicked(move |_| {
        on_action(SessionAction::Resume {
            session_id: id.clone(),
            cwd: cwd.clone(),
        })
    });
    btn
}
