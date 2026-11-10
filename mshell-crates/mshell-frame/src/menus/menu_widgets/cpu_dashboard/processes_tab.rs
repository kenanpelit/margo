//! Processes tab of the CPU dashboard: a sortable, searchable process table
//! read from `/proc` (CPU, RAM, swap, disk IO), optionally grouped by the
//! margo window each process descends from, with a confirmed
//! terminate / force-kill and a pause / resume (SIGSTOP / SIGCONT; a
//! paused row says so and offers Resume). Polls every 2 s only while the menu is revealed
//! AND this tab is the visible one; the `/proc` sweep runs on a blocking
//! thread (DESIGN.md §13.4 — never on the GTK thread).

use super::keyed_rows::KeyedRows;
use super::procs::{
    KillResult, ProcRow, ProcSnapshot, ProcSort, ProcTracker, group_by_window, pause_signal,
    read_snapshot, row_matches, send_signal, sort_rows,
};
use super::ui;
use crate::common_widgets::segmented::{Segmented, SegmentedStyle};
use crate::menus::reveal_poll::RevealPoll;
use mshell_services::margo_service;
use mshell_utils::units;
use relm4::gtk::prelude::{
    AccessibleExtManual, BoxExt, ButtonExt, EditableExt, OrientableExt, ToggleButtonExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Rows rendered per poll. The table answers "what's busy", so the long
/// idle tail is cut; search reaches anything beyond it.
const MAX_ROWS: usize = 60;
/// Processes shown under each window group.
const MAX_GROUP_MEMBERS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingKill {
    pid: i32,
    /// Start time from the snapshot the user picked the row in.
    start: u64,
    name: String,
}

pub(crate) struct ProcessesTabModel {
    poll: RevealPoll,
    in_flight: bool,
    tracker: ProcTracker,
    last_sweep: Option<Instant>,
    rows: Vec<ProcRow>,
    sort: ProcSort,
    query: String,
    group: bool,
    pending: Option<PendingKill>,
    /// Result line under the confirm bar ("Sent SIGTERM to …", errors).
    status: String,
    /// EPERM on the last attempt: offer the pkexec path (pid, start, signal).
    needs_admin: Option<(i32, u64, &'static str)>,
    list: gtk::ListBox,
    /// The rows on screen, kept across polls (only changed rows rebuild).
    shown: KeyedRows<RowKey, LiveRow>,
    /// The sort keys, in `sort_row` order.
    sorts: Vec<ProcSort>,
    sort_row: Segmented,
}

#[derive(Debug)]
pub(crate) enum ProcessesTabInput {
    /// Menu revealed AND this tab visible.
    SetActive(bool),
    Poll,
    Sort(ProcSort),
    Search(String),
    Group(bool),
    /// End this process: pid, start time, name.
    AskKill(i32, u64, String),
    CancelKill,
    /// Send `SIGTERM` (`false`) or `SIGKILL` (`true`) to the pending pid.
    ConfirmKill(bool),
    /// Re-send the refused signal through `pkexec kill`.
    AdminKill,
    /// Pause (`true`, SIGSTOP) or resume (`false`, SIGCONT): pid, start
    /// time, name. Reversible, so no confirm step.
    Pause(i32, u64, String, bool),
}

#[derive(Debug)]
pub(crate) enum ProcessesTabCmd {
    Snapshot(ProcSnapshot),
    AdminDone(Result<(), String>),
}

#[relm4::component(pub(crate))]
impl Component for ProcessesTabModel {
    type CommandOutput = ProcessesTabCmd;
    type Input = ProcessesTabInput;
    type Output = ();
    type Init = ();

    view! {
        #[root]
        gtk::Box {
            add_css_class: "cpu-dashboard-tab",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            gtk::SearchEntry {
                add_css_class: "menu-search",
                set_placeholder_text: Some("Search name, command or PID…"),
                connect_search_changed[sender] => move |e| {
                    sender.input(ProcessesTabInput::Search(e.text().to_string()));
                },
            },

            // Sort segmented row + group toggle.
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                #[local_ref]
                sort_row -> gtk::Box {
                    add_css_class: "cpu-dashboard-sort-row",
                    set_hexpand: true,
                },
                gtk::ToggleButton {
                    set_css_classes: &["ok-button-surface", "label-small"],
                    set_label: "By app",
                    set_tooltip_text: Some("Group processes under the app window they belong to"),
                    connect_toggled[sender] => move |b| {
                        if b.is_active() { b.add_css_class("selected"); } else { b.remove_css_class("selected"); }
                        sender.input(ProcessesTabInput::Group(b.is_active()));
                    },
                },
            },

            // Inline confirm (§8b danger: irreversible actions confirm first).
            gtk::Box {
                add_css_class: "cpu-dashboard-confirm",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,
                #[watch]
                set_visible: model.pending.is_some(),
                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_label: &model.pending.as_ref().map(|p| format!("End {} ({})?", p.name, p.pid)).unwrap_or_default(),
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_homogeneous: true,
                    gtk::Button {
                        set_css_classes: &["ok-button-surface", "label-small"],
                        set_label: "Cancel",
                        connect_clicked => ProcessesTabInput::CancelKill,
                    },
                    gtk::Button {
                        set_css_classes: &["ok-button-surface", "label-small"],
                        set_label: "Terminate",
                        set_tooltip_text: Some("SIGTERM: ask the process to quit"),
                        connect_clicked => ProcessesTabInput::ConfirmKill(false),
                    },
                    gtk::Button {
                        set_css_classes: &["ok-button-error", "label-small"],
                        set_label: "Force kill",
                        set_tooltip_text: Some("SIGKILL: stop it immediately (unsaved work is lost)"),
                        connect_clicked => ProcessesTabInput::ConfirmKill(true),
                    },
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                #[watch]
                set_visible: !model.status.is_empty(),
                gtk::Label {
                    set_css_classes: &["label-small", "dim-label"],
                    set_hexpand: true,
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_label: &model.status,
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "label-small"],
                    set_label: "Use admin",
                    set_tooltip_text: Some("Send the signal through pkexec"),
                    #[watch]
                    set_visible: model.needs_admin.is_some(),
                    connect_clicked => ProcessesTabInput::AdminKill,
                },
            },

            #[name = "fill_list"]
            gtk::ScrolledWindow {
                set_min_content_height: 0,
                set_max_content_height: 380,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,
                #[local_ref]
                list -> gtk::ListBox {
                    set_css_classes: &["ok-boxed-list", "compact", "cpu-dashboard-list"],
                    set_selection_mode: gtk::SelectionMode::None,
                },
            },

            gtk::Label {
                set_css_classes: &["label-small", "dim-label"],
                set_xalign: 0.0,
                set_wrap: true,
                #[watch]
                set_label: &format!(
                    "{} processes · CPU % is per core · disk IO only for your own processes",
                    model.rows.len()
                ),
            },
        }
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let s = sender.clone();
        let poll = RevealPoll::new(POLL_INTERVAL, move || {
            s.input_sender().send(ProcessesTabInput::Poll).is_ok()
        });
        // Built hidden (the Overview tab shows first); SetActive starts it.
        poll.set_revealed(false);

        let list = gtk::ListBox::new();
        const SORTS: [(ProcSort, &str, &str); 4] = [
            (ProcSort::Cpu, "CPU", "Sort by CPU"),
            (ProcSort::Mem, "RAM", "Sort by memory"),
            (ProcSort::Swap, "Swap", "Sort by swap"),
            (ProcSort::Io, "Disk", "Sort by disk read + write rate"),
        ];
        let sorts: Vec<ProcSort> = SORTS.iter().map(|(s, _, _)| *s).collect();
        let labels: Vec<&str> = SORTS.iter().map(|(_, l, _)| *l).collect();
        let active = sorts
            .iter()
            .position(|s| *s == ProcSort::default())
            .unwrap_or(0);
        let s = sender.clone();
        let by_index = sorts.clone();
        let sort_segments = Segmented::new(&labels, active, SegmentedStyle::ButtonRow, move |i| {
            s.input(ProcessesTabInput::Sort(by_index[i]));
        });
        for (b, (_, _, tip)) in sort_segments.buttons().iter().zip(SORTS) {
            b.set_tooltip_text(Some(tip));
        }
        let sort_row = sort_segments.root().clone();

        let mut model = ProcessesTabModel {
            poll,
            in_flight: false,
            tracker: ProcTracker::default(),
            last_sweep: None,
            rows: Vec::new(),
            sort: ProcSort::default(),
            query: String::new(),
            group: false,
            pending: None,
            status: String::new(),
            needs_admin: None,
            list: list.clone(),
            shown: KeyedRows::default(),
            sorts,
            sort_row: sort_segments,
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 380);
        model.render(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            ProcessesTabInput::SetActive(on) => {
                self.poll.set_revealed(on);
                if on {
                    sender.input(ProcessesTabInput::Poll);
                }
            }
            ProcessesTabInput::Poll => {
                if self.in_flight {
                    return;
                }
                self.in_flight = true;
                sender.oneshot_command(async {
                    let snap = tokio::task::spawn_blocking(|| read_snapshot(Path::new("/proc")))
                        .await
                        .unwrap_or_default();
                    ProcessesTabCmd::Snapshot(snap)
                });
            }
            ProcessesTabInput::Sort(sort) => {
                self.sort = sort;
                if let Some(i) = self.sorts.iter().position(|s| *s == sort) {
                    self.sort_row.set_active(i);
                }
                self.render(&sender);
            }
            ProcessesTabInput::Search(q) => {
                self.query = q;
                self.render(&sender);
            }
            ProcessesTabInput::Group(on) => {
                self.group = on;
                self.render(&sender);
            }
            ProcessesTabInput::AskKill(pid, start, name) => {
                self.pending = Some(PendingKill { pid, start, name });
                self.status.clear();
                self.needs_admin = None;
            }
            ProcessesTabInput::CancelKill => self.pending = None,
            ProcessesTabInput::ConfirmKill(force) => {
                let Some(p) = self.pending.take() else { return };
                let (sig, sig_name) = if force {
                    (libc::SIGKILL, "KILL")
                } else {
                    (libc::SIGTERM, "TERM")
                };
                self.needs_admin = None;
                self.status = match send_signal(p.pid, p.start, sig) {
                    Err(why) => why.to_string(),
                    Ok(KillResult::Sent) => format!("Sent SIG{sig_name} to {} ({})", p.name, p.pid),
                    Ok(KillResult::Gone) => format!("{} ({}) has already exited", p.name, p.pid),
                    Ok(KillResult::NotPermitted) => {
                        self.needs_admin = Some((p.pid, p.start, sig_name));
                        format!("{} ({}) belongs to another user", p.name, p.pid)
                    }
                    Ok(KillResult::Failed(e)) => format!("kill failed (errno {e})"),
                };
                sender.input(ProcessesTabInput::Poll);
            }
            ProcessesTabInput::Pause(pid, start, name, pause) => {
                self.pending = None;
                self.needs_admin = None;
                let (verb, sig_name) = if pause {
                    ("Paused", "STOP")
                } else {
                    ("Resumed", "CONT")
                };
                self.status = match pause_signal(pid, start, pause) {
                    Err(why) => why,
                    Ok(KillResult::Sent) => format!("{verb} {name} ({pid})"),
                    Ok(KillResult::Gone) => format!("{name} ({pid}) has already exited"),
                    Ok(KillResult::NotPermitted) => {
                        self.needs_admin = Some((pid, start, sig_name));
                        format!("{name} ({pid}) belongs to another user")
                    }
                    Ok(KillResult::Failed(e)) => format!("SIG{sig_name} failed (errno {e})"),
                };
                sender.input(ProcessesTabInput::Poll);
            }
            ProcessesTabInput::AdminKill => {
                let Some((pid, start, sig)) = self.needs_admin.take() else {
                    return;
                };
                self.status = format!("Waiting for authorization to signal {pid}…");
                sender.oneshot_command(async move {
                    ProcessesTabCmd::AdminDone(super::pkexec_kill(pid, start, sig).await)
                });
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            ProcessesTabCmd::Snapshot(snap) => {
                self.in_flight = false;
                let now = Instant::now();
                let elapsed = self
                    .last_sweep
                    .map(|t| now.duration_since(t).as_secs_f64())
                    .unwrap_or(0.0);
                self.last_sweep = Some(now);
                self.rows = self.tracker.update(&snap, elapsed);
                self.render(&sender);
            }
            ProcessesTabCmd::AdminDone(res) => {
                self.status = match res {
                    Ok(()) => "Signal sent".to_string(),
                    Err(e) => e,
                };
                sender.input(ProcessesTabInput::Poll);
            }
        }
    }
}

impl ProcessesTabModel {
    fn render(&mut self, sender: &ComponentSender<Self>) {
        let clients: Option<Vec<(i32, String)>> = self.group.then(|| {
            margo_service()
                .clients
                .get()
                .iter()
                .map(|c| (c.pid.get(), c.class.get()))
                .filter(|(pid, _)| *pid > 0)
                .collect()
        });
        let views = row_views(&self.rows, &self.query, self.sort, clients.as_deref());
        self.shown.sync(
            &self.list,
            views,
            |_, v| live_row(v, sender),
            update_row,
            |r| &r.row,
        );
    }
}

/// What one list row shows, keyed so a poll updates the same row.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RowKey {
    Placeholder,
    Group(Option<String>),
    /// pid + start time: a reused pid is a new row (and a new kill target).
    Proc(i32, u64),
}

#[derive(Debug, Clone, PartialEq)]
enum RowView {
    Placeholder(&'static str),
    Group {
        title: String,
        subtitle: String,
        cpu: String,
        rss: String,
    },
    Proc {
        pid: i32,
        start: u64,
        name: String,
        subtitle: String,
        cpu: String,
        rss: String,
        cmdline: String,
        /// Listed under a window group (indented).
        member: bool,
        /// Stopped by SIGSTOP: dimmed, "Paused" subtitle, Resume button.
        paused: bool,
    },
}

/// The rows to show, in order: filtered by `query`, sorted, capped at
/// [`MAX_ROWS`]; grouped under their window when `clients` is given.
fn row_views(
    all: &[ProcRow],
    query: &str,
    sort: ProcSort,
    clients: Option<&[(i32, String)]>,
) -> Vec<(RowKey, RowView)> {
    let mut rows: Vec<ProcRow> = all
        .iter()
        .filter(|r| row_matches(r, query))
        .cloned()
        .collect();
    sort_rows(&mut rows, sort);
    if rows.is_empty() {
        let text = if all.is_empty() {
            "Reading processes…"
        } else {
            "No process matches the search"
        };
        return vec![(RowKey::Placeholder, RowView::Placeholder(text))];
    }
    let Some(clients) = clients else {
        return rows
            .iter()
            .take(MAX_ROWS)
            .map(|r| (RowKey::Proc(r.pid, r.start), proc_view(r, false)))
            .collect();
    };
    let mut out = Vec::new();
    for g in group_by_window(&rows, clients, sort) {
        if out.len() >= MAX_ROWS {
            break;
        }
        let subtitle = format!(
            "{} process{} · {} swap",
            g.members.len(),
            if g.members.len() == 1 { "" } else { "es" },
            units::kib(g.swap_kb)
        );
        out.push((
            RowKey::Group(g.app.clone()),
            RowView::Group {
                title: g
                    .app
                    .clone()
                    .unwrap_or_else(|| "Other processes".to_string()),
                subtitle,
                cpu: format!("{:.1}%", g.cpu),
                rss: units::kib(g.rss_kb),
            },
        ));
        for i in g.members.iter().take(MAX_GROUP_MEMBERS) {
            if let Some(r) = rows.get(*i) {
                out.push((RowKey::Proc(r.pid, r.start), proc_view(r, true)));
            }
        }
    }
    out
}

fn proc_view(r: &ProcRow, member: bool) -> RowView {
    let mut sub = if r.paused {
        format!("Paused · {}", r.pid)
    } else {
        format!("{}", r.pid)
    };
    if r.swap_kb > 0 {
        sub.push_str(&format!(" · swap {}", units::kib(r.swap_kb)));
    }
    if let Some((rd, wr)) = r.io_bps
        && rd + wr > 0
    {
        sub.push_str(&format!(
            " · R {} W {}",
            units::rate(rd as f64),
            units::rate(wr as f64)
        ));
    }
    RowView::Proc {
        pid: r.pid,
        start: r.start,
        name: r.name.clone(),
        subtitle: sub,
        cpu: format!("{:.1}%", r.cpu),
        rss: units::kib(r.rss_kb),
        cmdline: r.cmdline.clone(),
        member,
        paused: r.paused,
    }
}

/// One list row's widgets, kept across polls.
struct LiveRow {
    row: gtk::ListBoxRow,
    kind: LiveKind,
}

enum LiveKind {
    Placeholder(gtk::Label),
    Item {
        body: gtk::Box,
        text: ui::LiveTitle,
        cpu: gtk::Label,
        rss: gtk::Label,
        /// Process rows: the ✕ button and who it ends. The click reads the
        /// row's current target, so it always matches what the row shows.
        kill: Option<(gtk::Button, Rc<RefCell<KillTarget>>)>,
        /// Process rows: the pause / resume button and whether the row is
        /// paused right now (the click sends the opposite).
        pause: Option<(gtk::Button, Rc<Cell<bool>>)>,
    },
}

/// Who a row's ✕ ends: pid, start time, name.
type KillTarget = (i32, u64, String);

type KillButton = (gtk::Button, Rc<RefCell<KillTarget>>);
type PauseButton = (gtk::Button, Rc<Cell<bool>>);

fn live_row(view: &RowView, sender: &ComponentSender<ProcessesTabModel>) -> LiveRow {
    let item = |kill: Option<KillButton>, pause: Option<PauseButton>| {
        let text = ui::live_title_block();
        let cpu = ui::value_label("", 6, "");
        let rss = ui::value_label("", 7, "");
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .build();
        body.append(&text.block);
        body.append(&cpu);
        body.append(&rss);
        if let Some((b, _)) = &pause {
            body.append(b);
        }
        if let Some((b, _)) = &kill {
            body.append(b);
        }
        let row = ui::plain_row(&body);
        LiveRow {
            row,
            kind: LiveKind::Item {
                body,
                text,
                cpu,
                rss,
                kill,
                pause,
            },
        }
    };
    match view {
        RowView::Placeholder(_) => {
            let label = gtk::Label::builder()
                .css_classes(["label-small", "dim-label"])
                .xalign(0.0)
                .wrap(true)
                .build();
            LiveRow {
                row: ui::plain_row(&label),
                kind: LiveKind::Placeholder(label),
            }
        }
        RowView::Group { .. } => {
            let r = item(None, None);
            if let LiveKind::Item { body, .. } = &r.kind {
                body.add_css_class("cpu-dashboard-group-head");
            }
            r
        }
        RowView::Proc {
            pid,
            start,
            name,
            paused,
            ..
        } => {
            let target = Rc::new(RefCell::new((*pid, *start, name.clone())));
            let button = ui::icon_button("close-symbolic", &kill_name(*pid, name));
            let s = sender.clone();
            let t = target.clone();
            button.connect_clicked(move |_| {
                let (pid, start, name) = t.borrow().clone();
                s.input(ProcessesTabInput::AskKill(pid, start, name));
            });

            let is_paused = Rc::new(Cell::new(*paused));
            let (icon, label) = pause_button_face(*pid, name, *paused);
            let pause = ui::icon_button(icon, &label);
            pause.set_tooltip_text(Some(&label));
            let s = sender.clone();
            let t = target.clone();
            let p = is_paused.clone();
            pause.connect_clicked(move |_| {
                let (pid, start, name) = t.borrow().clone();
                s.input(ProcessesTabInput::Pause(pid, start, name, !p.get()));
            });
            item(Some((button, target)), Some((pause, is_paused)))
        }
    }
}

fn kill_name(pid: i32, name: &str) -> String {
    format!("End {name} ({pid})")
}

/// Icon + accessible name of a row's pause button: Resume on a paused
/// row, Pause otherwise.
fn pause_button_face(pid: i32, name: &str, paused: bool) -> (&'static str, String) {
    if paused {
        (
            "media-playback-start-symbolic",
            format!("Resume {name} ({pid})"),
        )
    } else {
        (
            "media-playback-pause-symbolic",
            format!("Pause {name} ({pid})"),
        )
    }
}

/// Bring a kept row up to date, touching only what changed.
fn update_row(r: &LiveRow, view: &RowView) {
    match (&r.kind, view) {
        (LiveKind::Placeholder(label), RowView::Placeholder(text)) => ui::set_text(label, text),
        (
            LiveKind::Item { text, cpu, rss, .. },
            RowView::Group {
                title,
                subtitle,
                cpu: c,
                rss: m,
            },
        ) => {
            ui::set_title(text, title, subtitle);
            ui::set_text(cpu, c);
            ui::set_text(rss, m);
        }
        (
            LiveKind::Item {
                body,
                text,
                cpu,
                rss,
                kill,
                pause,
            },
            RowView::Proc {
                pid,
                start,
                name,
                subtitle,
                cpu: c,
                rss: m,
                cmdline,
                member,
                paused,
            },
        ) => {
            ui::set_title(text, name, subtitle);
            ui::set_class(&text.title, "dim-label", *paused);
            if let Some((button, is_paused)) = pause {
                let renamed = kill
                    .as_ref()
                    .is_some_and(|(_, target)| target.borrow().2 != *name);
                if is_paused.get() != *paused || renamed {
                    is_paused.set(*paused);
                    let (icon, label) = pause_button_face(*pid, name, *paused);
                    button.set_icon_name(icon);
                    button.update_property(&[gtk::accessible::Property::Label(&label)]);
                    button.set_tooltip_text(Some(&label));
                }
            }
            ui::set_text(cpu, c);
            ui::set_text(rss, m);
            ui::set_class(&r.row, "cpu-dashboard-group-member", *member);
            let tip = (!cmdline.is_empty()).then_some(cmdline.as_str());
            if body.tooltip_text().as_deref() != tip {
                body.set_tooltip_text(tip);
            }
            if let Some((button, target)) = kill
                && target.borrow().2 != *name
            {
                *target.borrow_mut() = (*pid, *start, name.clone());
                let label = kill_name(*pid, name);
                button.update_property(&[gtk::accessible::Property::Label(&label)]);
                button.set_tooltip_text(Some(&label));
            }
        }
        // A key always maps to the same view kind (RowKey ↔ RowView).
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: i32, ppid: i32, name: &str, cpu: f32) -> ProcRow {
        ProcRow {
            pid,
            ppid,
            uid: 1000,
            name: name.to_string(),
            cmdline: format!("/usr/bin/{name}"),
            cpu,
            rss_kb: 1024,
            swap_kb: 0,
            io_bps: None,
            start: u64::try_from(pid).unwrap_or(0) * 10,
            paused: false,
        }
    }

    fn keys(v: &[(RowKey, RowView)]) -> Vec<RowKey> {
        v.iter().map(|(k, _)| k.clone()).collect()
    }

    #[test]
    fn flat_rows_are_keyed_by_pid_in_sort_order() {
        let rows = vec![proc(10, 1, "a", 1.0), proc(20, 1, "b", 5.0)];
        let v = row_views(&rows, "", ProcSort::Cpu, None);
        assert_eq!(keys(&v), [RowKey::Proc(20, 200), RowKey::Proc(10, 100)]);
        // Same pids, other values: same keys (the rows are reused).
        let again = vec![proc(10, 1, "a", 9.0), proc(20, 1, "b", 0.5)];
        let v2 = row_views(&again, "", ProcSort::Cpu, None);
        let mut k1 = keys(&v);
        let mut k2 = keys(&v2);
        k1.sort_by_key(|k| format!("{k:?}"));
        k2.sort_by_key(|k| format!("{k:?}"));
        assert_eq!(k1, k2);
    }

    #[test]
    fn rows_cap_and_placeholders() {
        let many: Vec<ProcRow> = (2..200).map(|p| proc(p, 1, "x", 0.0)).collect();
        assert_eq!(row_views(&many, "", ProcSort::Cpu, None).len(), MAX_ROWS);
        assert_eq!(
            row_views(&[], "", ProcSort::Cpu, None),
            vec![(
                RowKey::Placeholder,
                RowView::Placeholder("Reading processes…")
            )]
        );
        assert_eq!(
            row_views(&many, "nomatch", ProcSort::Cpu, None),
            vec![(
                RowKey::Placeholder,
                RowView::Placeholder("No process matches the search")
            )]
        );
    }

    #[test]
    fn paused_rows_say_so_and_offer_resume() {
        let mut stopped = proc(30, 1, "sleep", 0.0);
        stopped.paused = true;
        let v = row_views(&[stopped, proc(31, 1, "cat", 0.0)], "", ProcSort::Cpu, None);
        let RowView::Proc {
            subtitle, paused, ..
        } = &v[0].1
        else {
            panic!("process row");
        };
        assert!(*paused);
        assert!(subtitle.starts_with("Paused · 30"), "{subtitle}");
        assert!(matches!(v[1].1, RowView::Proc { paused: false, .. }));
        assert_eq!(
            pause_button_face(30, "sleep", true),
            (
                "media-playback-start-symbolic",
                "Resume sleep (30)".to_string()
            )
        );
        assert_eq!(
            pause_button_face(31, "cat", false),
            (
                "media-playback-pause-symbolic",
                "Pause cat (31)".to_string()
            )
        );
    }

    #[test]
    fn grouped_rows_mark_members() {
        let rows = vec![proc(100, 1, "firefox", 3.0), proc(101, 100, "tab", 1.0)];
        let clients = vec![(100, "firefox".to_string())];
        let v = row_views(&rows, "", ProcSort::Cpu, Some(&clients));
        assert_eq!(v[0].0, RowKey::Group(Some("firefox".into())));
        assert!(matches!(v[1].1, RowView::Proc { member: true, .. }));
        assert_eq!(v.len(), 3);
    }
}
