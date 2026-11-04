//! Processes tab of the CPU dashboard: a sortable, searchable process table
//! read from `/proc` (CPU, RAM, swap, disk IO), optionally grouped by the
//! margo window each process descends from, with a confirmed
//! terminate / force-kill. Polls every 2 s only while the menu is revealed
//! AND this tab is the visible one; the `/proc` sweep runs on a blocking
//! thread (DESIGN.md §13.4 — never on the GTK thread).

use super::procs::{
    KillResult, ProcRow, ProcSnapshot, ProcSort, ProcTracker, fmt_kb, fmt_rate, group_by_window,
    read_snapshot, row_matches, send_signal, sort_rows,
};
use super::ui;
use crate::menus::reveal_poll::RevealPoll;
use mshell_services::margo_service;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, EditableExt, OrientableExt, ToggleButtonExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::Path;
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
    /// EPERM on the last attempt: offer the pkexec path.
    needs_admin: Option<(i32, &'static str)>,
    list: gtk::ListBox,
    sort_buttons: Vec<(ProcSort, gtk::Button)>,
}

#[derive(Debug)]
pub(crate) enum ProcessesTabInput {
    /// Menu revealed AND this tab visible.
    SetActive(bool),
    Poll,
    Sort(ProcSort),
    Search(String),
    Group(bool),
    AskKill(i32, String),
    CancelKill,
    /// Send `SIGTERM` (`false`) or `SIGKILL` (`true`) to the pending pid.
    ConfirmKill(bool),
    /// Re-send the refused signal through `pkexec kill`.
    AdminKill,
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
                add_css_class: "settings-search",
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
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 4,
                    set_homogeneous: true,
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
        let sort_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let mut sort_buttons = Vec::new();
        for (sort, label, tip) in [
            (ProcSort::Cpu, "CPU", "Sort by CPU"),
            (ProcSort::Mem, "RAM", "Sort by memory"),
            (ProcSort::Swap, "Swap", "Sort by swap"),
            (ProcSort::Io, "Disk", "Sort by disk read + write rate"),
        ] {
            let b = ui::text_button(label, false);
            b.set_tooltip_text(Some(tip));
            if sort == ProcSort::default() {
                b.add_css_class("selected");
            }
            let s = sender.clone();
            b.connect_clicked(move |_| s.input(ProcessesTabInput::Sort(sort)));
            sort_row.append(&b);
            sort_buttons.push((sort, b));
        }

        let model = ProcessesTabModel {
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
            sort_buttons,
        };
        let widgets = view_output!();
        model
            .list
            .append(&ui::placeholder_row("Reading processes…"));
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
                for (s, b) in &self.sort_buttons {
                    if *s == sort {
                        b.add_css_class("selected");
                    } else {
                        b.remove_css_class("selected");
                    }
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
            ProcessesTabInput::AskKill(pid, name) => {
                self.pending = Some(PendingKill { pid, name });
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
                self.status = match send_signal(p.pid, sig) {
                    Err(why) => why.to_string(),
                    Ok(KillResult::Sent) => format!("Sent SIG{sig_name} to {} ({})", p.name, p.pid),
                    Ok(KillResult::Gone) => format!("{} ({}) has already exited", p.name, p.pid),
                    Ok(KillResult::NotPermitted) => {
                        self.needs_admin = Some((p.pid, sig_name));
                        format!("{} ({}) belongs to another user", p.name, p.pid)
                    }
                    Ok(KillResult::Failed(e)) => format!("kill failed (errno {e})"),
                };
                sender.input(ProcessesTabInput::Poll);
            }
            ProcessesTabInput::AdminKill => {
                let Some((pid, sig)) = self.needs_admin.take() else {
                    return;
                };
                self.status = format!("Waiting for authorization to signal {pid}…");
                sender.oneshot_command(async move {
                    ProcessesTabCmd::AdminDone(super::pkexec_kill(pid, sig).await)
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
    fn render(&self, sender: &ComponentSender<Self>) {
        ui::clear_list(&self.list);
        let mut rows: Vec<ProcRow> = self
            .rows
            .iter()
            .filter(|r| row_matches(r, &self.query))
            .cloned()
            .collect();
        sort_rows(&mut rows, self.sort);
        if rows.is_empty() {
            let text = if self.rows.is_empty() {
                "Reading processes…"
            } else {
                "No process matches the search"
            };
            self.list.append(&ui::placeholder_row(text));
            return;
        }
        if self.group {
            let clients: Vec<(i32, String)> = margo_service()
                .clients
                .get()
                .iter()
                .map(|c| (c.pid.get(), c.class.get()))
                .filter(|(pid, _)| *pid > 0)
                .collect();
            let mut shown = 0;
            for g in group_by_window(&rows, &clients, self.sort) {
                if shown >= MAX_ROWS {
                    break;
                }
                let title = g
                    .app
                    .clone()
                    .unwrap_or_else(|| "Other processes".to_string());
                let subtitle = format!(
                    "{} process{} · {} swap",
                    g.members.len(),
                    if g.members.len() == 1 { "" } else { "es" },
                    fmt_kb(g.swap_kb)
                );
                let body = ui::row_body(
                    &title,
                    &subtitle,
                    &[
                        ui::value_label(&format!("{:.1}%", g.cpu), 6, "").into(),
                        ui::value_label(&fmt_kb(g.rss_kb), 7, "").into(),
                    ],
                );
                body.add_css_class("cpu-dashboard-group-head");
                self.list.append(&ui::plain_row(&body));
                shown += 1;
                for i in g.members.iter().take(MAX_GROUP_MEMBERS) {
                    if let Some(r) = rows.get(*i) {
                        let row = self.proc_row(r, sender);
                        row.add_css_class("cpu-dashboard-group-member");
                        self.list.append(&row);
                        shown += 1;
                    }
                }
            }
        } else {
            for r in rows.iter().take(MAX_ROWS) {
                self.list.append(&self.proc_row(r, sender));
            }
        }
    }

    fn proc_row(&self, r: &ProcRow, sender: &ComponentSender<Self>) -> gtk::ListBoxRow {
        let mut sub = format!("{}", r.pid);
        if r.swap_kb > 0 {
            sub.push_str(&format!(" · swap {}", fmt_kb(r.swap_kb)));
        }
        if let Some((rd, wr)) = r.io_bps
            && rd + wr > 0
        {
            sub.push_str(&format!(" · R {} W {}", fmt_rate(rd), fmt_rate(wr)));
        }
        let kill = ui::icon_button("close-symbolic", &format!("End {} ({})", r.name, r.pid));
        let (pid, name) = (r.pid, r.name.clone());
        let s = sender.clone();
        kill.connect_clicked(move |_| s.input(ProcessesTabInput::AskKill(pid, name.clone())));
        let body = ui::row_body(
            &r.name,
            &sub,
            &[
                ui::value_label(&format!("{:.1}%", r.cpu), 6, "").into(),
                ui::value_label(&fmt_kb(r.rss_kb), 7, "").into(),
                kill.into(),
            ],
        );
        if !r.cmdline.is_empty() {
            body.set_tooltip_text(Some(&r.cmdline));
        }
        ui::plain_row(&body)
    }
}
