//! SSH Sessions menu — the panel content for `MenuType::SshSessions`.
//!
//! Lists the hosts parsed from `~/.ssh/config` (see [`crate::ssh`]),
//! active connections first and tinted, with a search filter and a
//! live count. Clicking a row opens `kitty -e ssh <host>`. Active
//! sessions are re-polled while the panel is open. With hundreds of
//! hosts the rendered list is capped; the search narrows it.

use crate::ssh::{self, SshHost};
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Re-poll cadence for active sessions while the panel is open.
const POLL: Duration = Duration::from_secs(10);
/// Cap on rendered rows so a multi-hundred-host config stays snappy;
/// the search filter reaches the rest.
const MAX_ROWS: usize = 80;

pub(crate) struct SshSessionsMenuWidgetModel {
    hosts: Vec<SshHost>,
    active: Vec<String>,
    filter: String,
    content: gtk::Box,
    /// `true` once the active-session poll loop has been spawned (on
    /// first reveal), so a menu the user never opens runs no `pgrep`.
    poll_started: bool,
    /// Shared with the poll loop; gates the `pgrep` probe so it only
    /// runs while the panel is visible (it polled every 10 s on every
    /// monitor, forever, regardless of visibility — despite the
    /// "while the panel is open" comment).
    visible: Arc<AtomicBool>,
}

#[derive(Debug)]
pub(crate) enum SshSessionsMenuWidgetInput {
    Search(String),
    /// Sent by the host menu on show/hide. Starts the active-session
    /// poll lazily on first reveal and gates the probe on visibility.
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum SshSessionsMenuWidgetOutput {}

pub(crate) struct SshSessionsMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum SshSessionsMenuWidgetCommandOutput {
    Active(Vec<String>),
}

#[relm4::component(pub(crate))]
impl Component for SshSessionsMenuWidgetModel {
    type CommandOutput = SshSessionsMenuWidgetCommandOutput;
    type Input = SshSessionsMenuWidgetInput;
    type Output = SshSessionsMenuWidgetOutput;
    type Init = SshSessionsMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "ssh-sessions-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("utilities-terminal-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_label: "SSH Sessions",
                },
                gtk::Label {
                    add_css_class: "ssh-sessions-count",
                    #[watch]
                    set_label: &active_summary(&model.active),
                    #[watch]
                    set_visible: !model.active.is_empty(),
                },
            },

            #[name = "search_entry"]
            gtk::Entry {
                add_css_class: "ssh-sessions-search",
                set_placeholder_text: Some("Filter hosts…"),
                connect_changed[sender] => move |e| {
                    sender.input(SshSessionsMenuWidgetInput::Search(e.text().to_string()));
                },
            },

            gtk::ScrolledWindow {
                set_vexpand: true,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 320,

                #[local_ref]
                content -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 2,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // The active-session poll loop is started lazily on first reveal
        // (see `ParentRevealChanged`), not here — so a menu the user never
        // opens forks no `pgrep`.
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let model = SshSessionsMenuWidgetModel {
            hosts: ssh::load_hosts(),
            active: Vec::new(),
            filter: String::new(),
            content: content.clone(),
            poll_started: false,
            visible: Arc::new(AtomicBool::new(false)),
        };
        let widgets = view_output!();

        rebuild(&model.content, &model.hosts, &model.active, "", &sender);

        // Focus the filter each time the panel is shown so the user can
        // type immediately (the frame grants the menu keyboard focus).
        {
            let entry = widgets.search_entry.clone();
            root.connect_map(move |_| {
                entry.grab_focus();
            });
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            SshSessionsMenuWidgetInput::Search(term) => {
                self.filter = term;
                rebuild(
                    &self.content,
                    &self.hosts,
                    &self.active,
                    &self.filter,
                    &sender,
                );
            }
            SshSessionsMenuWidgetInput::ParentRevealChanged(visible) => {
                self.visible.store(visible, Ordering::Relaxed);
                if visible && !self.poll_started {
                    self.poll_started = true;
                    start_polling(&sender, self.visible.clone());
                }
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SshSessionsMenuWidgetCommandOutput::Active(active) => {
                if active != self.active {
                    self.active = active;
                    rebuild(
                        &self.content,
                        &self.hosts,
                        &self.active,
                        &self.filter,
                        &sender,
                    );
                }
            }
        }
    }
}

/// Spawn the active-session poll loop (lazily, on first reveal). The
/// `pgrep` probe is gated on `visible`, so while the panel is hidden the
/// loop just sleeps — no per-monitor subprocess every 10 s for a menu
/// nobody is looking at.
fn start_polling(sender: &ComponentSender<SshSessionsMenuWidgetModel>, visible: Arc<AtomicBool>) {
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        let mut first = true;
        loop {
            let delay = if first {
                Duration::from_millis(50)
            } else {
                POLL
            };
            first = false;
            tokio::select! {
                () = &mut shutdown_fut => break,
                _ = tokio::time::sleep(delay) => {}
            }
            if !visible.load(Ordering::Relaxed) {
                continue;
            }
            let active = ssh::active_targets().await;
            let _ = out.send(SshSessionsMenuWidgetCommandOutput::Active(active));
        }
    });
}

fn active_summary(active: &[String]) -> String {
    match active.len() {
        0 => String::new(),
        1 => "1 active".to_string(),
        n => format!("{n} active"),
    }
}

fn matches_filter(h: &SshHost, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    h.name.to_ascii_lowercase().contains(needle)
        || h.hostname.to_ascii_lowercase().contains(needle)
        || h.user.to_ascii_lowercase().contains(needle)
}

/// (Re)build the host list: active hosts first, then inactive; filtered
/// and capped. Active connections not matching any config host are
/// surfaced as ad-hoc rows so a live session is never hidden.
fn rebuild(
    container: &gtk::Box,
    hosts: &[SshHost],
    active: &[String],
    filter: &str,
    sender: &ComponentSender<SshSessionsMenuWidgetModel>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let needle = filter.trim().to_ascii_lowercase();

    let mut act: Vec<(SshHost, bool)> = Vec::new();
    let mut inact: Vec<(SshHost, bool)> = Vec::new();
    for h in hosts {
        if !matches_filter(h, &needle) {
            continue;
        }
        if ssh::host_is_active(h, active) {
            act.push((h.clone(), true));
        } else {
            inact.push((h.clone(), false));
        }
    }
    // Live targets with no matching config host (e.g. `ssh 1.2.3.4`).
    for target in active {
        let known = hosts
            .iter()
            .any(|h| ssh::host_is_active(h, std::slice::from_ref(target)));
        if !known && (needle.is_empty() || target.to_ascii_lowercase().contains(&needle)) {
            act.push((
                SshHost {
                    name: target.clone(),
                    hostname: String::new(),
                    user: String::new(),
                    port: String::new(),
                },
                true,
            ));
        }
    }

    let combined: Vec<(SshHost, bool)> = act.into_iter().chain(inact).collect();
    let total = combined.len();

    if total == 0 {
        let empty = gtk::Label::new(Some(if needle.is_empty() {
            "No hosts in ~/.ssh/config"
        } else {
            "No matching hosts"
        }));
        empty.add_css_class("label-small");
        empty.set_halign(gtk::Align::Start);
        container.append(&empty);
        return;
    }

    for (host, is_active) in combined.iter().take(MAX_ROWS) {
        container.append(&make_row(host, *is_active, sender));
    }
    if total > MAX_ROWS {
        let more = gtk::Label::new(Some(&format!("+{} more — refine search", total - MAX_ROWS)));
        more.add_css_class("label-small");
        more.set_halign(gtk::Align::Start);
        more.set_margin_top(4);
        container.append(&more);
    }
}

/// One host row: status dot + name / subtitle. Click connects.
fn make_row(
    host: &SshHost,
    is_active: bool,
    sender: &ComponentSender<SshSessionsMenuWidgetModel>,
) -> gtk::Button {
    let btn = gtk::Button::new();
    btn.set_css_classes(if is_active {
        &["ssh-host-row", "active"]
    } else {
        &["ssh-host-row"]
    });

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);

    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.set_css_classes(if is_active {
        &["ssh-status-dot", "active"]
    } else {
        &["ssh-status-dot"]
    });
    dot.set_valign(gtk::Align::Center);
    row.append(&dot);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    let name = gtk::Label::new(Some(&host.name));
    name.add_css_class("ssh-host-name");
    name.set_halign(gtk::Align::Start);
    name.set_xalign(0.0);
    text.append(&name);
    let sub = host.subtitle();
    if !sub.is_empty() && sub != host.name {
        let sub_l = gtk::Label::new(Some(&sub));
        sub_l.add_css_class("ssh-host-sub");
        sub_l.set_halign(gtk::Align::Start);
        sub_l.set_xalign(0.0);
        sub_l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&sub_l);
    }
    row.append(&text);

    let go = gtk::Image::from_icon_name("go-next-symbolic");
    go.add_css_class("ssh-host-go");
    go.set_valign(gtk::Align::Center);
    row.append(&go);

    btn.set_child(Some(&row));

    let name = host.name.clone();
    let _ = sender; // reserved for a future close-on-connect output
    btn.connect_clicked(move |_| {
        ssh::connect(&name);
    });

    btn
}
