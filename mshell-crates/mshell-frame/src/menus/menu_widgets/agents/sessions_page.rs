//! The AI agents menu's Sessions page: running Claude Code sessions
//! grouped Needs you / Working / Idle (click jumps to the session's
//! terminal), each with what it is doing (current tool, todo progress)
//! and chips for its model, permission mode, context use, estimated cost
//! and Remote Control; then recent saved sessions to resume. With several
//! Claude accounts every row names its account. Built once and updated in
//! place on every poll (see [`super::keyed_rows`]).

use super::keyed_rows::{KeyedList, KeyedRow};
use super::library_section::{LibUi, LibrarySection};
use crate::agents::SessionsSnapshot;
use crate::agents::library::classify::LibStatus;
use crate::agents::library::{LibAction, LibSession, LibrarySnapshot};
use crate::agents::local_stats::compact_count;
use crate::agents::pricing::{self, Currency};
use crate::agents::sessions::{self, LiveSession, SavedSession, SessionStatus};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, Cast, EditableExt, WidgetExt};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
    /// Start a new session in `cwd` under the account at `config_dir`.
    New {
        cwd: String,
        config_dir: PathBuf,
    },
    /// Open a URL or file in its default app (a Remote Control session on
    /// claude.ai, a CLAUDE.md).
    Open(String),
    /// Change saved sessions on disk (after the row's confirm).
    Library(LibAction),
    /// List the leftovers of deleted sessions for a confirm.
    FindLeftovers,
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
    /// Full-text search over every prompt typed.
    search: gtk::SearchEntry,
    query: String,
    match_title: gtk::Label,
    match_box: gtk::Box,
    matches: KeyedList<SavedRow>,
    no_match: gtk::Label,
    /// "New session in…": recent projects, shown on demand.
    show_new: bool,
    new_title: gtk::Label,
    new_box: gtk::Box,
    new_rows: KeyedList<NewRow>,
    multi: bool,
    /// Every saved session on disk (the session manager).
    library: LibrarySection,
    /// The scan's sessions by `(account dir, id)`, newest copy: recent
    /// and matching rows show their status from it.
    lib_by_id: HashMap<(PathBuf, String), LibSession>,
    /// The recent sessions last shown, to re-annotate on a new scan.
    last_saved: Vec<SavedSession>,
    on_action: Rc<dyn Fn(SessionAction)>,
}

/// Most projects offered for a new session.
const NEW_PROJECTS: usize = 8;

/// Projects to start a new session in: recent saved sessions' projects,
/// then running sessions' folders, each `(config dir, project)` once.
pub(crate) fn recent_projects(snap: &SessionsSnapshot, limit: usize) -> Vec<NewItem> {
    let mut out: Vec<NewItem> = Vec::new();
    let saved = snap
        .saved
        .iter()
        .map(|s| (&s.config_dir, &s.project, &s.account));
    let live = snap
        .live
        .iter()
        .map(|s| (&s.config_dir, &s.cwd, &s.account));
    for (dir, project, account) in saved.chain(live) {
        if project.is_empty()
            || out
                .iter()
                .any(|n| n.config_dir == *dir && n.project == *project)
        {
            continue;
        }
        out.push(NewItem {
            config_dir: dir.clone(),
            project: project.clone(),
            account: account.clone(),
            multi: snap.multi_account(),
        });
        if out.len() == limit {
            break;
        }
    }
    out
}

impl SessionsPage {
    /// Build the page into `container` (which it then owns).
    pub(crate) fn new(
        container: &gtk::Box,
        on_action: impl Fn(SessionAction) + 'static,
        on_search: impl Fn(String) + 'static,
        on_lib: impl Fn(LibUi) + 'static,
    ) -> Self {
        let on_action: Rc<dyn Fn(SessionAction)> = Rc::new(on_action);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search every prompt you typed…"));
        search.add_css_class("agents-search");
        search.connect_search_changed(move |e| on_search(e.text().trim().to_string()));
        container.append(&search);
        let new_title = section("New session in…");
        let new_box = boxed_list();
        container.append(&new_title);
        container.append(&new_box);
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
        let match_title = section("Matching prompts");
        let match_box = boxed_list();
        let no_match = small("No prompt matches", &["label-small", "dim-label"]);
        container.append(&match_title);
        container.append(&match_box);
        container.append(&no_match);
        let library = LibrarySection::new(container, on_action.clone(), Rc::new(on_lib));
        let page = SessionsPage {
            notice,
            loading,
            unavailable,
            groups,
            none_running,
            recent_title,
            recent: KeyedList::new(recent_box.clone()),
            recent_box,
            search,
            query: String::new(),
            matches: KeyedList::new(match_box.clone()),
            match_title,
            match_box,
            no_match,
            show_new: false,
            new_rows: KeyedList::new(new_box.clone()),
            new_title,
            new_box,
            multi: false,
            library,
            lib_by_id: HashMap::new(),
            last_saved: Vec::new(),
            on_action,
        };
        page.show_loading_only();
        page.show_matches(false);
        page.new_title.set_visible(false);
        page.new_box.set_visible(false);
        page
    }

    /// Rows for `saved`, each with what the scan knows about it.
    fn saved_items(&self, saved: &[SavedSession], now_ms: i64, multi: bool) -> Vec<SavedItem> {
        saved
            .iter()
            .map(|s| SavedItem {
                lib: self
                    .lib_by_id
                    .get(&(s.config_dir.clone(), s.session_id.clone()))
                    .cloned(),
                session: s.clone(),
                now_ms,
                multi,
            })
            .collect()
    }

    /// Show a fresh scan of every saved session.
    pub(crate) fn set_library(&mut self, snap: LibrarySnapshot, now_ms: i64) {
        self.lib_by_id.clear();
        // Newest first: the first copy of an id wins.
        for s in &snap.sessions {
            self.lib_by_id
                .entry((s.config_dir.clone(), s.id.clone()))
                .or_insert_with(|| s.clone());
        }
        self.library.set_snapshot(snap, now_ms);
        let items = self.saved_items(&self.last_saved, now_ms, self.multi);
        let on_action = self.on_action.clone();
        self.recent
            .sync(&items, |item| SavedRow::new(item, on_action.clone()));
    }

    /// The session manager's own controls.
    pub(crate) fn library_ui(&mut self, msg: LibUi, now_ms: i64) {
        self.library.handle(msg, now_ms);
    }

    fn show_matches(&self, on: bool) {
        self.match_title.set_visible(on && !self.matches.is_empty());
        self.match_box.set_visible(on && !self.matches.is_empty());
        self.no_match.set_visible(on && self.matches.is_empty());
    }

    /// Show the hits of a full-text search for `query` (the entry may
    /// have moved on; stale results are dropped).
    pub(crate) fn set_matches(&mut self, query: &str, hits: &[SavedSession], now_ms: i64) {
        if query != self.search.text().trim() {
            return;
        }
        self.query = query.to_string();
        self.library.set_query(query, now_ms);
        let items = self.saved_items(hits, now_ms, self.multi);
        let on_action = self.on_action.clone();
        self.matches
            .sync(&items, |item| SavedRow::new(item, on_action.clone()));
        let searching = !self.query.is_empty();
        self.show_matches(searching);
        self.recent_title
            .set_visible(!searching && !self.recent.is_empty());
        self.recent_box
            .set_visible(!searching && !self.recent.is_empty());
    }

    /// Show or hide "New session in…".
    pub(crate) fn toggle_new(&mut self) {
        self.show_new = !self.show_new;
        let on = self.show_new && !self.new_rows.is_empty();
        self.new_title.set_visible(on);
        self.new_box.set_visible(on);
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
        self.last_saved = snap.saved.clone();
        let items = self.saved_items(&snap.saved, now_ms, multi);
        let on_action = self.on_action.clone();
        self.recent
            .sync(&items, |item| SavedRow::new(item, on_action.clone()));
        let searching = !self.query.is_empty();
        self.recent_title
            .set_visible(!searching && !self.recent.is_empty());
        self.recent_box
            .set_visible(!searching && !self.recent.is_empty());
        self.multi = multi;
        let projects = recent_projects(snap, NEW_PROJECTS);
        let on_action = self.on_action.clone();
        self.new_rows
            .sync(&projects, |item| NewRow::new(item, on_action.clone()));
        let on = self.show_new && !self.new_rows.is_empty();
        self.new_title.set_visible(on);
        self.new_box.set_visible(on);
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
    /// Claude's last reply, folded under the row.
    reply_btn: gtk::Button,
    reply: gtk::Label,
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
        remote.connect_clicked(move |_| act(SessionAction::Open(u.borrow().clone())));
        top.append(&remote);

        let reply_btn = gtk::Button::from_icon_name("pan-down-symbolic");
        reply_btn.set_css_classes(&["ok-button-flat", "agents-session-action"]);
        reply_btn.set_valign(gtk::Align::Center);
        mshell_common::a11y::name(&reply_btn, "Show Claude's last reply");
        top.append(&reply_btn);
        let reply = gtk::Label::new(None);
        reply.set_css_classes(&["label-small", "agents-session-reply"]);
        reply.set_halign(gtk::Align::Start);
        reply.set_xalign(0.0);
        reply.set_wrap(true);
        reply.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        reply.set_selectable(true);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_child(Some(&reply));
        root.append(&revealer);
        reply_btn.connect_clicked(move |b| {
            let open = !revealer.reveals_child();
            revealer.set_reveal_child(open);
            b.set_icon_name(if open {
                "pan-up-symbolic"
            } else {
                "pan-down-symbolic"
            });
            let name = if open {
                "Hide Claude's last reply"
            } else {
                "Show Claude's last reply"
            };
            mshell_common::a11y::name(b, name);
        });

        let r = LiveRow {
            root,
            lamp,
            title,
            subtitle,
            doing,
            chips,
            remote,
            reply_btn,
            reply,
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
        let reply = s.detail.as_ref().map_or("", |d| d.reply.as_str());
        self.reply.set_label(reply);
        self.reply_btn.set_visible(!reply.is_empty());
    }
}

/// A project to start a new session in.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NewItem {
    pub(crate) config_dir: PathBuf,
    pub(crate) project: String,
    pub(crate) account: String,
    pub(crate) multi: bool,
}

/// "Start Claude in <project>"; click opens a terminal running `claude`.
pub(crate) struct NewRow {
    button: gtk::Button,
    title: gtk::Label,
    subtitle: gtk::Label,
}

impl NewRow {
    fn new(item: &NewItem, on_action: Rc<dyn Fn(SessionAction)>) -> Self {
        let button = gtk::Button::new();
        button.set_css_classes(&["agents-session-row"]);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let (block, title, subtitle) = text_block();
        row.append(&block);
        let add = gtk::Image::from_icon_name("list-add-symbolic");
        add.set_valign(gtk::Align::Center);
        add.add_css_class("agents-session-go");
        row.append(&add);
        button.set_child(Some(&row));
        button.set_tooltip_text(Some("Start a new Claude Code session here"));
        let (cwd, dir) = (item.project.clone(), item.config_dir.clone());
        button.connect_clicked(move |_| {
            on_action(SessionAction::New {
                cwd: cwd.clone(),
                config_dir: dir.clone(),
            })
        });
        let r = NewRow {
            button,
            title,
            subtitle,
        };
        r.update(item);
        r
    }
}

impl KeyedRow for NewRow {
    type Item = NewItem;
    type Key = (PathBuf, String);

    fn key(item: &Self::Item) -> Self::Key {
        (item.config_dir.clone(), item.project.clone())
    }

    fn widget(&self) -> gtk::Widget {
        self.button.clone().upcast()
    }

    fn update(&self, item: &Self::Item) {
        self.title.set_label(&sessions::tilde(&item.project));
        self.subtitle.set_label(&if item.multi {
            format!("New session · {}", item.account)
        } else {
            "New session".to_string()
        });
    }
}

/// A recent (or matching) session as its row shows it, with what the
/// scan of the saved sessions knows about it.
#[derive(Debug, Clone)]
pub(crate) struct SavedItem {
    pub(crate) session: SavedSession,
    pub(crate) now_ms: i64,
    pub(crate) multi: bool,
    pub(crate) lib: Option<LibSession>,
}

/// Where a recent session resumes from, or why it doesn't: a moved
/// project resumes from its new folder; orphan, misplaced and unknown
/// ones don't resume from here.
pub(crate) fn saved_resume(item: &SavedItem) -> Result<String, String> {
    match &item.lib {
        Some(l) if l.status != LibStatus::Ok => match l.resume_dir() {
            Some(dir) => Ok(dir),
            None => Err(l.status.reason(&l.cwd)),
        },
        _ => Ok(item.session.project.clone()),
    }
}

/// A recent session to resume: last prompt, project, age; click resumes.
pub(crate) struct SavedRow {
    button: gtk::Button,
    glyph: gtk::Image,
    title: gtk::Label,
    subtitle: gtk::Label,
    go: gtk::Image,
    /// Where a click resumes it; `None` = it can't resume from here.
    cwd: Rc<RefCell<Option<String>>>,
}

impl SavedRow {
    fn new(item: &SavedItem, on_action: Rc<dyn Fn(SessionAction)>) -> Self {
        let button = gtk::Button::new();
        button.set_css_classes(&["agents-session-row"]);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let glyph = gtk::Image::new();
        glyph.set_valign(gtk::Align::Center);
        row.append(&glyph);
        let (block, title, subtitle) = text_block();
        row.append(&block);
        let play = gtk::Image::from_icon_name("media-playback-start-symbolic");
        play.set_valign(gtk::Align::Center);
        play.add_css_class("agents-session-go");
        row.append(&play);
        button.set_child(Some(&row));
        let cwd = Rc::new(RefCell::new(None));
        let (id, c, dir) = (
            item.session.session_id.clone(),
            cwd.clone(),
            item.session.config_dir.clone(),
        );
        button.connect_clicked(move |_| {
            let Some(cwd) = c.borrow().clone() else {
                return;
            };
            on_action(SessionAction::Resume {
                session_id: id.clone(),
                cwd,
                config_dir: dir.clone(),
            })
        });
        let r = SavedRow {
            button,
            glyph,
            title,
            subtitle,
            go: play,
            cwd,
        };
        r.update(item);
        r
    }
}

impl KeyedRow for SavedRow {
    type Item = SavedItem;
    type Key = String;

    fn key(item: &Self::Item) -> String {
        item.session.session_id.clone()
    }

    fn widget(&self) -> gtk::Widget {
        self.button.clone().upcast()
    }

    fn update(&self, item: &Self::Item) {
        let (s, now_ms, multi) = (&item.session, &item.now_ms, &item.multi);
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
        let resume = saved_resume(item);
        let status = item.lib.as_ref().map(|l| &l.status);
        match status.and_then(LibStatus::icon) {
            Some(icon) => {
                self.glyph.set_icon_name(Some(icon));
                let name = status.map_or("", LibStatus::name);
                self.glyph.set_css_classes(&["agents-library-status", name]);
                self.glyph.set_visible(true);
            }
            None => self.glyph.set_visible(false),
        }
        self.go.set_visible(resume.is_ok());
        let tip = match (&resume, status) {
            (Err(why), _) => why.clone(),
            (Ok(dir), Some(LibStatus::Moved(_))) => {
                format!("Resume in a new terminal, from {}", sessions::tilde(dir))
            }
            (Ok(_), _) => "Resume in a new terminal".to_string(),
        };
        self.button.set_tooltip_text(Some(&tip));
        self.cwd.replace(resume.ok());
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
    fn new_session_projects_are_recent_and_unique() {
        let saved = |p: &str, ms| SavedSession {
            session_id: format!("s{ms}"),
            project: p.into(),
            last_prompt: String::new(),
            last_ms: ms,
            prompts: 1,
            config_dir: PathBuf::from("/h/.claude"),
            account: "default".into(),
        };
        let mut running = test_session(SessionStatus::Busy, 0);
        running.cwd = "/r/live".into();
        running.config_dir = PathBuf::from("/h/.claude");
        let snap = SessionsSnapshot {
            available: true,
            live: vec![running],
            saved: vec![
                saved("/r/a", 30),
                saved("/r/b", 20),
                saved("/r/a", 10),
                saved("", 5),
            ],
            accounts: vec!["default".into()],
        };
        let got: Vec<_> = recent_projects(&snap, 8)
            .into_iter()
            .map(|n| n.project)
            .collect();
        assert_eq!(got, ["/r/a", "/r/b", "/r/live"]);
        assert_eq!(recent_projects(&snap, 1).len(), 1);
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
