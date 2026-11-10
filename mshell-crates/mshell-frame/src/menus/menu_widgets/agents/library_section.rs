//! The Sessions page's session manager: every saved Claude Code session
//! on disk, grouped by project folder, with a status glyph for the ones
//! that can't resume as they are (moved, orphan, misplaced, unknown),
//! sort (date / name / count; the active one again reverses), filters
//! (moved, orphaned, empty) and the bulk actions (Delete empty, Leftovers),
//! each behind an inline confirm. Rows: [`super::library_rows`].

use super::keyed_rows::KeyedList;
use super::library_rows::{GroupItem, GroupRow};
use super::sessions_page::SessionAction;
use crate::agents::library::leftovers::Leftover;
use crate::agents::library::view::{self, Filter, Sort, SortKey};
use crate::agents::library::{LibAction, LibrarySnapshot};
use crate::common_widgets::segmented::{Segmented, SegmentedStyle};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use std::rc::Rc;

/// What the section's own controls ask for; the menu hands it back to
/// [`LibrarySection::handle`].
#[derive(Debug, Clone)]
pub(crate) enum LibUi {
    Sort(SortKey),
    Filter(Filter),
    AskDeleteEmpty,
    /// The leftovers were listed (off the GTK thread).
    Leftovers(Vec<Leftover>),
    Confirm,
    Cancel,
}

/// The bulk action waiting for a confirm.
#[derive(Debug, Clone)]
enum Pending {
    DeleteEmpty(usize),
    Leftovers(Vec<Leftover>),
}

const SORTS: [SortKey; 3] = [SortKey::Date, SortKey::Name, SortKey::Count];
const SORT_LABELS: [&str; 3] = ["Date", "Name", "Count"];

pub(crate) struct LibrarySection {
    title: gtk::Label,
    tools: gtk::Box,
    sort_row: Segmented,
    moved: gtk::Button,
    broken: gtk::Button,
    empty: gtk::Button,
    delete_empty: gtk::Button,
    confirm: gtk::Box,
    confirm_label: gtk::Label,
    confirm_list: gtk::Label,
    confirm_go: gtk::Button,
    status: gtk::Label,
    groups: KeyedList<GroupRow>,
    snap: Option<LibrarySnapshot>,
    sort: Sort,
    filter: Filter,
    query: String,
    pending: Option<Pending>,
    on_action: Rc<dyn Fn(SessionAction)>,
}

fn small_button(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.set_css_classes(&["ok-button-surface", "label-small"]);
    b
}

/// `1.2 MB`-style size.
fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// The confirm text for removing `items`: a count line and one line per
/// leftover.
pub(crate) fn leftover_text(items: &[Leftover]) -> (String, String) {
    use crate::agents::library::leftovers::LeftoverKind;
    let total: u64 = items.iter().map(|l| l.bytes).sum();
    let head = format!(
        "Remove {} leftover{} of deleted sessions ({}), untouched for a day or more?",
        items.len(),
        if items.len() == 1 { "" } else { "s" },
        bytes(total)
    );
    let lines = items
        .iter()
        .map(|l| {
            let what = match l.kind {
                LeftoverKind::Project => "project folder with no session",
                _ => "no transcript",
            };
            format!(
                "{} · {} · {}",
                crate::agents::sessions::tilde(&l.path.to_string_lossy()),
                bytes(l.bytes),
                what
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    (head, lines)
}

impl LibrarySection {
    pub(crate) fn new(
        container: &gtk::Box,
        on_action: Rc<dyn Fn(SessionAction)>,
        on_ui: Rc<dyn Fn(LibUi)>,
    ) -> Self {
        let title = gtk::Label::new(Some("All sessions"));
        title.add_css_class("section-label-menu-widget");
        title.set_halign(gtk::Align::Start);
        container.append(&title);
        let tools = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let ui = on_ui.clone();
        let sort_row = Segmented::new(&SORT_LABELS, 0, SegmentedStyle::ButtonRow, move |i| {
            ui(LibUi::Sort(SORTS[i]))
        });
        for (b, tip) in sort_row.buttons().iter().zip([
            "Newest first (again: oldest first)",
            "A to Z (again: Z to A)",
            "Projects with the most sessions first",
        ]) {
            b.set_tooltip_text(Some(tip));
        }
        tools.append(sort_row.root());
        let filters = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        filters.set_homogeneous(true);
        let filter_button = |f: Filter, tip: &str| {
            let b = small_button("");
            b.set_tooltip_text(Some(tip));
            let ui = on_ui.clone();
            b.connect_clicked(move |_| ui(LibUi::Filter(f)));
            filters.append(&b);
            b
        };
        let moved = filter_button(
            Filter::Moved,
            "Only sessions whose folder was renamed or moved (they resume from the new folder)",
        );
        let broken = filter_button(
            Filter::Broken,
            "Only orphan and misplaced sessions (they can't resume as they are)",
        );
        let empty = filter_button(Filter::Empty, "Only sessions nothing was ever sent in");
        tools.append(&filters);
        let bulk = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        bulk.set_homogeneous(true);
        let delete_empty = small_button("Delete empty");
        delete_empty.set_tooltip_text(Some(
            "Delete every empty session (running and locked ones are kept)",
        ));
        let ui = on_ui.clone();
        delete_empty.connect_clicked(move |_| ui(LibUi::AskDeleteEmpty));
        bulk.append(&delete_empty);
        let leftovers = small_button("Leftovers…");
        leftovers.set_tooltip_text(Some(
            "Find session-env / file-history folders without a transcript and project folders with no session",
        ));
        let act = on_action.clone();
        leftovers.connect_clicked(move |_| act(SessionAction::FindLeftovers));
        bulk.append(&leftovers);
        tools.append(&bulk);
        container.append(&tools);

        let confirm = gtk::Box::new(gtk::Orientation::Vertical, 6);
        confirm.add_css_class("agents-confirm");
        let confirm_label = gtk::Label::new(None);
        confirm_label.set_css_classes(&["label-medium-bold"]);
        confirm_label.set_xalign(0.0);
        confirm_label.set_wrap(true);
        confirm.append(&confirm_label);
        let confirm_list = gtk::Label::new(None);
        confirm_list.set_css_classes(&["label-small", "dim-label"]);
        confirm_list.set_xalign(0.0);
        confirm_list.set_wrap(true);
        confirm_list.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        confirm_list.set_selectable(true);
        confirm.append(&confirm_list);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.set_homogeneous(true);
        let cancel = small_button("Cancel");
        let ui = on_ui.clone();
        cancel.connect_clicked(move |_| ui(LibUi::Cancel));
        let confirm_go = gtk::Button::with_label("Delete");
        confirm_go.set_css_classes(&["ok-button-error", "label-small"]);
        let ui = on_ui.clone();
        confirm_go.connect_clicked(move |_| ui(LibUi::Confirm));
        row.append(&cancel);
        row.append(&confirm_go);
        confirm.append(&row);
        container.append(&confirm);

        let status = gtk::Label::new(Some("Loading…"));
        status.set_css_classes(&["label-small", "dim-label"]);
        status.set_halign(gtk::Align::Start);
        status.set_xalign(0.0);
        status.set_wrap(true);
        container.append(&status);
        let groups_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        container.append(&groups_box);
        let mut s = LibrarySection {
            title,
            tools,
            sort_row,
            moved,
            broken,
            empty,
            delete_empty,
            confirm,
            confirm_label,
            confirm_list,
            confirm_go,
            status,
            groups: KeyedList::new(groups_box),
            snap: None,
            sort: Sort::default(),
            filter: Filter::All,
            query: String::new(),
            pending: None,
            on_action,
        };
        s.render(0);
        s
    }

    /// Show a fresh scan.
    pub(crate) fn set_snapshot(&mut self, snap: LibrarySnapshot, now_ms: i64) {
        self.snap = Some(snap);
        self.render(now_ms);
    }

    /// Filter the list by the page's search text too.
    pub(crate) fn set_query(&mut self, query: &str, now_ms: i64) {
        if self.query != query {
            self.query = query.to_string();
            self.render(now_ms);
        }
    }

    pub(crate) fn handle(&mut self, msg: LibUi, now_ms: i64) {
        match msg {
            LibUi::Sort(k) => self.sort = self.sort.choose(k),
            LibUi::Filter(f) => {
                self.filter = if self.filter == f { Filter::All } else { f };
            }
            LibUi::AskDeleteEmpty => {
                let n = self
                    .snap
                    .as_ref()
                    .map_or(0, |s| view::counts(&s.sessions).deletable_empty);
                self.pending = (n > 0).then_some(Pending::DeleteEmpty(n));
            }
            LibUi::Leftovers(items) => {
                self.pending = Some(Pending::Leftovers(items));
            }
            LibUi::Confirm => match self.pending.take() {
                Some(Pending::DeleteEmpty(_)) => {
                    (self.on_action)(SessionAction::Library(LibAction::DeleteEmpty));
                }
                Some(Pending::Leftovers(items)) if !items.is_empty() => {
                    (self.on_action)(SessionAction::Library(LibAction::RemoveLeftovers(items)));
                }
                _ => {}
            },
            LibUi::Cancel => self.pending = None,
        }
        self.render(now_ms);
    }

    fn render_confirm(&self) {
        match &self.pending {
            None => self.confirm.set_visible(false),
            Some(Pending::DeleteEmpty(n)) => {
                self.confirm_label.set_label(&format!(
                    "Delete {n} empty session{}? Running and locked ones are kept.",
                    if *n == 1 { "" } else { "s" }
                ));
                self.confirm_list.set_visible(false);
                self.confirm_go.set_label("Delete");
                self.confirm_go.set_visible(true);
                self.confirm.set_visible(true);
            }
            Some(Pending::Leftovers(items)) if items.is_empty() => {
                self.confirm_label
                    .set_label("No leftovers: nothing older than a day is left behind.");
                self.confirm_list.set_visible(false);
                self.confirm_go.set_visible(false);
                self.confirm.set_visible(true);
            }
            Some(Pending::Leftovers(items)) => {
                let (head, lines) = leftover_text(items);
                self.confirm_label.set_label(&head);
                self.confirm_list.set_label(&lines);
                self.confirm_list.set_visible(true);
                self.confirm_go.set_label("Remove");
                self.confirm_go.set_visible(true);
                self.confirm.set_visible(true);
            }
        }
    }

    fn render(&mut self, now_ms: i64) {
        self.render_confirm();
        let Some(snap) = &self.snap else {
            self.tools.set_visible(false);
            self.status.set_label("Loading…");
            self.status.set_visible(true);
            return;
        };
        let c = view::counts(&snap.sessions);
        self.title.set_label(&format!("All sessions · {}", c.all));
        self.tools.set_visible(c.all > 0);
        for (i, k) in SORTS.iter().enumerate() {
            let arrow = if self.sort.key == *k && self.sort.reversed {
                " ↑"
            } else {
                ""
            };
            self.sort_row
                .set_label(i, &format!("{}{arrow}", SORT_LABELS[i]));
        }
        self.sort_row
            .set_active(SORTS.iter().position(|k| *k == self.sort.key).unwrap_or(0));
        for (b, f, label, n) in [
            (&self.moved, Filter::Moved, "Moved", c.moved),
            (&self.broken, Filter::Broken, "Orphaned", c.broken),
            (&self.empty, Filter::Empty, "Empty", c.empty),
        ] {
            b.set_label(&format!("{label} · {n}"));
            b.set_css_classes(if self.filter == f {
                &["ok-button-surface", "label-small", "selected"]
            } else {
                &["ok-button-surface", "label-small"]
            });
            b.set_sensitive(n > 0 || self.filter == f);
        }
        self.delete_empty.set_sensitive(c.deletable_empty > 0);
        let groups = view::groups(&snap.sessions, self.filter, &self.query, self.sort);
        let auto_open = self.filter != Filter::All || !self.query.is_empty();
        let items: Vec<GroupItem> = groups
            .into_iter()
            .map(|group| GroupItem {
                group,
                now_ms,
                multi: snap.multi,
                auto_open,
            })
            .collect();
        self.status.set_visible(items.is_empty());
        self.status.set_label(if c.all == 0 {
            "No saved Claude Code sessions"
        } else {
            "No session matches"
        });
        let on_action = self.on_action.clone();
        self.groups
            .sync(&items, |item| GroupRow::new(item, on_action.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::library::leftovers::LeftoverKind;
    use std::path::PathBuf;

    #[test]
    fn leftover_confirm_lists_each_item_and_the_total() {
        let item = |kind, path: &str, bytes| Leftover {
            kind,
            config_dir: PathBuf::from("/h/.claude"),
            name: String::new(),
            path: PathBuf::from(path),
            bytes,
            mtime_ms: 0,
        };
        let (head, lines) = leftover_text(&[
            item(LeftoverKind::SessionEnv, "/h/.claude/session-env/x", 1_500),
            item(LeftoverKind::Project, "/h/.claude/projects/-w", 0),
        ]);
        assert_eq!(
            head,
            "Remove 2 leftovers of deleted sessions (1.5 KB), untouched for a day or more?"
        );
        let lines: Vec<&str> = lines.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("x · 1.5 KB · no transcript"));
        assert!(lines[1].ends_with("-w · 0 B · project folder with no session"));
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(2_500_000), "2.5 MB");
    }
}
