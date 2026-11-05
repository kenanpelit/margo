//! The AI agents menu's Sessions page: running Claude Code sessions
//! grouped Needs you / Working / Idle (click jumps to the session's
//! terminal), then recent saved sessions to resume. Built once and
//! updated in place on every poll (see [`super::keyed_rows`]).

use super::keyed_rows::{KeyedList, KeyedRow};
use crate::agents::SessionsSnapshot;
use crate::agents::sessions::{self, LiveSession, SavedSession, SessionStatus};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, Cast, WidgetExt};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// What a row asks the menu to do.
#[derive(Debug, Clone)]
pub(crate) enum SessionAction {
    Focus(u32),
    Resume { session_id: String, cwd: String },
}

/// The page's widgets, built once. A refresh shows or hides the fixed
/// parts and syncs the rows by session id ([`KeyedList`]), so the focused
/// row and the scroll position survive the 3-second poll.
pub(crate) struct SessionsPage {
    notice: gtk::Label,
    loading: gtk::Label,
    unavailable: gtk::Label,
    /// Needs you / Working / Idle: the section title and its rows.
    groups: Vec<(SessionStatus, gtk::Label, gtk::Box, KeyedList<LiveRow>)>,
    none_running: gtk::Label,
    recent_title: gtk::Label,
    recent_box: gtk::Box,
    recent: KeyedList<SavedRow>,
    on_action: Rc<dyn Fn(SessionAction)>,
}

impl SessionsPage {
    /// Build the page into `container` (which it then owns).
    pub(crate) fn new(container: &gtk::Box, on_action: impl Fn(SessionAction) + 'static) -> Self {
        let notice = small("", &["label-small", "dim-label"]);
        let loading = small("Loading…", &["label-small", "dim-label"]);
        let unavailable = small(
            "No Claude Code sessions folder — Claude Code 2.1 or newer writes it",
            &["label-small", "dim-label"],
        );
        container.append(&notice);
        container.append(&loading);
        container.append(&unavailable);
        let mut groups = Vec::new();
        for status in [
            SessionStatus::Waiting,
            SessionStatus::Busy,
            SessionStatus::Idle,
        ] {
            let title = section(status.label());
            let list = boxed_list();
            container.append(&title);
            container.append(&list);
            groups.push((status, title, list.clone(), KeyedList::new(list)));
        }
        let none_running = small(
            "No Claude Code session is running",
            &["label-small", "dim-label"],
        );
        container.append(&none_running);
        let recent_title = section("Recent");
        let recent_box = boxed_list();
        container.append(&recent_title);
        container.append(&recent_box);
        let page = SessionsPage {
            notice,
            loading,
            unavailable,
            groups,
            none_running,
            recent_title,
            recent: KeyedList::new(recent_box.clone()),
            recent_box,
            on_action: Rc::new(on_action),
        };
        page.show_loading_only();
        page
    }

    fn show_loading_only(&self) {
        self.loading.set_visible(true);
        self.unavailable.set_visible(false);
        for (_, title, list, _) in &self.groups {
            title.set_visible(false);
            list.set_visible(false);
        }
        self.none_running.set_visible(false);
        self.recent_title.set_visible(false);
        self.recent_box.set_visible(false);
    }

    /// Show `snap` (`None` = still loading). `notice` is a one-line result
    /// of the last action (e.g. "No window found for that session").
    pub(crate) fn update(
        &mut self,
        snap: Option<&SessionsSnapshot>,
        notice: Option<&str>,
        now_ms: i64,
    ) {
        self.notice.set_label(notice.unwrap_or(""));
        self.notice.set_visible(notice.is_some());
        let Some(snap) = snap else {
            self.show_loading_only();
            return;
        };
        self.loading.set_visible(false);
        self.unavailable.set_visible(!snap.available);
        for (status, title, list, rows) in &mut self.groups {
            let items: Vec<(LiveSession, i64)> = snap
                .live
                .iter()
                .filter(|s| s.status == *status)
                .map(|s| (s.clone(), now_ms))
                .collect();
            let on_action = self.on_action.clone();
            rows.sync(&items, |item| LiveRow::new(item, on_action.clone()));
            title.set_visible(!rows.is_empty());
            list.set_visible(!rows.is_empty());
        }
        self.none_running
            .set_visible(snap.live.is_empty() && snap.available);
        let items: Vec<(SavedSession, i64)> =
            snap.saved.iter().cloned().map(|s| (s, now_ms)).collect();
        let on_action = self.on_action.clone();
        self.recent
            .sync(&items, |item| SavedRow::new(item, on_action.clone()));
        self.recent_title.set_visible(!self.recent.is_empty());
        self.recent_box.set_visible(!self.recent.is_empty());
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

/// Title + dim subtitle; returns the box and both labels.
fn text_block() -> (gtk::Box, gtk::Label, gtk::Label) {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.set_hexpand(true);
    let t = gtk::Label::new(None);
    t.set_css_classes(&["label-medium"]);
    t.set_halign(gtk::Align::Start);
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&t);
    let s = gtk::Label::new(None);
    s.set_css_classes(&["label-small", "dim-label"]);
    s.set_halign(gtk::Align::Start);
    s.set_xalign(0.0);
    s.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    b.append(&s);
    (b, t, s)
}

fn since(now_ms: i64, then_ms: i64) -> String {
    crate::agents::limits::span(now_ms - then_ms)
}

/// A running session: lamp, name, where + status; click jumps to it.
pub(crate) struct LiveRow {
    button: gtk::Button,
    lamp: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    pid: Rc<Cell<u32>>,
}

impl LiveRow {
    fn new(item: &(LiveSession, i64), on_action: Rc<dyn Fn(SessionAction)>) -> Self {
        let button = gtk::Button::new();
        button.set_css_classes(&["agents-session-row"]);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        lamp.set_valign(gtk::Align::Center);
        row.append(&lamp);
        let (block, title, subtitle) = text_block();
        row.append(&block);
        let go = gtk::Image::from_icon_name("go-next-symbolic");
        go.set_valign(gtk::Align::Center);
        go.add_css_class("agents-session-go");
        row.append(&go);
        button.set_child(Some(&row));
        button.set_tooltip_text(Some("Jump to its terminal"));
        let pid = Rc::new(Cell::new(item.0.pid));
        let p = pid.clone();
        button.connect_clicked(move |_| on_action(SessionAction::Focus(p.get())));
        let r = LiveRow {
            button,
            lamp,
            title,
            subtitle,
            pid,
        };
        r.update(item);
        r
    }
}

impl KeyedRow for LiveRow {
    type Item = (LiveSession, i64);
    type Key = String;

    fn key(item: &Self::Item) -> String {
        item.0.session_id.clone()
    }

    fn widget(&self) -> gtk::Widget {
        self.button.clone().upcast()
    }

    fn update(&self, (s, now_ms): &Self::Item) {
        self.lamp
            .set_css_classes(&["agents-lamp", s.status.css_class()]);
        let mut sub = format!(
            "{} · {} {}",
            sessions::tilde(&s.cwd),
            s.status.label(),
            since(*now_ms, s.since_ms)
        );
        if !s.waiting_for.is_empty() {
            sub.push_str(&format!(" · {}", s.waiting_for));
        }
        self.title.set_label(&s.name);
        self.subtitle.set_label(&sub);
        self.pid.set(s.pid);
    }
}

/// A recent session to resume: last prompt, project, age; click resumes.
pub(crate) struct SavedRow {
    button: gtk::Button,
    title: gtk::Label,
    subtitle: gtk::Label,
    cwd: Rc<RefCell<String>>,
}

impl SavedRow {
    fn new(item: &(SavedSession, i64), on_action: Rc<dyn Fn(SessionAction)>) -> Self {
        let button = gtk::Button::new();
        button.set_css_classes(&["agents-session-row"]);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let (block, title, subtitle) = text_block();
        row.append(&block);
        let play = gtk::Image::from_icon_name("media-playback-start-symbolic");
        play.set_valign(gtk::Align::Center);
        play.add_css_class("agents-session-go");
        row.append(&play);
        button.set_child(Some(&row));
        button.set_tooltip_text(Some("Resume in a new terminal"));
        let cwd = Rc::new(RefCell::new(String::new()));
        let (id, c) = (item.0.session_id.clone(), cwd.clone());
        button.connect_clicked(move |_| {
            on_action(SessionAction::Resume {
                session_id: id.clone(),
                cwd: c.borrow().clone(),
            })
        });
        let r = SavedRow {
            button,
            title,
            subtitle,
            cwd,
        };
        r.update(item);
        r
    }
}

impl KeyedRow for SavedRow {
    type Item = (SavedSession, i64);
    type Key = String;

    fn key(item: &Self::Item) -> String {
        item.0.session_id.clone()
    }

    fn widget(&self) -> gtk::Widget {
        self.button.clone().upcast()
    }

    fn update(&self, (s, now_ms): &Self::Item) {
        let title = if s.last_prompt.is_empty() {
            s.session_id.as_str()
        } else {
            s.last_prompt.as_str()
        };
        self.title.set_label(title);
        self.subtitle.set_label(&format!(
            "{} · {} ago · {} prompt{}",
            sessions::tilde(&s.project),
            since(*now_ms, s.last_ms),
            s.prompts,
            if s.prompts == 1 { "" } else { "s" }
        ));
        self.cwd.replace(s.project.clone());
    }
}
