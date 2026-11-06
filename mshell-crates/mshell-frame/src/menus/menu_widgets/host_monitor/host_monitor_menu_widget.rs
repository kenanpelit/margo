//! Host Monitor menu — the panel for `MenuType::HostMonitor` (the Host
//! Monitor pill, `mshellctl menu host-monitor`).
//!
//! A §12 header (meta: "2 down" / "All up"; actions: add, check now), then
//! one boxed-list row per host in `network.host_monitor.hosts`: a status
//! glyph, the label (or host) over the host and its state, the round-trip
//! time coloured by tier (< 100 ms, < 300 ms, slower) and a remove button.
//! The add form takes an IP, a host name or a pasted link (reduced to its
//! host) and an optional label. The checks themselves run in the pill's
//! shared [`HOST_MONITOR_POLL`]; this menu keeps it running while open.

use crate::bars::bar_widgets::host_monitor::{
    HOST_MONITOR_POLL, HostSnapshot, monitor_config, state_text,
};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::MonitoredHost;
use mshell_config::schema::menu_widgets::MenuWidget;
use mshell_utils::host_monitor::{self, HostState, HostStatus, LatencyTier};
use mshell_utils::shared_poll::PollSubscription;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct HostMonitorMenuWidgetModel {
    snapshot: Option<HostSnapshot>,
    /// What the list shows now (config order + last states).
    rows: Vec<HostStatus>,
    poll: PollSubscription<HostSnapshot>,
    meta: gtk::Label,
    list: gtk::Box,
    hint: gtk::Label,
    form: gtk::Revealer,
    host: gtk::Entry,
    label: gtk::Entry,
    status: gtk::Label,
}

impl std::fmt::Debug for HostMonitorMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostMonitorMenuWidgetModel")
            .field("rows", &self.rows.len())
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum HostMonitorMenuWidgetInput {
    ParentRevealChanged(bool),
    Refresh,
    ToggleForm,
    Add,
    Remove(String),
}

#[derive(Debug)]
pub(crate) enum HostMonitorMenuWidgetOutput {}

pub(crate) struct HostMonitorMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum HostMonitorMenuWidgetCommandOutput {
    Snapshot(HostSnapshot),
}

#[relm4::component(pub(crate))]
impl Component for HostMonitorMenuWidgetModel {
    type CommandOutput = HostMonitorMenuWidgetCommandOutput;
    type Input = HostMonitorMenuWidgetInput;
    type Output = HostMonitorMenuWidgetOutput;
    type Init = HostMonitorMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "host-monitor-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            #[local_ref]
            header_root -> gtk::Box {},

            #[local_ref]
            form_widget -> gtk::Revealer {
                set_transition_type: gtk::RevealerTransitionType::SlideDown,
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                    #[local_ref]
                    host_widget -> gtk::Entry {
                        set_placeholder_text: Some("IP, host name or link (e.g. 192.168.1.1, nas.lan)"),
                        connect_activate => HostMonitorMenuWidgetInput::Add,
                    },
                    #[local_ref]
                    label_widget -> gtk::Entry {
                        set_placeholder_text: Some("Label (optional)"),
                        connect_activate => HostMonitorMenuWidgetInput::Add,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,
                        set_halign: gtk::Align::End,
                        gtk::Button {
                            set_css_classes: &["ok-button-surface", "ok-button-cell"],
                            set_label: "Cancel",
                            connect_clicked => HostMonitorMenuWidgetInput::ToggleForm,
                        },
                        gtk::Button {
                            set_css_classes: &["ok-button-primary", "ok-button-cell"],
                            set_label: "Add",
                            connect_clicked => HostMonitorMenuWidgetInput::Add,
                        },
                    },
                },
            },

            #[local_ref]
            status_widget -> gtk::Label {
                set_css_classes: &["label-small", "dim-label"],
                set_xalign: 0.0,
                set_wrap: true,
                set_visible: false,
            },

            #[local_ref]
            hint_widget -> gtk::Label {
                set_css_classes: &["label-small", "dim-label"],
                set_xalign: 0.0,
                set_wrap: true,
            },

            #[name = "fill_list"]
            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_propagate_natural_height: true,

                #[local_ref]
                list_widget -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_css_classes: &["ok-boxed-list", "compact", "host-monitor-list"],
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let poll = HOST_MONITOR_POLL.subscribe();
        let mut rx = poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let snap = rx.borrow_and_update().clone();
                if let Some(s) = snap {
                    let _ = out.send(HostMonitorMenuWidgetCommandOutput::Snapshot(s));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = rx.changed() => if changed.is_err() { break },
                }
            }
        });

        let (s_add, s_refresh) = (sender.clone(), sender.clone());
        let header = PanelHeader::new("server-symbolic", MenuWidget::HostMonitor.display_name())
            .with_meta()
            .action("list-add-symbolic", "Add a host", move || {
                s_add.input(HostMonitorMenuWidgetInput::ToggleForm);
            })
            .action("view-refresh-symbolic", "Check now", move || {
                s_refresh.input(HostMonitorMenuWidgetInput::Refresh);
            });
        let header_root = header.root().clone();
        let meta = header.meta().cloned().unwrap_or_default();
        let form_widget = gtk::Revealer::new();
        let host_widget = gtk::Entry::new();
        let label_widget = gtk::Entry::new();
        let status_widget = gtk::Label::new(None);
        let hint_widget = gtk::Label::new(None);
        let list_widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let mut model = HostMonitorMenuWidgetModel {
            snapshot: None,
            rows: Vec::new(),
            poll,
            meta,
            list: list_widget.clone(),
            hint: hint_widget.clone(),
            form: form_widget.clone(),
            host: host_widget.clone(),
            label: label_widget.clone(),
            status: status_widget.clone(),
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 420);
        model.render(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            HostMonitorMenuWidgetInput::ParentRevealChanged(visible) => {
                self.poll.set_active(visible);
                if visible {
                    // Pick up a hand-edited host list.
                    self.render(&sender);
                } else {
                    self.form.set_reveal_child(false);
                    self.set_status("");
                }
                return;
            }
            HostMonitorMenuWidgetInput::Refresh => {
                HOST_MONITOR_POLL.refresh_now();
                return;
            }
            HostMonitorMenuWidgetInput::ToggleForm => {
                let open = !self.form.reveals_child();
                self.form.set_reveal_child(open);
                if open {
                    self.host.grab_focus();
                }
                return;
            }
            HostMonitorMenuWidgetInput::Add => {
                let hosts = monitor_config().hosts;
                match host_monitor::validate_new(&self.host.text(), &hosts) {
                    Err(e) => self.set_status(&e),
                    Ok(host) => {
                        let entry = MonitoredHost {
                            host: host.clone(),
                            label: self.label.text().trim().to_string(),
                        };
                        config_manager()
                            .update_config(move |c| c.network.host_monitor.hosts.push(entry));
                        self.host.set_text("");
                        self.label.set_text("");
                        self.form.set_reveal_child(false);
                        self.set_status(&format!("Added {host}"));
                        HOST_MONITOR_POLL.refresh_now();
                    }
                }
            }
            HostMonitorMenuWidgetInput::Remove(host) => {
                config_manager()
                    .update_config(|c| c.network.host_monitor.hosts.retain(|h| h.host != host));
                self.set_status(&format!("Removed {host}"));
                HOST_MONITOR_POLL.refresh_now();
            }
        }
        self.render(&sender);
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            HostMonitorMenuWidgetCommandOutput::Snapshot(s) => {
                self.snapshot = Some(s);
                self.render(&sender);
            }
        }
    }
}

impl HostMonitorMenuWidgetModel {
    fn set_status(&self, text: &str) {
        self.status.set_label(text);
        self.status.set_visible(!text.is_empty());
    }

    fn render(&mut self, sender: &ComponentSender<Self>) {
        let rows = host_monitor::rows(
            &monitor_config().hosts,
            self.snapshot.as_ref().map_or(&[][..], |s| &s.checked[..]),
        );
        let unavailable = self.snapshot.as_ref().and_then(|s| s.unavailable);
        self.meta.set_label(&meta_text(&rows));
        let hint = if let Some(why) = unavailable {
            why.to_string()
        } else if rows.is_empty() {
            "No hosts yet. Add a router, a server or a VPN endpoint to see when it stops answering."
                .to_string()
        } else {
            String::new()
        };
        self.hint.set_label(&hint);
        self.hint.set_visible(!hint.is_empty());
        self.list.set_visible(!rows.is_empty());
        if rows == self.rows && self.list.first_child().is_some() {
            return;
        }
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        for r in &rows {
            self.list.append(&row(r, sender));
        }
        self.rows = rows;
    }
}

/// The header's meta: "All up", "1 down", "Checking…", or nothing.
pub(crate) fn meta_text(rows: &[HostStatus]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    match host_monitor::down_count(rows) {
        0 if rows.iter().any(|r| r.state == HostState::Checking) => "Checking…".to_string(),
        0 => "All up".to_string(),
        n => format!("{n} down"),
    }
}

/// `(glyph, css state)` of a host's leading status icon.
fn glyph(state: &HostState) -> (&'static str, &'static str) {
    match state {
        HostState::Checking => ("network-wired-acquiring-symbolic", "checking"),
        HostState::Up { .. } => ("network-wired-symbolic", "up"),
        HostState::Down { .. } => ("network-wired-disconnected-symbolic", "down"),
    }
}

fn tier_class(tier: LatencyTier) -> &'static str {
    match tier {
        LatencyTier::Good => "good",
        LatencyTier::Slow => "slow",
        LatencyTier::Bad => "bad",
    }
}

fn row(r: &HostStatus, sender: &ComponentSender<HostMonitorMenuWidgetModel>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("host-monitor-row");

    let (icon, state_class) = glyph(&r.state);
    let image = gtk::Image::from_icon_name(icon);
    image.set_css_classes(&["host-monitor-status", state_class]);
    image.set_valign(gtk::Align::Center);
    row.append(&image);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(r.display()));
    title.add_css_class("label-medium");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);
    let sub = gtk::Label::new(Some(&subtitle(r)));
    sub.set_css_classes(&["label-small", "dim-label"]);
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&sub);
    row.append(&text);

    if let HostState::Up {
        latency_ms: Some(ms),
    } = r.state
    {
        let latency = gtk::Label::new(Some(&host_monitor::format_latency(ms)));
        latency.set_css_classes(&[
            "host-monitor-latency",
            tier_class(host_monitor::latency_tier(ms)),
        ]);
        latency.set_valign(gtk::Align::Center);
        row.append(&latency);
    }

    let remove = gtk::Button::from_icon_name("user-trash-symbolic");
    remove.add_css_class("ok-button-flat");
    remove.set_valign(gtk::Align::Center);
    mshell_common::a11y::name(&remove, &format!("Stop monitoring {}", r.display()));
    {
        let (s, host) = (sender.clone(), r.host.clone());
        remove.connect_clicked(move |_| s.input(HostMonitorMenuWidgetInput::Remove(host.clone())));
    }
    row.append(&remove);
    row
}

/// The dim line under the name: the host (when a label hides it) and the
/// state without its latency (that has its own coloured column).
pub(crate) fn subtitle(r: &HostStatus) -> String {
    let state = match &r.state {
        HostState::Up { .. } => "Up".to_string(),
        other => state_text(other),
    };
    if r.display() == r.host {
        state
    } else {
        format!("{} · {state}", r.host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(host: &str, label: &str, state: HostState) -> HostStatus {
        HostStatus {
            host: host.into(),
            label: label.into(),
            state,
        }
    }

    #[test]
    fn header_and_row_text() {
        let up = host(
            "nas.lan",
            "NAS",
            HostState::Up {
                latency_ms: Some(1.0),
            },
        );
        let down = host(
            "10.0.0.9",
            "",
            HostState::Down {
                reason: "No reply".into(),
            },
        );
        assert_eq!(meta_text(&[]), "");
        assert_eq!(meta_text(std::slice::from_ref(&up)), "All up");
        assert_eq!(
            meta_text(&[up.clone(), host("x", "", HostState::Checking)]),
            "Checking…"
        );
        assert_eq!(meta_text(&[up.clone(), down.clone()]), "1 down");
        assert_eq!(subtitle(&up), "nas.lan · Up");
        assert_eq!(subtitle(&down), "Down · No reply");
    }
}
