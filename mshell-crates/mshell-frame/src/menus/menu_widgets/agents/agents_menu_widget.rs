//! AI agents menu — the panel for `MenuType::Agents`.
//!
//! Usage: one card per provider (Claude, plus Codex when signed in) with a
//! row per plan window — used percent on a severity-tinted bar, where the
//! current burn rate takes it by the reset, and the reset time + the
//! per-hour budget that would last until then. Below, today's local Claude
//! Code stats (prompts, tokens, tokens per model).
//!
//! The menu subscribes to the shared [`USAGE_POLL`] only while revealed.

use crate::agents::limits::{self, LimitWindow, PaceStatus, Provider, WindowKind};
use crate::agents::local_stats::compact_count;
use crate::agents::{ProviderLimits, USAGE_POLL, UsageSnapshot, request_refresh};
use mshell_process::SpawnReaped;
use mshell_utils::shared_poll::PollSubscription;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct AgentsMenuWidgetModel {
    usage_box: gtk::Box,
    plan_label: gtk::Label,
    snapshot: Option<UsageSnapshot>,
    poll: PollSubscription<UsageSnapshot>,
}

#[derive(Debug)]
pub(crate) enum AgentsMenuWidgetInput {
    Refresh,
    OpenDashboard,
    /// Sent by the host menu on show/hide; gates the shared poll.
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum AgentsMenuWidgetOutput {}

pub(crate) struct AgentsMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum AgentsMenuWidgetCommandOutput {
    Usage(UsageSnapshot),
}

#[relm4::component(pub(crate))]
impl Component for AgentsMenuWidgetModel {
    type CommandOutput = AgentsMenuWidgetCommandOutput;
    type Input = AgentsMenuWidgetInput;
    type Output = AgentsMenuWidgetOutput;
    type Init = AgentsMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "agents-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("ai-agents-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_xalign: 0.0,
                    set_label: "AI Agents",
                },
                #[local_ref]
                plan_label -> gtk::Label {
                    add_css_class: "panel-header-meta",
                    set_valign: gtk::Align::Center,
                },
                #[name = "refresh_btn"]
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_icon_name: "view-refresh-symbolic",
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input(AgentsMenuWidgetInput::Refresh);
                    },
                },
            },

            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_max_content_height: 560,
                set_propagate_natural_height: true,

                #[local_ref]
                usage_box -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },
            },

            // Footer: its own region below the list (DESIGN.md §5).
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::End,
                set_margin_top: 6,
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Usage page",
                    connect_clicked[sender] => move |_| {
                        sender.input(AgentsMenuWidgetInput::OpenDashboard);
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
        let poll = USAGE_POLL.subscribe();
        let mut rx = poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let snap = rx.borrow_and_update().clone();
                if let Some(s) = snap {
                    let _ = out.send(AgentsMenuWidgetCommandOutput::Usage(s));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = rx.changed() => if changed.is_err() { break },
                }
            }
        });

        let usage_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let plan_label = gtk::Label::new(None);
        let model = AgentsMenuWidgetModel {
            usage_box: usage_box.clone(),
            plan_label: plan_label.clone(),
            snapshot: None,
            poll,
        };
        let widgets = view_output!();
        mshell_common::a11y::name(&widgets.refresh_btn, "Refresh limits");
        rebuild_usage(&model.usage_box, None);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AgentsMenuWidgetInput::Refresh => request_refresh(),
            AgentsMenuWidgetInput::OpenDashboard => open_url(Provider::Claude.dashboard_url()),
            AgentsMenuWidgetInput::ParentRevealChanged(visible) => {
                self.poll.set_active(visible);
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AgentsMenuWidgetCommandOutput::Usage(s) => {
                if self.snapshot.as_ref() != Some(&s) {
                    let plan = s
                        .claude
                        .usage
                        .as_ref()
                        .map(|u| u.plan.as_str())
                        .unwrap_or("");
                    self.plan_label.set_label(plan);
                    rebuild_usage(&self.usage_box, Some(&s));
                    self.snapshot = Some(s);
                }
            }
        }
    }
}

pub(crate) fn open_url(url: &str) {
    if let Err(e) = std::process::Command::new("xdg-open")
        .arg(url)
        .spawn_reaped()
    {
        tracing::warn!(error = %e, "xdg-open failed");
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(classes);
    l.set_halign(gtk::Align::Start);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l
}

fn section(title: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(title));
    l.add_css_class("section-label-menu-widget");
    l.set_halign(gtk::Align::Start);
    l.set_xalign(0.0);
    l
}

fn boxed_list() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_css_classes(&["ok-boxed-list", "compact"]);
    b
}

/// Rebuild the Usage page from `s` (`None` = still loading).
fn rebuild_usage(container: &gtk::Box, s: Option<&UsageSnapshot>) {
    clear(container);
    let Some(s) = s else {
        container.append(&label("Loading…", &["label-small", "dim-label"]));
        return;
    };
    provider_card(container, &s.claude, s);
    if let Some(codex) = &s.codex {
        provider_card(container, codex, s);
    }
    today_card(container, s);
}

fn provider_card(container: &gtk::Box, p: &ProviderLimits, s: &UsageSnapshot) {
    let title = p
        .usage
        .as_ref()
        .map_or_else(|| p.provider.name().to_string(), |u| u.plan.clone());
    container.append(&section(&title));
    let list = boxed_list();
    if let Some(u) = &p.usage {
        for w in &u.windows {
            let rate = (p.provider == Provider::Claude && w.kind == WindowKind::Session)
                .then_some(s.session_rate)
                .flatten();
            list.append(&window_row(w, s.now_ms, rate, p.error.is_some()));
        }
    }
    if let Some(e) = &p.error {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        row.append(&label(&e.message(p.provider), &["label-small-error"]));
        if let Some(at) = p.fetched_at {
            row.append(&label(
                &format!(
                    "Showing the reading from {} ago",
                    limits::span(s.now_ms - at)
                ),
                &["label-small", "dim-label"],
            ));
        }
        list.append(&row);
    }
    if p.usage.is_none() && p.error.is_none() {
        list.append(&label("Loading…", &["label-small", "dim-label"]));
    }
    container.append(&list);
}

fn window_row(w: &LimitWindow, now_ms: i64, rate: Option<f64>, stale: bool) -> gtk::Box {
    let pace = limits::pace(w, now_ms, rate);
    let severity = pace.status.severity_class();
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    row.add_css_class("agents-window-row");
    if stale {
        row.add_css_class("stale");
    }

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = label(&w.label, &["label-medium-bold"]);
    name.set_hexpand(true);
    top.append(&name);
    let pct = gtk::Label::new(Some(&format!("{:.0}%", w.percent)));
    pct.set_css_classes(&["agents-window-pct", severity]);
    top.append(&pct);
    row.append(&top);

    let bar = gtk::ProgressBar::new();
    bar.set_css_classes(&["agents-bar", severity]);
    bar.set_fraction((w.percent / 100.0).clamp(0.0, 1.0));
    row.append(&bar);

    row.append(&label(&limits::forecast_text(w, &pace), &["label-small"]));
    let reset = limits::reset_text(w, &pace);
    if !reset.is_empty() && pace.status != PaceStatus::Idle {
        row.append(&label(&reset, &["label-small", "dim-label"]));
    }
    row
}

fn today_card(container: &gtk::Box, s: &UsageSnapshot) {
    container.append(&section("Today · Claude Code"));
    let list = boxed_list();
    let l = &s.local;
    list.append(&stat_row(
        "Prompts",
        &format!(
            "{} in {} session{}",
            l.prompts,
            l.sessions,
            if l.sessions == 1 { "" } else { "s" }
        ),
    ));
    list.append(&stat_row(
        "Tokens",
        &format!(
            "{} · {} output",
            compact_count(l.tokens.total()),
            compact_count(l.tokens.output)
        ),
    ));
    for (model, n) in l.by_model.iter().take(4) {
        list.append(&stat_row(model, &compact_count(*n)));
    }
    container.append(&list);
}

fn stat_row(title: &str, value: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let t = label(title, &["label-medium"]);
    t.set_hexpand(true);
    t.set_valign(gtk::Align::Center);
    row.append(&t);
    let v = gtk::Label::new(Some(value));
    v.set_css_classes(&["label-medium", "dim-label", "agents-stat-value"]);
    v.set_valign(gtk::Align::Center);
    row.append(&v);
    row
}
