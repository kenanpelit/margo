//! AI agents menu — the panel for `MenuType::Agents`.
//!
//! Usage: one card per provider (Claude, plus Codex when signed in) with a
//! row per plan window — used percent on a severity-tinted bar, where the
//! current burn rate takes it by the reset, and the reset time + the
//! per-hour budget that would last until then. Below, today's local Claude
//! Code stats (prompts, tokens, tokens per model).
//!
//! Sessions (second tab of the §12 segmented control): running Claude
//! Code sessions grouped Needs you / Working / Idle — click one to jump to
//! its terminal — and recent saved sessions to resume (see
//! [`super::sessions_page`]).
//!
//! The menu subscribes to the shared [`USAGE_POLL`] / [`SESSIONS_POLL`]
//! only while revealed.

use crate::agents::limits::{self, LimitWindow, PaceStatus, Provider, WindowKind};
use crate::agents::local_stats::compact_count;
use crate::agents::sessions;
use crate::agents::{
    ProviderLimits, SESSIONS_POLL, SessionsSnapshot, USAGE_POLL, UsageSnapshot, request_refresh,
};
use crate::menus::menu_widgets::agents::sessions_page::{SessionAction, SessionsPage};
use mshell_utils::shared_poll::PollSubscription;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct AgentsMenuWidgetModel {
    usage_box: gtk::Box,
    /// Built once; updated in place so focus and scroll survive a poll.
    sessions_page: SessionsPage,
    /// Usage | Sessions, built once (the label and active state change).
    tab_buttons: Vec<gtk::Button>,
    plan_label: gtk::Label,
    snapshot: Option<UsageSnapshot>,
    sessions: Option<SessionsSnapshot>,
    /// 0 = Usage, 1 = Sessions.
    tab: u8,
    /// Result of the last jump / resume, shown atop the Sessions page.
    notice: Option<String>,
    poll: PollSubscription<UsageSnapshot>,
    sessions_poll: PollSubscription<SessionsSnapshot>,
}

#[derive(Debug)]
pub(crate) enum AgentsMenuWidgetInput {
    SelectTab(u8),
    Session(SessionAction),
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
    Sessions(SessionsSnapshot),
    /// A jump / resume finished; `Some` = what went wrong.
    ActionDone(Option<String>),
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

            #[local_ref]
            tabs -> gtk::Box {
                add_css_class: "agents-tabs",
                set_orientation: gtk::Orientation::Horizontal,
                set_homogeneous: true,
                set_spacing: 4,
            },

            #[name = "usage_scroll"]
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

            #[name = "sessions_scroll"]
            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_max_content_height: 560,
                set_propagate_natural_height: true,
                set_visible: false,

                #[local_ref]
                sessions_box -> gtk::Box {
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

        let sessions_poll = SESSIONS_POLL.subscribe();
        let mut srx = sessions_poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let snap = srx.borrow_and_update().clone();
                if let Some(s) = snap {
                    let _ = out.send(AgentsMenuWidgetCommandOutput::Sessions(s));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = srx.changed() => if changed.is_err() { break },
                }
            }
        });

        let usage_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let sessions_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let plan_label = gtk::Label::new(None);
        let s = sender.clone();
        let sessions_page = SessionsPage::new(&sessions_box, move |a| {
            s.input(AgentsMenuWidgetInput::Session(a));
        });
        let tab_buttons = (0..2u8)
            .map(|i| {
                let b = gtk::Button::new();
                let s = sender.clone();
                b.connect_clicked(move |_| s.input(AgentsMenuWidgetInput::SelectTab(i)));
                tabs.append(&b);
                b
            })
            .collect();
        let mut model = AgentsMenuWidgetModel {
            usage_box: usage_box.clone(),
            sessions_page,
            tab_buttons,
            plan_label: plan_label.clone(),
            snapshot: None,
            sessions: None,
            tab: 0,
            notice: None,
            poll,
            sessions_poll,
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.usage_scroll, 560);
        crate::menus::list_fit::mark_menu_list(&widgets.sessions_scroll, 560);
        mshell_common::a11y::name(&widgets.refresh_btn, "Refresh");
        rebuild_usage(&model.usage_box, None);
        model.update_sessions();
        model.update_tabs();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AgentsMenuWidgetInput::SelectTab(tab) => {
                self.tab = tab;
                widgets.usage_scroll.set_visible(tab == 0);
                widgets.sessions_scroll.set_visible(tab == 1);
                self.update_tabs();
            }
            AgentsMenuWidgetInput::Session(action) => {
                self.notice = None;
                sender.oneshot_command(async move {
                    let result = tokio::task::spawn_blocking(move || run_session_action(action))
                        .await
                        .unwrap_or_else(|_| Some("The action failed".into()));
                    AgentsMenuWidgetCommandOutput::ActionDone(result)
                });
            }
            AgentsMenuWidgetInput::Refresh => {
                request_refresh();
                SESSIONS_POLL.refresh_now();
            }
            AgentsMenuWidgetInput::OpenDashboard => open_url(Provider::Claude.dashboard_url()),
            AgentsMenuWidgetInput::ParentRevealChanged(visible) => {
                self.poll.set_active(visible);
                self.sessions_poll.set_active(visible);
                if !visible {
                    self.notice = None;
                }
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
            AgentsMenuWidgetCommandOutput::Sessions(s) => {
                if self.sessions.as_ref() != Some(&s) {
                    self.sessions = Some(s);
                    self.update_sessions();
                    self.update_tabs();
                }
            }
            AgentsMenuWidgetCommandOutput::ActionDone(notice) => {
                self.notice = notice;
                self.update_sessions();
            }
        }
    }
}

impl AgentsMenuWidgetModel {
    fn update_sessions(&mut self) {
        self.sessions_page.update(
            self.sessions.as_ref(),
            self.notice.as_deref(),
            chrono::Utc::now().timestamp_millis(),
        );
    }

    /// The §12 segmented control: Usage | Sessions (with a running count).
    fn update_tabs(&self) {
        let live = self.sessions.as_ref().map_or(0, |s| s.live.len());
        let labels = ["Usage".to_string(), format!("Sessions · {live}")];
        for (i, (b, text)) in self.tab_buttons.iter().zip(&labels).enumerate() {
            b.set_label(text);
            b.set_css_classes(if i as u8 == self.tab {
                &["agents-tab", "active"]
            } else {
                &["agents-tab"]
            });
        }
    }
}

/// Run a row's action (blocking: tmux / kitty / margo IPC); `Some` is the
/// reason it didn't work.
fn run_session_action(action: SessionAction) -> Option<String> {
    match action {
        SessionAction::Focus(pid) => (!sessions::focus_session(pid))
            .then(|| "Couldn't find that session's terminal window".to_string()),
        SessionAction::Resume { session_id, cwd } => sessions::resume(&session_id, &cwd).err(),
    }
}

pub(crate) fn open_url(url: &str) {
    if let Err(e) = mshell_process::desktop::open(url) {
        tracing::warn!(error = %e, url, "open failed");
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
