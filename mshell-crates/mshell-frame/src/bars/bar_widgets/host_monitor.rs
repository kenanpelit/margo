//! Host Monitor bar pill — are the hosts in `network.host_monitor` up?
//!
//! A server glyph, tinted `--success` while every host answers and
//! `--error` with a count of the down hosts beside it when any does not;
//! neutral with no hosts. The pings run once for the whole shell in
//! [`HOST_MONITOR_POLL`] (`mshell_utils::host_monitor`), while a pill or the
//! menu is mapped, and that one loop sends the down / recovery
//! notifications. Click opens `MenuType::HostMonitor`; right-click checks
//! now.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, HostMonitorConfig, NetworkConfigStoreFields,
};
use mshell_process::desktop::{Notice, notify};
use mshell_utils::host_monitor::{self, HostState, HostStatus, Transition};
use mshell_utils::shared_poll::{BoxFuture, PollSubscription, SharedPoll};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};

/// The result of one round of checks.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct HostSnapshot {
    pub(crate) checked: Vec<HostStatus>,
    /// Why nothing could be checked (`ping` missing).
    pub(crate) unavailable: Option<&'static str>,
}

pub(crate) fn monitor_config() -> HostMonitorConfig {
    config_manager()
        .config()
        .network()
        .host_monitor()
        .get_untracked()
}

pub(crate) static HOST_MONITOR_POLL: LazyLock<SharedPoll<HostSnapshot>> =
    LazyLock::new(|| SharedPoll::new(|| host_monitor::interval(&monitor_config()), probe));

/// Last up/down per host, for the notifications (one copy for the shell).
static LAST: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);

fn probe() -> BoxFuture<HostSnapshot> {
    Box::pin(async {
        let cfg = monitor_config();
        if cfg.hosts.is_empty() {
            if let Ok(mut last) = LAST.lock() {
                last.clear();
            }
            return HostSnapshot::default();
        }
        if let Some(why) = host_monitor::unavailable() {
            return HostSnapshot {
                checked: Vec::new(),
                unavailable: Some(why),
            };
        }
        let checked = host_monitor::check_all(&cfg).await;
        let changes = match LAST.lock() {
            Ok(mut last) => host_monitor::transitions(&mut last, &checked),
            Err(_) => Vec::new(),
        };
        for change in changes {
            announce(&cfg, &change);
        }
        HostSnapshot {
            checked,
            unavailable: None,
        }
    })
}

fn announce(cfg: &HostMonitorConfig, change: &Transition) {
    let (summary, body, icon, tag) = match change {
        Transition::Down { name, reason } if cfg.notify_down => (
            format!("{name} is down"),
            reason.clone(),
            "network-wired-disconnected-symbolic",
            name,
        ),
        Transition::Recovered { name } if cfg.notify_recovery => (
            format!("{name} is back up"),
            "Answering again".to_string(),
            "network-wired-symbolic",
            name,
        ),
        _ => return,
    };
    // One notification per host: a recovery replaces its down notice.
    notify(
        Notice::new("Host Monitor", summary, body)
            .icon(icon)
            .replace(format!("host-monitor-{tag}")),
    );
}

pub(crate) struct HostMonitorModel {
    snapshot: Option<HostSnapshot>,
    _poll: Rc<PollSubscription<HostSnapshot>>,
}

#[derive(Debug)]
pub(crate) enum HostMonitorInput {
    Clicked,
    Refresh,
}

#[derive(Debug)]
pub(crate) enum HostMonitorOutput {
    Clicked,
}

pub(crate) struct HostMonitorInit {}

#[derive(Debug)]
pub(crate) enum HostMonitorCommandOutput {
    Snapshot(HostSnapshot),
}

#[relm4::component(pub)]
impl Component for HostMonitorModel {
    type CommandOutput = HostMonitorCommandOutput;
    type Input = HostMonitorInput;
    type Output = HostMonitorOutput;
    type Init = HostMonitorInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "host-monitor-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(HostMonitorInput::Clicked);
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        set_icon_name: Some("server-symbolic"),
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                    },
                    #[name = "count"]
                    gtk::Label {
                        add_css_class: "host-monitor-bar-label",
                        set_visible: false,
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let poll = Rc::new(HOST_MONITOR_POLL.subscribe());
        {
            let p = poll.clone();
            root.connect_map(move |_| p.set_active(true));
            let p = poll.clone();
            root.connect_unmap(move |_| p.set_active(false));
        }
        let mut rx = poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let snap = rx.borrow_and_update().clone();
                if let Some(s) = snap {
                    let _ = out.send(HostMonitorCommandOutput::Snapshot(s));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = rx.changed() => if changed.is_err() { break },
                }
            }
        });

        let right = gtk::GestureClick::new();
        right.set_button(gtk::gdk::BUTTON_SECONDARY);
        {
            let s = sender.clone();
            right.connect_released(move |_, _, _, _| s.input(HostMonitorInput::Refresh));
        }
        root.add_controller(right);

        let model = HostMonitorModel {
            snapshot: None,
            _poll: poll,
        };
        let widgets = view_output!();
        apply_visual(&widgets, &root, None);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            HostMonitorInput::Clicked => {
                let _ = sender.output(HostMonitorOutput::Clicked);
            }
            HostMonitorInput::Refresh => HOST_MONITOR_POLL.refresh_now(),
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            HostMonitorCommandOutput::Snapshot(s) => {
                if self.snapshot.as_ref() != Some(&s) {
                    apply_visual(widgets, root, Some(&s));
                    self.snapshot = Some(s);
                }
            }
        }
    }
}

fn apply_visual(widgets: &HostMonitorModelWidgets, root: &gtk::Box, s: Option<&HostSnapshot>) {
    let rows = host_monitor::rows(
        &monitor_config().hosts,
        s.map_or(&[][..], |s| &s.checked[..]),
    );
    let down = host_monitor::down_count(&rows);
    widgets.count.set_visible(down > 0);
    widgets.count.set_label(&down.to_string());
    root.remove_css_class("error");
    root.remove_css_class("ok");
    let all_up = !rows.is_empty() && rows.iter().all(|r| matches!(r.state, HostState::Up { .. }));
    if down > 0 || s.is_some_and(|s| s.unavailable.is_some()) {
        root.add_css_class("error");
    } else if all_up {
        root.add_css_class("ok");
    }
    root.set_tooltip_text(Some(&tooltip(&rows, s.and_then(|s| s.unavailable))));
}

/// One line per host, then the click hints.
pub(crate) fn tooltip(rows: &[HostStatus], unavailable: Option<&str>) -> String {
    let mut lines = Vec::new();
    if let Some(why) = unavailable {
        lines.push(format!("Host Monitor: {why}"));
    } else if rows.is_empty() {
        lines.push("Host Monitor: no hosts yet".to_string());
    } else {
        lines.push(match host_monitor::down_count(rows) {
            0 => "Host Monitor".to_string(),
            1 => "Host Monitor: 1 host down".to_string(),
            n => format!("Host Monitor: {n} hosts down"),
        });
        for r in rows {
            lines.push(format!("{} — {}", r.display(), state_text(&r.state)));
        }
    }
    lines.push("Click: open  ·  Right-click: check now".into());
    lines.join("\n")
}

/// `Up · 12 ms`, `Down · No reply`, `Checking…`.
pub(crate) fn state_text(state: &HostState) -> String {
    match state {
        HostState::Checking => "Checking…".to_string(),
        HostState::Up { latency_ms: None } => "Up".to_string(),
        HostState::Up {
            latency_ms: Some(ms),
        } => format!("Up · {}", host_monitor::format_latency(*ms)),
        HostState::Down { reason } => format!("Down · {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_lists_every_host() {
        let rows = vec![
            HostStatus {
                host: "nas.lan".into(),
                label: "NAS".into(),
                state: HostState::Up {
                    latency_ms: Some(2.4),
                },
            },
            HostStatus {
                host: "10.0.0.9".into(),
                label: String::new(),
                state: HostState::Down {
                    reason: "No reply".into(),
                },
            },
        ];
        assert_eq!(
            tooltip(&rows, None),
            "Host Monitor: 1 host down\nNAS — Up · 2 ms\n10.0.0.9 — Down · No reply\nClick: open  ·  Right-click: check now"
        );
        assert!(tooltip(&[], None).starts_with("Host Monitor: no hosts yet"));
        assert!(tooltip(&rows, Some("ping is not installed")).contains("not installed"));
        assert_eq!(state_text(&HostState::Checking), "Checking…");
    }
}
