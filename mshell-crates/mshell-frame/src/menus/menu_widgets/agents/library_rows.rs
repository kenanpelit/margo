//! Rows of the Sessions page's session manager: a project folder (header
//! + its sessions, unfolded on click) and one saved session (click
//! resumes it; the chevron unfolds Copy / Rename / Lock / Repair /
//! Delete, each destructive one behind an inline confirm).

use super::keyed_rows::{KeyedList, KeyedRow};
use super::sessions_page::SessionAction;
use crate::agents::library::classify::LibStatus;
use crate::agents::library::view::Group;
use crate::agents::library::{LibAction, LibSession, SessionRef};
use crate::agents::sessions;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, Cast, EditableExt, EntryExt, WidgetExt};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Sessions a folder draws before "Show more".
pub(crate) const PAGE: usize = 20;

/// One project folder as the list shows it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GroupItem {
    pub(crate) group: Group,
    pub(crate) now_ms: i64,
    pub(crate) multi: bool,
    /// Open regardless of the user's choice (a filter or search is on).
    pub(crate) auto_open: bool,
}

struct GroupInner {
    chevron: gtk::Image,
    name: gtk::Label,
    lamp: gtk::Box,
    count: gtk::Label,
    revealer: gtk::Revealer,
    list: RefCell<KeyedList<SessionRow>>,
    more: gtk::Button,
    item: RefCell<Option<GroupItem>>,
    /// The user unfolded it.
    open: Cell<bool>,
    limit: Cell<usize>,
    on_action: Rc<dyn Fn(SessionAction)>,
}

impl GroupInner {
    fn is_open(&self) -> bool {
        self.open.get() || self.item.borrow().as_ref().is_some_and(|i| i.auto_open)
    }

    /// Show the open state and (when open) sync the session rows.
    fn refresh(&self) {
        let open = self.is_open();
        self.revealer.set_reveal_child(open);
        self.chevron.set_icon_name(Some(if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
        if !open {
            return;
        }
        let Some(item) = self.item.borrow().clone() else {
            return;
        };
        let shown = item.group.sessions.len().min(self.limit.get());
        let rows: Vec<RowItem> = item.group.sessions[..shown]
            .iter()
            .map(|s| RowItem {
                session: s.clone(),
                now_ms: item.now_ms,
                multi: item.multi,
            })
            .collect();
        let on_action = self.on_action.clone();
        self.list
            .borrow_mut()
            .sync(&rows, |r| SessionRow::new(r, on_action.clone()));
        let rest = item.group.sessions.len() - shown;
        self.more.set_visible(rest > 0);
        self.more
            .set_label(&format!("Show {} more", rest.min(PAGE)));
    }
}

/// A project folder: its header and, unfolded, its sessions.
pub(crate) struct GroupRow {
    root: gtk::Box,
    inner: Rc<GroupInner>,
}

impl GroupRow {
    pub(crate) fn new(item: &GroupItem, on_action: Rc<dyn Fn(SessionAction)>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.add_css_class("agents-library-group");
        let head = gtk::Button::new();
        head.set_css_classes(&["ok-button-flat", "agents-library-group-head"]);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let chevron = gtk::Image::from_icon_name("pan-end-symbolic");
        chevron.add_css_class("agents-session-go");
        row.append(&chevron);
        let name = gtk::Label::new(None);
        name.set_css_classes(&["label-medium-bold"]);
        name.set_halign(gtk::Align::Start);
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        row.append(&name);
        let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        lamp.set_css_classes(&["agents-lamp", "busy"]);
        lamp.set_valign(gtk::Align::Center);
        lamp.set_tooltip_text(Some("A session here is running"));
        row.append(&lamp);
        let count = gtk::Label::new(None);
        count.set_css_classes(&["label-small", "dim-label", "agents-stat-value"]);
        row.append(&count);
        head.set_child(Some(&row));
        root.append(&head);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.set_css_classes(&["ok-boxed-list", "compact", "agents-session-list"]);
        let rows_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&rows_box);
        let more = gtk::Button::with_label("Show more");
        more.set_css_classes(&["agents-session-row", "label-small"]);
        body.append(&more);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_child(Some(&body));
        root.append(&revealer);
        let inner = Rc::new(GroupInner {
            chevron,
            name,
            lamp,
            count,
            revealer,
            list: RefCell::new(KeyedList::new(rows_box)),
            more: more.clone(),
            item: RefCell::new(None),
            open: Cell::new(false),
            limit: Cell::new(PAGE),
            on_action,
        });
        let i = inner.clone();
        head.connect_clicked(move |_| {
            i.open.set(!i.is_open());
            i.refresh();
        });
        let i = inner.clone();
        more.connect_clicked(move |_| {
            i.limit.set(i.limit.get() + PAGE);
            i.refresh();
        });
        let r = GroupRow { root, inner };
        r.update(item);
        r
    }
}

impl KeyedRow for GroupRow {
    type Item = GroupItem;
    type Key = String;

    fn key(item: &GroupItem) -> String {
        item.group.key()
    }

    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn update(&self, item: &GroupItem) {
        let g = &item.group;
        let mut name = sessions::tilde(&g.folder);
        if item.multi {
            name.push_str(&format!(" · {}", g.account));
        }
        self.inner.name.set_label(&name);
        self.inner.name.set_tooltip_text(Some(&g.folder));
        self.inner.lamp.set_visible(g.running);
        let n = g.sessions.len();
        self.inner
            .count
            .set_label(&format!("{n} session{}", if n == 1 { "" } else { "s" }));
        self.inner.item.replace(Some(item.clone()));
        self.inner.refresh();
    }
}

/// One saved session as its row shows it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RowItem {
    pub(crate) session: LibSession,
    pub(crate) now_ms: i64,
    pub(crate) multi: bool,
}

/// What the unfolded part of a session row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Actions,
    Renaming,
    ConfirmDelete,
    ConfirmRepair,
}

struct RowInner {
    glyph: gtk::Image,
    lamp: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    chips: gtk::Box,
    go: gtk::Image,
    main: gtk::Button,
    fold: gtk::Button,
    revealer: gtk::Revealer,
    actions: gtk::Box,
    copy: gtk::Button,
    rename: gtk::Button,
    lock: gtk::Button,
    repair: gtk::Button,
    delete: gtk::Button,
    rename_box: gtk::Box,
    entry: gtk::Entry,
    confirm: gtk::Box,
    confirm_label: gtk::Label,
    confirm_go: gtk::Button,
    session: RefCell<Option<LibSession>>,
    mode: Cell<Mode>,
    on_action: Rc<dyn Fn(SessionAction)>,
}

impl RowInner {
    fn set_mode(&self, mode: Mode) {
        self.mode.set(mode);
        self.actions.set_visible(mode == Mode::Actions);
        self.rename_box.set_visible(mode == Mode::Renaming);
        self.confirm
            .set_visible(matches!(mode, Mode::ConfirmDelete | Mode::ConfirmRepair));
        let Some(s) = self.session.borrow().clone() else {
            return;
        };
        match mode {
            Mode::Renaming => {
                let current = if s.custom_title.is_empty() {
                    s.title.clone()
                } else {
                    s.custom_title.clone()
                };
                self.entry.set_text(&current);
                self.entry.grab_focus();
            }
            Mode::ConfirmDelete => {
                self.confirm_label
                    .set_label("Delete this session? Its transcript and history go for good.");
                self.confirm_go.set_label("Delete");
                self.confirm_go
                    .set_css_classes(&["ok-button-error", "label-small"]);
            }
            Mode::ConfirmRepair => {
                self.confirm_label.set_label(&format!(
                    "Move its transcript into the project folder of {}? Nothing is overwritten.",
                    sessions::tilde(&s.cwd)
                ));
                self.confirm_go.set_label("Move");
                self.confirm_go
                    .set_css_classes(&["ok-button-surface", "label-small"]);
            }
            Mode::Actions => {}
        }
    }

    fn act(&self, action: LibAction) {
        (self.on_action)(SessionAction::Library(action));
        self.unfold(false);
    }

    /// Show or hide the actions under the row.
    fn unfold(&self, open: bool) {
        self.revealer.set_reveal_child(open);
        self.set_mode(Mode::Actions);
        self.copy.set_label("Copy command");
        self.fold.set_icon_name(if open {
            "pan-up-symbolic"
        } else {
            "pan-down-symbolic"
        });
        let name = if open {
            "Hide session actions"
        } else {
            "Show session actions"
        };
        mshell_common::a11y::name(&self.fold, name);
    }

    fn reference(&self) -> Option<SessionRef> {
        self.session.borrow().as_ref().map(SessionRef::of)
    }
}

/// A saved session.
pub(crate) struct SessionRow {
    root: gtk::Box,
    inner: Rc<RowInner>,
}

fn small_button(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.set_css_classes(&["ok-button-surface", "label-small"]);
    b
}

impl SessionRow {
    pub(crate) fn new(item: &RowItem, on_action: Rc<dyn Fn(SessionAction)>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("agents-session-row");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        root.append(&top);
        let main = gtk::Button::new();
        main.set_css_classes(&["ok-button-flat", "agents-session-main"]);
        main.set_hexpand(true);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let glyph = gtk::Image::new();
        glyph.set_valign(gtk::Align::Center);
        row.append(&glyph);
        let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        lamp.set_css_classes(&["agents-lamp", "busy"]);
        lamp.set_valign(gtk::Align::Center);
        row.append(&lamp);
        let block = gtk::Box::new(gtk::Orientation::Vertical, 2);
        block.set_hexpand(true);
        let title = gtk::Label::new(None);
        title.set_css_classes(&["label-medium"]);
        title.set_halign(gtk::Align::Start);
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        block.append(&title);
        let subtitle = gtk::Label::new(None);
        subtitle.set_css_classes(&["label-small", "dim-label"]);
        subtitle.set_halign(gtk::Align::Start);
        subtitle.set_xalign(0.0);
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
        block.append(&subtitle);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        chips.add_css_class("agents-session-chips");
        block.append(&chips);
        row.append(&block);
        let go = gtk::Image::from_icon_name("media-playback-start-symbolic");
        go.set_valign(gtk::Align::Center);
        go.add_css_class("agents-session-go");
        row.append(&go);
        main.set_child(Some(&row));
        top.append(&main);
        let fold = gtk::Button::from_icon_name("pan-down-symbolic");
        fold.set_css_classes(&["ok-button-flat", "agents-session-action"]);
        fold.set_valign(gtk::Align::Center);
        mshell_common::a11y::name(&fold, "Show session actions");
        top.append(&fold);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.add_css_class("agents-library-actions");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let copy = small_button("Copy command");
        copy.set_tooltip_text(Some("Copy `cd <folder> && claude --resume <id>`"));
        let rename = small_button("Rename");
        let lock = small_button("Lock");
        let repair = small_button("Repair");
        repair.set_tooltip_text(Some(
            "Move the transcript into the project folder of its working directory",
        ));
        let delete = small_button("Delete");
        for b in [&copy, &rename, &lock, &repair, &delete] {
            actions.append(b);
        }
        body.append(&actions);
        let rename_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.set_max_length(crate::agents::library::ops::TITLE_MAX as i32);
        entry.set_placeholder_text(Some("Session name"));
        mshell_common::a11y::name(&entry, "New session name");
        rename_box.append(&entry);
        let rename_cancel = small_button("Cancel");
        let rename_save = small_button("Save");
        rename_box.append(&rename_cancel);
        rename_box.append(&rename_save);
        body.append(&rename_box);
        let confirm = gtk::Box::new(gtk::Orientation::Vertical, 6);
        confirm.add_css_class("agents-confirm");
        let confirm_label = gtk::Label::new(None);
        confirm_label.set_css_classes(&["label-small"]);
        confirm_label.set_xalign(0.0);
        confirm_label.set_wrap(true);
        confirm.append(&confirm_label);
        let confirm_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        confirm_row.set_homogeneous(true);
        let confirm_cancel = small_button("Cancel");
        let confirm_go = small_button("Delete");
        confirm_row.append(&confirm_cancel);
        confirm_row.append(&confirm_go);
        confirm.append(&confirm_row);
        body.append(&confirm);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_child(Some(&body));
        root.append(&revealer);

        let inner = Rc::new(RowInner {
            glyph,
            lamp,
            title,
            subtitle,
            chips,
            go,
            main: main.clone(),
            fold: fold.clone(),
            revealer: revealer.clone(),
            actions,
            copy: copy.clone(),
            rename: rename.clone(),
            lock: lock.clone(),
            repair: repair.clone(),
            delete: delete.clone(),
            rename_box,
            entry: entry.clone(),
            confirm,
            confirm_label,
            confirm_go: confirm_go.clone(),
            session: RefCell::new(None),
            mode: Cell::new(Mode::Actions),
            on_action,
        });
        inner.set_mode(Mode::Actions);
        Self::connect(&inner, &main, &fold);
        let i = inner.clone();
        copy.connect_clicked(move |b| {
            let line = i
                .session
                .borrow()
                .as_ref()
                .and_then(LibSession::resume_line);
            if let Some(line) = line {
                b.clipboard().set_text(&line);
                b.set_label("Copied");
            }
        });
        let i = inner.clone();
        rename.connect_clicked(move |_| i.set_mode(Mode::Renaming));
        let i = inner.clone();
        delete.connect_clicked(move |_| i.set_mode(Mode::ConfirmDelete));
        let i = inner.clone();
        repair.connect_clicked(move |_| i.set_mode(Mode::ConfirmRepair));
        let i = inner.clone();
        lock.connect_clicked(move |_| {
            let s = i.session.borrow().clone();
            if let Some(s) = s {
                i.act(LibAction::Lock(SessionRef::of(&s), !s.locked));
            }
        });
        let i = inner.clone();
        rename_cancel.connect_clicked(move |_| i.set_mode(Mode::Actions));
        let i = inner.clone();
        confirm_cancel.connect_clicked(move |_| i.set_mode(Mode::Actions));
        let save = {
            let i = inner.clone();
            move || {
                let title = i.entry.text().to_string();
                if let Some(r) = i.reference()
                    && crate::agents::library::ops::clean_title(&title).is_ok()
                {
                    i.act(LibAction::Rename(r, title));
                }
            }
        };
        let s2 = save.clone();
        rename_save.connect_clicked(move |_| s2());
        entry.connect_activate(move |_| save());
        let i = inner.clone();
        confirm_go.connect_clicked(move |_| {
            let Some(r) = i.reference() else { return };
            match i.mode.get() {
                Mode::ConfirmDelete => i.act(LibAction::Delete(r)),
                Mode::ConfirmRepair => i.act(LibAction::Repair(r)),
                _ => {}
            }
        });
        let r = SessionRow { root, inner };
        r.update(item);
        r
    }

    fn connect(inner: &Rc<RowInner>, main: &gtk::Button, fold: &gtk::Button) {
        let i = inner.clone();
        main.connect_clicked(move |_| {
            let Some(s) = i.session.borrow().clone() else {
                return;
            };
            if s.no_resume_reason().is_some() {
                return;
            }
            if let Some(dir) = s.resume_dir() {
                (i.on_action)(SessionAction::Resume {
                    session_id: s.id.clone(),
                    cwd: dir,
                    config_dir: s.config_dir.clone(),
                });
            }
        });
        let i = inner.clone();
        fold.connect_clicked(move |_| i.unfold(!i.revealer.reveals_child()));
    }
}

/// `2026-11-05 21:51`, local time.
pub(crate) fn local_stamp(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// The dim line under a session's title.
pub(crate) fn subtitle(s: &LibSession, now_ms: i64, multi: bool) -> String {
    let mut parts = vec![
        local_stamp(s.mtime_ms),
        format!("{} ago", crate::agents::limits::span(now_ms - s.mtime_ms)),
    ];
    if !s.branch.is_empty() {
        parts.push(s.branch.clone());
    }
    if multi {
        parts.push(s.account.clone());
    }
    parts.join(" · ")
}

/// The chips under a session: `(text, extra class)`.
pub(crate) fn row_chips(s: &LibSession) -> Vec<(&'static str, &'static str)> {
    let mut v = Vec::new();
    if s.running {
        v.push(("running", "accent"));
    }
    match s.status {
        LibStatus::Ok => {}
        LibStatus::Moved(_) => v.push(("moved", "accent")),
        LibStatus::Unknown => v.push(("unknown", "")),
        _ => v.push((s.status.name(), "warn")),
    }
    if s.empty {
        v.push(("empty", ""));
    }
    if s.locked {
        v.push(("locked", ""));
    }
    v
}

impl KeyedRow for SessionRow {
    type Item = RowItem;
    type Key = String;

    fn key(item: &RowItem) -> String {
        item.session.key()
    }

    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn update(&self, item: &RowItem) {
        let s = &item.session;
        let i = &self.inner;
        i.title.set_label(&s.title);
        i.subtitle.set_label(&subtitle(s, item.now_ms, item.multi));
        match s.status.icon() {
            Some(icon) if !s.running => {
                i.glyph.set_icon_name(Some(icon));
                i.glyph
                    .set_css_classes(&["agents-library-status", s.status.name()]);
                i.glyph.set_tooltip_text(Some(&s.status.reason(&s.cwd)));
                i.glyph.set_visible(true);
            }
            _ => i.glyph.set_visible(false),
        }
        i.lamp.set_visible(s.running);
        while let Some(c) = i.chips.first_child() {
            i.chips.remove(&c);
        }
        let chips = row_chips(s);
        for (text, class) in &chips {
            let l = gtk::Label::new(Some(text));
            l.add_css_class("agents-chip");
            if !class.is_empty() {
                l.add_css_class(class);
            }
            i.chips.append(&l);
        }
        i.chips.set_visible(!chips.is_empty());
        let blocked = s.no_resume_reason();
        i.go.set_visible(blocked.is_none());
        let tip = match (&blocked, &s.status) {
            (Some(r), _) => r.clone(),
            (None, LibStatus::Moved(to)) => format!(
                "Resume in a new terminal, from {}",
                sessions::tilde(&to.to_string_lossy())
            ),
            (None, _) => "Resume in a new terminal".to_string(),
        };
        i.main.set_tooltip_text(Some(&tip));
        i.copy.set_sensitive(s.resume_line().is_some());
        i.rename.set_sensitive(s.can_rename());
        i.rename
            .set_tooltip_text((!s.can_rename()).then_some(if s.running {
                "Rename it in Claude Code with /rename while it runs"
            } else {
                "An orphaned session can't be renamed"
            }));
        i.lock.set_label(if s.locked { "Unlock" } else { "Lock" });
        i.lock.set_tooltip_text(Some(if s.locked {
            "Allow deleting it again"
        } else {
            "Protect it from deletion"
        }));
        i.repair.set_visible(s.status == LibStatus::Misplaced);
        i.repair.set_sensitive(s.can_repair());
        i.delete.set_sensitive(s.can_delete());
        i.delete.set_tooltip_text(match (s.running, s.locked) {
            (true, _) => Some("A running session can't be deleted"),
            (_, true) => Some("Locked — unlock it to delete"),
            _ => None,
        });
        // A confirm for an action that is no longer allowed folds away.
        let stale = match i.mode.get() {
            Mode::ConfirmDelete => !s.can_delete(),
            Mode::ConfirmRepair => !s.can_repair(),
            Mode::Renaming => !s.can_rename(),
            Mode::Actions => false,
        };
        i.session.replace(Some(s.clone()));
        if stale {
            i.set_mode(Mode::Actions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::library::tests::session;
    use std::path::PathBuf;

    #[test]
    fn chips_name_status_empty_and_lock() {
        let mut s = session("t", "/r/a", 1);
        assert!(row_chips(&s).is_empty());
        s.status = LibStatus::Moved(PathBuf::from("/r/b"));
        s.empty = true;
        s.locked = true;
        let got: Vec<_> = row_chips(&s).into_iter().map(|(t, _)| t).collect();
        assert_eq!(got, ["moved", "empty", "locked"]);
        s.status = LibStatus::Misplaced;
        s.running = true;
        assert_eq!(
            row_chips(&s)[..2],
            [("running", "accent"), ("misplaced", "warn")]
        );
    }

    #[test]
    fn subtitle_has_date_age_branch_and_account() {
        let mut s = session("t", "/r/a", 1_793_808_300_000);
        s.branch = "main".into();
        s.account = "work".into();
        let line = subtitle(&s, 1_793_808_300_000 + 3 * 3_600_000, true);
        assert!(line.ends_with(" · 3h 0m ago · main · work"), "{line}");
        assert!(line.starts_with("20"));
    }
}
