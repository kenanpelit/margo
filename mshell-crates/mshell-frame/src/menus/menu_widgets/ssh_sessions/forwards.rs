//! SSH Sessions menu · **Forwards** tab: named `ssh -L` profiles
//! (`network.ssh_forwards`) started / stopped as systemd user services
//! (`mshell_utils::ssh_forward`), with their live state, open-in-browser /
//! copy-URL for the local end, the journal in a terminal, and an inline
//! form to add one. States are polled every few seconds, only while the
//! menu is open.

use crate::menus::reveal_poll::RevealGate;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, NetworkConfigStoreFields, SshForward};
use mshell_utils::ssh_forward::{self, ForwardState};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, CheckButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(5);

fn saved_forwards() -> Vec<SshForward> {
    config_manager()
        .config()
        .network()
        .ssh_forwards()
        .get_untracked()
}

/// Parse a port entry (`""` / junk → 0, rejected by validation).
pub(crate) fn port_of(text: &str) -> u16 {
    text.trim().parse().unwrap_or(0)
}

/// The row subtitle: `localhost:5433 → db:5432 via host · Connected`.
pub(crate) fn subtitle(f: &SshForward, state: ForwardState) -> String {
    format!(
        "localhost:{} → {}:{} via {} · {}",
        f.local_port,
        f.remote_host,
        f.remote_port,
        f.host,
        state.label()
    )
}

pub(crate) struct ForwardsModel {
    forwards: Vec<SshForward>,
    states: HashMap<String, ForwardState>,
    list: gtk::Box,
    status: gtk::Label,
    form: gtk::Revealer,
    name: gtk::Entry,
    host: gtk::Entry,
    local_port: gtk::Entry,
    remote_host: gtk::Entry,
    remote_port: gtk::Entry,
    https: gtk::CheckButton,
    visible: RevealGate,
    poll_started: bool,
}

impl std::fmt::Debug for ForwardsModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardsModel")
            .field("forwards", &self.forwards.len())
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum ForwardsInput {
    /// The menu was shown / hidden (gates the state poll).
    Reveal(bool),
    /// Start (`true`) / stop a forward by name.
    Set(String, bool),
    Open(String),
    Copy(String),
    Logs(String),
    Delete(String),
    ToggleForm,
    Add,
}

#[derive(Debug)]
pub(crate) enum ForwardsCmd {
    States(HashMap<String, ForwardState>),
    /// A start / stop finished (`Err`: why); re-poll after.
    Done(Result<(), String>),
}

#[relm4::component(pub(crate))]
impl Component for ForwardsModel {
    type CommandOutput = ForwardsCmd;
    type Input = ForwardsInput;
    type Output = ();
    type Init = ();

    view! {
        #[root]
        gtk::Box {
            add_css_class: "ssh-forwards",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            #[local_ref]
            list_widget -> gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_css_classes: &["ok-boxed-list", "compact"],
            },

            #[local_ref]
            status_widget -> gtk::Label {
                set_css_classes: &["label-small", "dim-label"],
                set_xalign: 0.0,
                set_wrap: true,
            },

            #[local_ref]
            form_widget -> gtk::Revealer {
                set_transition_type: gtk::RevealerTransitionType::SlideDown,
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                    #[local_ref]
                    name_widget -> gtk::Entry {
                        set_placeholder_text: Some("Name (e.g. Grafana)"),
                    },
                    #[local_ref]
                    host_widget -> gtk::Entry {
                        set_placeholder_text: Some("SSH host — an alias from ~/.ssh/config or user@host"),
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,
                        #[local_ref]
                        local_port_widget -> gtk::Entry {
                            set_placeholder_text: Some("Local port"),
                            set_input_purpose: gtk::InputPurpose::Digits,
                            set_width_chars: 8,
                        },
                        #[local_ref]
                        remote_host_widget -> gtk::Entry {
                            set_hexpand: true,
                            set_text: "localhost",
                            set_placeholder_text: Some("Remote host (as the SSH host sees it)"),
                        },
                        #[local_ref]
                        remote_port_widget -> gtk::Entry {
                            set_placeholder_text: Some("Remote port"),
                            set_input_purpose: gtk::InputPurpose::Digits,
                            set_width_chars: 8,
                        },
                    },
                    #[local_ref]
                    https_widget -> gtk::CheckButton {
                        set_label: Some("Open as https://"),
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,
                        set_halign: gtk::Align::End,
                        gtk::Button {
                            set_css_classes: &["ok-button-surface", "ok-button-cell"],
                            set_label: "Cancel",
                            connect_clicked => ForwardsInput::ToggleForm,
                        },
                        gtk::Button {
                            set_css_classes: &["ok-button-primary", "ok-button-cell"],
                            set_label: "Add",
                            connect_clicked => ForwardsInput::Add,
                        },
                    },
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::End,
                set_margin_top: 8,
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Add forward",
                    #[watch]
                    set_visible: !model.form.reveals_child(),
                    connect_clicked => ForwardsInput::ToggleForm,
                },
            },
        }
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let list_widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let status_widget = gtk::Label::new(None);
        let form_widget = gtk::Revealer::new();
        let name_widget = gtk::Entry::new();
        let host_widget = gtk::Entry::new();
        let local_port_widget = gtk::Entry::new();
        let remote_host_widget = gtk::Entry::new();
        let remote_port_widget = gtk::Entry::new();
        let https_widget = gtk::CheckButton::new();
        let model = ForwardsModel {
            forwards: saved_forwards(),
            states: HashMap::new(),
            list: list_widget.clone(),
            status: status_widget.clone(),
            form: form_widget.clone(),
            name: name_widget.clone(),
            host: host_widget.clone(),
            local_port: local_port_widget.clone(),
            remote_host: remote_host_widget.clone(),
            remote_port: remote_port_widget.clone(),
            https: https_widget.clone(),
            visible: RevealGate::new(false),
            poll_started: false,
        };
        let widgets = view_output!();
        model.render(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            ForwardsInput::Reveal(on) => {
                self.visible.set(on);
                if on {
                    self.forwards = saved_forwards();
                    self.render(&sender);
                    if !self.poll_started {
                        self.poll_started = true;
                        start_polling(&sender, self.visible.clone());
                    }
                }
                return;
            }
            ForwardsInput::Set(name, on) => {
                if let Some(f) = self.find(&name) {
                    self.status.set_label(&format!(
                        "{} {}…",
                        if on { "Starting" } else { "Stopping" },
                        f.name
                    ));
                    self.states.insert(
                        f.name.clone(),
                        if on {
                            ForwardState::Starting
                        } else {
                            ForwardState::Stopped
                        },
                    );
                    sender.oneshot_command(async move {
                        let r = if on {
                            ssh_forward::start(&f).await
                        } else {
                            ssh_forward::stop(&f).await
                        };
                        ForwardsCmd::Done(r)
                    });
                }
            }
            ForwardsInput::Open(name) => {
                if let Some(f) = self.find(&name) {
                    let url = ssh_forward::url(&f);
                    if let Err(e) = mshell_process::desktop::open(&url) {
                        self.status.set_label(&format!("Could not open {url}: {e}"));
                    }
                }
                return;
            }
            ForwardsInput::Copy(name) => {
                if let Some(f) = self.find(&name) {
                    let url = ssh_forward::url(&f);
                    self.list.clipboard().set_text(&url);
                    self.status.set_label(&format!("Copied {url}"));
                }
                return;
            }
            ForwardsInput::Logs(name) => {
                if let Some(f) = self.find(&name) {
                    use mshell_process::SpawnReaped;
                    let cmd = ssh_forward::logs_command(&f);
                    match mshell_process::terminal::command(&cmd, "ssh-forward") {
                        Ok((_, mut c)) => {
                            if let Err(e) = c.spawn_reaped() {
                                self.status
                                    .set_label(&format!("Could not open a terminal: {e}"));
                            }
                        }
                        Err(e) => self.status.set_label(&format!("No terminal: {e}")),
                    }
                }
                return;
            }
            ForwardsInput::Delete(name) => {
                if let Some(f) = self.find(&name) {
                    // Never leave an orphaned tunnel running.
                    let stop = f.clone();
                    sender.oneshot_command(async move {
                        let _ = ssh_forward::stop(&stop).await;
                        ForwardsCmd::Done(Ok(()))
                    });
                    config_manager().update_config(|c| {
                        c.network.ssh_forwards.retain(|x| x.name != f.name);
                    });
                    self.forwards.retain(|x| x.name != f.name);
                    self.status.set_label(&format!("Removed {}", f.name));
                }
            }
            ForwardsInput::ToggleForm => {
                self.form.set_reveal_child(!self.form.reveals_child());
                if self.form.reveals_child() {
                    self.name.grab_focus();
                }
            }
            ForwardsInput::Add => {
                let f = SshForward {
                    name: self.name.text().trim().to_string(),
                    host: self.host.text().trim().to_string(),
                    local_port: port_of(&self.local_port.text()),
                    remote_host: match self.remote_host.text().trim() {
                        "" => "localhost".to_string(),
                        h => h.to_string(),
                    },
                    remote_port: port_of(&self.remote_port.text()),
                    https: self.https.is_active(),
                };
                match ssh_forward::validate(&f, &self.forwards) {
                    Err(e) => self.status.set_label(&e),
                    Ok(()) => {
                        let saved = f.clone();
                        config_manager().update_config(|c| c.network.ssh_forwards.push(saved));
                        self.status
                            .set_label(&format!("Added {} — switch it on to connect", f.name));
                        self.forwards.push(f);
                        for e in [&self.name, &self.host, &self.local_port, &self.remote_port] {
                            e.set_text("");
                        }
                        self.form.set_reveal_child(false);
                    }
                }
            }
        }
        self.render(&sender);
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _: &Self::Root,
    ) {
        match msg {
            ForwardsCmd::States(states) => {
                if states == self.states {
                    return;
                }
                self.states = states;
            }
            ForwardsCmd::Done(result) => {
                match result {
                    Ok(()) => self.status.set_label(""),
                    Err(e) => self.status.set_label(&e),
                }
                let forwards = self.forwards.clone();
                sender.oneshot_command(async move { ForwardsCmd::States(probe(&forwards).await) });
            }
        }
        self.render(&sender);
    }
}

impl ForwardsModel {
    fn find(&self, name: &str) -> Option<SshForward> {
        self.forwards.iter().find(|f| f.name == name).cloned()
    }

    fn render(&self, sender: &ComponentSender<Self>) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        if let Some(why) = ssh_forward::unavailable() {
            self.list.append(&hint(why));
            return;
        }
        if self.forwards.is_empty() {
            self.list.append(&hint(
                "No forwards yet. Add one to reach a remote port (a database, a web UI) on localhost.",
            ));
            return;
        }
        for f in &self.forwards {
            let state = self
                .states
                .get(&f.name)
                .copied()
                .unwrap_or(ForwardState::Stopped);
            self.list.append(&row(f, state, sender));
        }
    }
}

fn hint(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(&["label-small", "dim-label"]);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_margin_top(8);
    l.set_margin_bottom(8);
    l.set_margin_start(12);
    l.set_margin_end(12);
    l
}

fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("ok-button-flat");
    b.set_valign(gtk::Align::Center);
    mshell_common::a11y::name(&b, name);
    b
}

fn row(f: &SshForward, state: ForwardState, sender: &ComponentSender<ForwardsModel>) -> gtk::Box {
    let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    r.add_css_class("ssh-forward-row");
    if state == ForwardState::Running {
        r.add_css_class("active");
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(&f.name));
    title.add_css_class("ssh-host-name");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);
    let sub = gtk::Label::new(Some(&subtitle(f, state)));
    sub.add_css_class("ssh-host-sub");
    if matches!(state, ForwardState::Retrying { .. } | ForwardState::Failed) {
        sub.add_css_class("warn");
    }
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&sub);
    r.append(&text);

    let url = ssh_forward::url(f);
    type Make = fn(String) -> ForwardsInput;
    let actions: [(&str, String, Make); 4] = [
        (
            "open-in-new-symbolic",
            format!("Open {url}"),
            ForwardsInput::Open,
        ),
        (
            "edit-copy-symbolic",
            format!("Copy {url}"),
            ForwardsInput::Copy,
        ),
        (
            "text-x-generic-symbolic",
            format!("Logs of {}", f.name),
            ForwardsInput::Logs,
        ),
        (
            "user-trash-symbolic",
            format!("Remove {}", f.name),
            ForwardsInput::Delete,
        ),
    ];
    for (icon, label, make) in actions {
        let b = icon_button(icon, &label);
        let s = sender.clone();
        let name = f.name.clone();
        b.connect_clicked(move |_| s.input(make(name.clone())));
        r.append(&b);
    }

    let sw = gtk::Switch::new();
    sw.set_valign(gtk::Align::Center);
    sw.set_active(state.is_on());
    sw.set_state(state.is_on());
    mshell_common::a11y::name(&sw, &format!("Connect {}", f.name));
    let s = sender.clone();
    let name = f.name.clone();
    sw.connect_state_set(move |_, on| {
        s.input(ForwardsInput::Set(name.clone(), on));
        glib::Propagation::Stop
    });
    r.append(&sw);
    r
}

/// The state of every forward (by name).
async fn probe(forwards: &[SshForward]) -> HashMap<String, ForwardState> {
    let mut out = HashMap::new();
    for f in forwards {
        out.insert(f.name.clone(), ssh_forward::state(f).await);
    }
    out
}

fn start_polling(sender: &ComponentSender<ForwardsModel>, visible: RevealGate) {
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        let mut first = true;
        loop {
            if !first {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = visible.tick(POLL) => {}
                }
            }
            first = false;
            let _ = out.send(ForwardsCmd::States(probe(&saved_forwards()).await));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_text_and_ports() {
        let f = SshForward {
            name: "db".into(),
            host: "kenan@homelab".into(),
            local_port: 5433,
            remote_host: "db.internal".into(),
            remote_port: 5432,
            https: false,
        };
        assert_eq!(
            subtitle(&f, ForwardState::Running),
            "localhost:5433 → db.internal:5432 via kenan@homelab · Connected"
        );
        assert_eq!(port_of(" 8080 "), 8080);
        assert_eq!(port_of("http"), 0);
        assert_eq!(port_of("70000"), 0);
    }
}
