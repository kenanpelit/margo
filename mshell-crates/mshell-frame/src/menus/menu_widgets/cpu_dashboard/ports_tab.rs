//! Ports tab of the CPU dashboard: listening TCP/UDP sockets from
//! `ss -tulpnH`, grouped by exposure, with search, copy-PID and a
//! confirmed terminate / force-kill of the owning process. Polls every
//! 5 s only while the menu is revealed and this tab is visible.

use super::ports::{Exposure, PortRow, ProtoFilter, parse_ss, port_matches, sort_ports};
use super::procs::{KillResult, send_signal};
use super::ui;
use crate::menus::reveal_poll::RevealPoll;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingKill {
    pid: i32,
    name: String,
    port: u16,
}

pub(crate) struct PortsTabModel {
    poll: RevealPoll,
    in_flight: bool,
    /// `None` until the first scan; `Err` when `ss` is missing / failed.
    rows: Option<Result<Vec<PortRow>, String>>,
    proto: ProtoFilter,
    query: String,
    pending: Option<PendingKill>,
    status: String,
    needs_admin: Option<(i32, &'static str)>,
    groups: gtk::Box,
    proto_buttons: Vec<(ProtoFilter, gtk::Button)>,
}

#[derive(Debug)]
pub(crate) enum PortsTabInput {
    SetActive(bool),
    Poll,
    Proto(ProtoFilter),
    Search(String),
    CopyPid(i32),
    AskKill(i32, String, u16),
    CancelKill,
    ConfirmKill(bool),
    AdminKill,
}

#[derive(Debug)]
pub(crate) enum PortsTabCmd {
    Scanned(Result<Vec<PortRow>, String>),
    AdminDone(Result<(), String>),
}

async fn scan() -> Result<Vec<PortRow>, String> {
    let out = tokio::process::Command::new("ss")
        .arg("-tulpnH")
        .output()
        .await
        .map_err(|_| "ss is not installed (package iproute2)".to_string())?;
    if !out.status.success() {
        return Err(format!(
            "ss failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(parse_ss(&String::from_utf8_lossy(&out.stdout)))
}

#[relm4::component(pub(crate))]
impl Component for PortsTabModel {
    type CommandOutput = PortsTabCmd;
    type Input = PortsTabInput;
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
                set_placeholder_text: Some("Search port, process or PID…"),
                connect_search_changed[sender] => move |e| {
                    sender.input(PortsTabInput::Search(e.text().to_string()));
                },
            },

            #[local_ref]
            proto_row -> gtk::Box {
                add_css_class: "cpu-dashboard-sort-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                set_homogeneous: true,
            },

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
                    set_label: &model.pending.as_ref().map(|p| format!(
                        "End {} ({}) listening on :{}?", p.name, p.pid, p.port
                    )).unwrap_or_default(),
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_homogeneous: true,
                    gtk::Button {
                        set_css_classes: &["ok-button-surface", "label-small"],
                        set_label: "Cancel",
                        connect_clicked => PortsTabInput::CancelKill,
                    },
                    gtk::Button {
                        set_css_classes: &["ok-button-surface", "label-small"],
                        set_label: "Terminate",
                        set_tooltip_text: Some("SIGTERM: ask the process to quit"),
                        connect_clicked => PortsTabInput::ConfirmKill(false),
                    },
                    gtk::Button {
                        set_css_classes: &["ok-button-error", "label-small"],
                        set_label: "Force kill",
                        set_tooltip_text: Some("SIGKILL: stop it immediately"),
                        connect_clicked => PortsTabInput::ConfirmKill(true),
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
                    connect_clicked => PortsTabInput::AdminKill,
                },
            },

            #[name = "fill_list"]
            gtk::ScrolledWindow {
                set_min_content_height: 0,
                set_max_content_height: 380,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,
                #[local_ref]
                groups -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },
            },
        }
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let s = sender.clone();
        let poll = RevealPoll::new(POLL_INTERVAL, move || {
            s.input_sender().send(PortsTabInput::Poll).is_ok()
        });
        poll.set_revealed(false);

        let groups = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let proto_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let mut proto_buttons = Vec::new();
        for (p, label) in [
            (ProtoFilter::Tcp, "TCP"),
            (ProtoFilter::Udp, "UDP"),
            (ProtoFilter::All, "All"),
        ] {
            let b = ui::text_button(label, false);
            if p == ProtoFilter::default() {
                b.add_css_class("selected");
            }
            let s = sender.clone();
            b.connect_clicked(move |_| s.input(PortsTabInput::Proto(p)));
            proto_row.append(&b);
            proto_buttons.push((p, b));
        }
        let model = PortsTabModel {
            poll,
            in_flight: false,
            rows: None,
            proto: ProtoFilter::default(),
            query: String::new(),
            pending: None,
            status: String::new(),
            needs_admin: None,
            groups: groups.clone(),
            proto_buttons,
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 380);
        model.render(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            PortsTabInput::SetActive(on) => {
                self.poll.set_revealed(on);
                if on {
                    sender.input(PortsTabInput::Poll);
                }
            }
            PortsTabInput::Poll => {
                if !self.in_flight {
                    self.in_flight = true;
                    sender.oneshot_command(async { PortsTabCmd::Scanned(scan().await) });
                }
            }
            PortsTabInput::Proto(p) => {
                self.proto = p;
                for (q, b) in &self.proto_buttons {
                    if *q == p {
                        b.add_css_class("selected");
                    } else {
                        b.remove_css_class("selected");
                    }
                }
                self.render(&sender);
            }
            PortsTabInput::Search(q) => {
                self.query = q;
                self.render(&sender);
            }
            PortsTabInput::CopyPid(pid) => {
                self.groups.clipboard().set_text(&pid.to_string());
                self.status = format!("Copied PID {pid}");
                self.needs_admin = None;
            }
            PortsTabInput::AskKill(pid, name, port) => {
                self.pending = Some(PendingKill { pid, name, port });
                self.status.clear();
                self.needs_admin = None;
            }
            PortsTabInput::CancelKill => self.pending = None,
            PortsTabInput::ConfirmKill(force) => {
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
                sender.input(PortsTabInput::Poll);
            }
            PortsTabInput::AdminKill => {
                let Some((pid, sig)) = self.needs_admin.take() else {
                    return;
                };
                self.status = format!("Waiting for authorization to signal {pid}…");
                sender.oneshot_command(async move {
                    PortsTabCmd::AdminDone(super::pkexec_kill(pid, sig).await)
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
            PortsTabCmd::Scanned(rows) => {
                self.in_flight = false;
                self.rows = Some(rows.map(|mut r| {
                    sort_ports(&mut r);
                    r
                }));
                self.render(&sender);
            }
            PortsTabCmd::AdminDone(res) => {
                self.status = match res {
                    Ok(()) => "Signal sent".to_string(),
                    Err(e) => e,
                };
                sender.input(PortsTabInput::Poll);
            }
        }
    }
}

impl PortsTabModel {
    fn render(&self, sender: &ComponentSender<Self>) {
        while let Some(c) = self.groups.first_child() {
            self.groups.remove(&c);
        }
        let single = |text: &str| {
            let list = gtk::ListBox::new();
            list.set_css_classes(&["ok-boxed-list", "compact", "cpu-dashboard-list"]);
            list.set_selection_mode(gtk::SelectionMode::None);
            list.append(&ui::placeholder_row(text));
            list
        };
        let rows = match &self.rows {
            None => {
                self.groups.append(&single("Scanning listening ports…"));
                return;
            }
            Some(Err(e)) => {
                self.groups.append(&single(e));
                return;
            }
            Some(Ok(rows)) => rows,
        };
        let visible: Vec<&PortRow> = rows
            .iter()
            .filter(|r| port_matches(r, &self.query, self.proto))
            .collect();
        if visible.is_empty() {
            let text = if rows.is_empty() {
                "Nothing is listening"
            } else {
                "No port matches the search"
            };
            self.groups.append(&single(text));
            return;
        }
        for exposure in [
            Exposure::AllInterfaces,
            Exposure::Specific,
            Exposure::Loopback,
        ] {
            let in_group: Vec<&&PortRow> = visible
                .iter()
                .filter(|r| r.exposure() == exposure)
                .collect();
            if in_group.is_empty() {
                continue;
            }
            let caption = gtk::Label::builder()
                .label(exposure.label().to_uppercase())
                .css_classes(["cpu-dashboard-section-label"])
                .xalign(0.0)
                .build();
            self.groups.append(&caption);
            let list = gtk::ListBox::new();
            list.set_css_classes(&["ok-boxed-list", "compact", "cpu-dashboard-list"]);
            list.set_selection_mode(gtk::SelectionMode::None);
            for r in in_group {
                list.append(&port_row(r, sender));
            }
            self.groups.append(&list);
        }
    }
}

fn port_row(r: &PortRow, sender: &ComponentSender<PortsTabModel>) -> gtk::ListBoxRow {
    let addr = if r.addr.contains(':') {
        format!("[{}]", r.addr)
    } else {
        r.addr.clone()
    };
    let mut trailing: Vec<gtk::Widget> =
        vec![ui::value_label(&format!(":{}", r.port), 6, "").into()];
    let (title, subtitle) = match r.owners.first() {
        Some(owner) => {
            let pids: Vec<String> = r.owners.iter().map(|o| o.pid.to_string()).collect();
            let copy = ui::icon_button("edit-copy-symbolic", &format!("Copy PID {}", owner.pid));
            let s = sender.clone();
            let pid = owner.pid;
            copy.connect_clicked(move |_| s.input(PortsTabInput::CopyPid(pid)));
            let kill = ui::icon_button(
                "close-symbolic",
                &format!("End {} ({})", owner.name, owner.pid),
            );
            let s = sender.clone();
            let (name, port) = (owner.name.clone(), r.port);
            kill.connect_clicked(move |_| s.input(PortsTabInput::AskKill(pid, name.clone(), port)));
            trailing.push(copy.into());
            trailing.push(kill.into());
            (
                owner.name.clone(),
                format!("{} {addr} · pid {}", r.proto, pids.join(", ")),
            )
        }
        None => (
            "Hidden owner".to_string(),
            format!("{} {addr} · owned by another user", r.proto),
        ),
    };
    ui::plain_row(&ui::row_body(&title, &subtitle, &trailing))
}
