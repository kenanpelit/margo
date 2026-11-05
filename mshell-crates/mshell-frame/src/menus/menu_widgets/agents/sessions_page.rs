//! The AI agents menu's Sessions page: running Claude Code sessions
//! grouped Needs you / Working / Idle (click jumps to the session's
//! terminal), each with what it is doing (current tool, todo progress)
//! and chips for its model, permission mode, context use, estimated cost
//! and Remote Control; then recent saved sessions to resume. With several
//! Claude accounts every row names its account. Built once and updated in
//! place on every poll (see [`super::keyed_rows`]).

use super::keyed_rows::{KeyedList, KeyedRow};
use crate::agents::SessionsSnapshot;
use crate::agents::local_stats::compact_count;
use crate::agents::pricing::{self, Currency};
use crate::agents::sessions::{self, LiveSession, SavedSession, SessionStatus};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, Cast, WidgetExt};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

/// What a row asks the menu to do.
#[derive(Debug, Clone)]
pub(crate) enum SessionAction {
    Focus(u32),
    Resume {
        session_id: String,
        cwd: String,
        /// The account's config dir; empty = the default account.
        config_dir: PathBuf,
    },
    /// Open a URL (a Remote Control session on claude.ai).
    OpenUrl(String),
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
        let multi = snap.multi_account();
        let eur = pricing::current().eur;
        for (status, title, list, rows) in &mut self.groups {
            let items: Vec<LiveItem> = snap
                .live
                .iter()
                .filter(|s| s.status == *status)
                .map(|s| LiveItem {
                    session: s.clone(),
                    now_ms,
                    multi,
                    eur,
                })
                .collect();
            let on_action = self.on_action.clone();
            rows.sync(&items, |item| LiveRow::new(item, on_action.clone()));
            title.set_visible(!rows.is_empty());
            list.set_visible(!rows.is_empty());
        }
        self.none_running
            .set_visible(snap.live.is_empty() && snap.available);
        let items: Vec<(SavedSession, i64, bool)> = snap
            .saved
            .iter()
            .cloned()
            .map(|s| (s, now_ms, multi))
            .collect();
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

/// One running session as a row shows it.
#[derive(Debug, Clone)]
pub(crate) struct LiveItem {
    pub(crate) session: LiveSession,
    pub(crate) now_ms: i64,
    /// Several accounts: name the session's.
    pub(crate) multi: bool,
    /// EUR per USD, for the cost chip when EUR is chosen.
    pub(crate) eur: Option<f64>,
}

/// What the session is doing, one line: the todo in progress (with
/// progress) or the tool it runs. Empty when idle or unknown.
pub(crate) fn doing_line(s: &LiveSession) -> String {
    let Some(d) = s.detail.as_ref() else {
        return String::new();
    };
    let todos = d
        .todos
        .map(|t| format!("{}/{} ", t.done, t.total))
        .unwrap_or_default();
    if s.status == SessionStatus::Idle {
        return d
            .todos
            .filter(|t| t.done < t.total)
            .map(|t| format!("Todos {}/{}", t.done, t.total))
            .unwrap_or_default();
    }
    if !d.todo_now.is_empty() {
        format!("{todos}{}", d.todo_now)
    } else if !d.tool.is_empty() {
        format!("{todos}{}", d.tool)
    } else {
        todos.trim_end().to_string()
    }
}

/// The chips under a session: account, model, permission mode, context
/// use, estimated cost, remote. `(text, extra class)`.
pub(crate) fn chips(
    s: &LiveSession,
    multi: bool,
    currency: Currency,
    eur: Option<f64>,
) -> Vec<(String, &'static str)> {
    let mut v = Vec::new();
    if multi {
        v.push((s.account.clone(), ""));
    }
    if let Some(d) = &s.detail {
        if !d.model.is_empty() {
            v.push((d.model.clone(), ""));
        }
        match d.mode.as_str() {
            "" | "default" => {}
            "bypassPermissions" => v.push(("bypass".into(), "warn")),
            "acceptEdits" => v.push(("accept edits".into(), "")),
            "plan" => v.push(("plan".into(), "")),
            other => v.push((other.to_string(), "")),
        }
        if d.context > 0 {
            let text = match s.context_fraction {
                Some(f) => format!("ctx {} · {:.0}%", compact_count(d.context), f * 100.0),
                None => format!("ctx {}", compact_count(d.context)),
            };
            let class = match s.context_fraction {
                Some(f) if f >= 0.9 => "danger",
                Some(f) if f >= 0.7 => "warn",
                _ => "",
            };
            v.push((text, class));
        }
    }
    if let Some(c) = s.cost_usd.and_then(|c| pricing::money(c, currency, eur)) {
        v.push((format!("~{c}"), ""));
    }
    if s.bridge.is_some() {
        v.push(("remote".into(), "accent"));
    }
    v
}

/// A running session: lamp, title, where + status, what it is doing and
/// its chips; click jumps to it. Remote Control sessions get an "open on
/// claude.ai" button.
pub(crate) struct LiveRow {
    root: gtk::Box,
    lamp: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    doing: gtk::Label,
    chips: gtk::Box,
    remote: gtk::Button,
    pid: Rc<Cell<u32>>,
    url: Rc<RefCell<String>>,
}

impl LiveRow {
    fn new(item: &LiveItem, on_action: Rc<dyn Fn(SessionAction)>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("agents-session-row");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        root.append(&top);
        let button = gtk::Button::new();
        button.set_css_classes(&["ok-button-flat", "agents-session-main"]);
        button.set_hexpand(true);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        lamp.set_valign(gtk::Align::Center);
        row.append(&lamp);
        let (block, title, subtitle) = text_block();
        let doing = gtk::Label::new(None);
        doing.set_css_classes(&["label-small", "agents-session-doing"]);
        doing.set_halign(gtk::Align::Start);
        doing.set_xalign(0.0);
        doing.set_ellipsize(gtk::pango::EllipsizeMode::End);
        block.append(&doing);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        chips.add_css_class("agents-session-chips");
        block.append(&chips);
        row.append(&block);
        let go = gtk::Image::from_icon_name("go-next-symbolic");
        go.set_valign(gtk::Align::Center);
        go.add_css_class("agents-session-go");
        row.append(&go);
        button.set_child(Some(&row));
        button.set_tooltip_text(Some("Jump to its terminal"));
        top.append(&button);
        let pid = Rc::new(Cell::new(item.session.pid));
        let p = pid.clone();
        let act = on_action.clone();
        button.connect_clicked(move |_| act(SessionAction::Focus(p.get())));

        let remote = gtk::Button::from_icon_name("web-browser-symbolic");
        remote.set_css_classes(&["ok-button-flat", "agents-session-action"]);
        remote.set_valign(gtk::Align::Center);
        mshell_common::a11y::name(&remote, "Open on claude.ai (Remote Control)");
        let url = Rc::new(RefCell::new(String::new()));
        let u = url.clone();
        let act = on_action.clone();
        remote.connect_clicked(move |_| act(SessionAction::OpenUrl(u.borrow().clone())));
        top.append(&remote);

        let r = LiveRow {
            root,
            lamp,
            title,
            subtitle,
            doing,
            chips,
            remote,
            pid,
            url,
        };
        r.update(item);
        r
    }
}

impl KeyedRow for LiveRow {
    type Item = LiveItem;
    type Key = String;

    fn key(item: &Self::Item) -> String {
        item.session.key()
    }

    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn update(&self, item: &Self::Item) {
        let s = &item.session;
        self.lamp
            .set_css_classes(&["agents-lamp", s.status.css_class()]);
        let mut place = sessions::tilde(&s.cwd);
        if let Some(b) = s
            .detail
            .as_ref()
            .map(|d| d.branch.as_str())
            .filter(|b| !b.is_empty())
        {
            place.push_str(&format!(" ({b})"));
        }
        let mut sub = format!(
            "{place} · {} {}",
            s.status.label(),
            since(item.now_ms, s.since_ms)
        );
        if !s.waiting_for.is_empty() {
            sub.push_str(&format!(" · {}", s.waiting_for));
        }
        self.title.set_label(s.title());
        self.subtitle.set_label(&sub);
        let doing = doing_line(s);
        self.doing.set_label(&doing);
        self.doing.set_visible(!doing.is_empty());
        while let Some(c) = self.chips.first_child() {
            self.chips.remove(&c);
        }
        let list = chips(s, item.multi, crate::agents::settings::currency(), item.eur);
        for (text, class) in &list {
            let l = gtk::Label::new(Some(text));
            l.add_css_class("agents-chip");
            if !class.is_empty() {
                l.add_css_class(class);
            }
            self.chips.append(&l);
        }
        self.chips.set_visible(!list.is_empty());
        self.pid.set(s.pid);
        let url = s.remote_url().unwrap_or_default();
        self.remote.set_visible(!url.is_empty());
        self.url.replace(url);
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
    fn new(item: &(SavedSession, i64, bool), on_action: Rc<dyn Fn(SessionAction)>) -> Self {
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
        let (id, c, dir) = (
            item.0.session_id.clone(),
            cwd.clone(),
            item.0.config_dir.clone(),
        );
        button.connect_clicked(move |_| {
            on_action(SessionAction::Resume {
                session_id: id.clone(),
                cwd: c.borrow().clone(),
                config_dir: dir.clone(),
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
    type Item = (SavedSession, i64, bool);
    type Key = String;

    fn key(item: &Self::Item) -> String {
        item.0.session_id.clone()
    }

    fn widget(&self) -> gtk::Widget {
        self.button.clone().upcast()
    }

    fn update(&self, (s, now_ms, multi): &Self::Item) {
        let title = if s.last_prompt.is_empty() {
            s.session_id.as_str()
        } else {
            s.last_prompt.as_str()
        };
        self.title.set_label(title);
        let account = if *multi {
            format!(" · {}", s.account)
        } else {
            String::new()
        };
        self.subtitle.set_label(&format!(
            "{} · {} ago · {} prompt{}{account}",
            sessions::tilde(&s.project),
            since(*now_ms, s.last_ms),
            s.prompts,
            if s.prompts == 1 { "" } else { "s" }
        ));
        self.cwd.replace(s.project.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::details::{SessionDetail, Todos};
    use crate::agents::sessions::test_session;

    fn live(status: SessionStatus) -> LiveSession {
        let mut s = test_session(status, 0);
        s.account = "work".into();
        s.detail = Some(SessionDetail {
            model: "opus-5-5".into(),
            mode: "bypassPermissions".into(),
            tool: "Bash: npm test".into(),
            context: 377_136,
            todos: Some(Todos { done: 1, total: 3 }),
            ..SessionDetail::default()
        });
        s.cost_usd = Some(1.234);
        s.context_fraction = Some(0.377);
        s
    }

    #[test]
    fn doing_line_prefers_the_todo_then_the_tool() {
        let mut s = live(SessionStatus::Busy);
        assert_eq!(doing_line(&s), "1/3 Bash: npm test");
        s.detail.as_mut().unwrap().todo_now = "Fixing the bug".into();
        assert_eq!(doing_line(&s), "1/3 Fixing the bug");
        // Idle: no tool, only unfinished todos.
        assert_eq!(doing_line(&live(SessionStatus::Idle)), "Todos 1/3");
        assert_eq!(doing_line(&test_session(SessionStatus::Busy, 0)), "");
    }

    #[test]
    fn chips_name_account_model_mode_context_cost_and_remote() {
        let mut s = live(SessionStatus::Busy);
        s.bridge = Some("session_x".into());
        let got: Vec<_> = chips(&s, true, Currency::Usd, None)
            .into_iter()
            .map(|(t, c)| format!("{t}{}", if c.is_empty() { "" } else { "!" }))
            .collect();
        assert_eq!(
            got,
            [
                "work",
                "opus-5-5",
                "bypass!",
                "ctx 377.1k · 38%",
                "~$1.23",
                "remote!"
            ]
        );
        // One account: no account chip; EUR without a rate: no cost chip.
        let v = chips(&s, false, Currency::Eur, None);
        assert!(v.iter().all(|(t, _)| t != "work" && !t.starts_with('~')));
        let v = chips(&s, false, Currency::Eur, Some(0.9));
        assert!(v.iter().any(|(t, _)| t == "~€1.11"));
    }
}
