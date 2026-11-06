//! AI agents menu — the panel for `MenuType::Agents`.
//!
//! Usage: one card per provider (Claude, plus Codex when signed in) with a
//! row per plan window — used percent on a severity-tinted bar, where the
//! current burn rate takes it by the reset, and the reset time + the
//! per-hour budget that would last until then. Below, today's local Claude
//! Code stats (prompts, tokens, tokens per model).
//!
//! Below that, the last 30 days: where the tokens went per project and
//! by hour of day ([`HISTORY_POLL`]).
//!
//! Sessions (second tab of the §12 segmented control): running Claude
//! Code sessions grouped Needs you / Working / Idle — click one to jump to
//! its terminal, unfold one to read Claude's last reply — recent saved
//! sessions to resume, a full-text search over every prompt typed, and
//! (header +) a new session in a recent project (see
//! [`super::sessions_page`]).
//!
//! Memory (third tab): every CLAUDE.md, click to edit
//! ([`super::memory_page`]).
//!
//! Plugins (fourth tab): installed Claude Code plugins and bare skills,
//! read-only ([`super::plugins_page`]).
//!
//! The menu subscribes to the shared polls only while revealed.

use crate::agents::limits::{self, LimitWindow, PaceStatus, Provider, WindowKind};
use crate::agents::local_stats::compact_count;
use crate::agents::memory::MemoryFile;
use crate::agents::plugins::Inventory;
use crate::agents::sessions;
use crate::agents::sessions::SavedSession;
use crate::agents::{
    HISTORY_DAYS, HISTORY_POLL, HistorySnapshot, ProviderLimits, SESSIONS_POLL, SessionsSnapshot,
    USAGE_POLL, UsageSnapshot, pricing, request_refresh,
};
use crate::common_widgets::segmented::{Segmented, SegmentedStyle};
use crate::menus::menu_widgets::agents::memory_page::rebuild_memory;
use crate::menus::menu_widgets::agents::plugins_page::rebuild_plugins;
use crate::menus::menu_widgets::agents::sessions_page::{SessionAction, SessionsPage};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use mshell_config::schema::menu_widgets::MenuWidget;
use mshell_utils::shared_poll::PollSubscription;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct AgentsMenuWidgetModel {
    usage_box: gtk::Box,
    /// Built once; updated in place so focus and scroll survive a poll.
    sessions_page: SessionsPage,
    /// Usage | Sessions, built once (the label and active state change).
    tabs: Segmented,
    plan_label: gtk::Label,
    snapshot: Option<UsageSnapshot>,
    sessions: Option<SessionsSnapshot>,
    history: Option<HistorySnapshot>,
    memory_box: gtk::Box,
    plugins_box: gtk::Box,
    /// 0 = Usage, 1 = Sessions, 2 = Memory, 3 = Plugins.
    tab: u8,
    /// Bumped per search; an older search's result is dropped.
    search_gen: u64,
    /// Result of the last jump / resume, shown atop the Sessions page.
    notice: Option<String>,
    poll: PollSubscription<UsageSnapshot>,
    sessions_poll: PollSubscription<SessionsSnapshot>,
    history_poll: PollSubscription<HistorySnapshot>,
    _settings: mshell_common::scoped_effects::EffectScope,
}

#[derive(Debug)]
pub(crate) enum AgentsMenuWidgetInput {
    SelectTab(u8),
    Session(SessionAction),
    /// The Sessions page's search entry changed.
    Search(String),
    /// Header +: show / hide "New session in…".
    ToggleNew,
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
    History(HistorySnapshot),
    /// `(generation, query, hits)` of a full-text search.
    SearchDone(u64, String, Vec<SavedSession>),
    Memory(Vec<MemoryFile>),
    Plugins(Inventory),
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
            #[local_ref]
            header_root -> gtk::Box {},

            #[local_ref]
            tabs_root -> gtk::Box {},

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

            #[name = "memory_scroll"]
            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_propagate_natural_height: true,
                set_visible: false,

                #[local_ref]
                memory_box -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },
            },

            #[name = "plugins_scroll"]
            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_propagate_natural_height: true,
                set_visible: false,

                #[local_ref]
                plugins_box -> gtk::Box {
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

        let history_poll = HISTORY_POLL.subscribe();
        let mut hrx = history_poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let snap = hrx.borrow_and_update().clone();
                if let Some(s) = snap {
                    let _ = out.send(AgentsMenuWidgetCommandOutput::History(s));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = hrx.changed() => if changed.is_err() { break },
                }
            }
        });

        let usage_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let sessions_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let memory_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let plugins_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let s = sender.clone();
        let tabs = Segmented::new(
            &["Usage", "Sessions", "Memory", "Plugins"],
            0,
            SegmentedStyle::Capsule,
            move |i| {
                s.input(AgentsMenuWidgetInput::SelectTab(i as u8));
            },
        );
        let tabs_root = tabs.root().clone();
        let s = sender.clone();
        let header = PanelHeader::new("ai-agents-symbolic", MenuWidget::Agents.display_name())
            .with_meta()
            .action("list-add-symbolic", "New Claude Code session", {
                let s = sender.clone();
                move || s.input(AgentsMenuWidgetInput::ToggleNew)
            })
            .action("view-refresh-symbolic", "Refresh", move || {
                s.input(AgentsMenuWidgetInput::Refresh);
            });
        let header_root = header.root().clone();
        let plan_label = header.meta().cloned().unwrap_or_default();
        let s = sender.clone();
        let s2 = sender.clone();
        let sessions_page = SessionsPage::new(
            &sessions_box,
            move |a| s.input(AgentsMenuWidgetInput::Session(a)),
            move |q| s2.input(AgentsMenuWidgetInput::Search(q)),
        );
        let mut model = AgentsMenuWidgetModel {
            usage_box: usage_box.clone(),
            sessions_page,
            tabs,
            plan_label: plan_label.clone(),
            snapshot: None,
            sessions: None,
            history: None,
            memory_box: memory_box.clone(),
            plugins_box: plugins_box.clone(),
            tab: 0,
            search_gen: 0,
            notice: None,
            poll,
            sessions_poll,
            history_poll,
            _settings: crate::agents::settings::watch_settings(),
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.usage_scroll, 560);
        crate::menus::list_fit::mark_menu_list(&widgets.sessions_scroll, 560);
        crate::menus::list_fit::mark_menu_list(&widgets.memory_scroll, 560);
        crate::menus::list_fit::mark_menu_list(&widgets.plugins_scroll, 560);
        rebuild_usage(&model.usage_box, None, None);
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
                widgets.memory_scroll.set_visible(tab == 2);
                widgets.plugins_scroll.set_visible(tab == 3);
                self.update_tabs();
                match tab {
                    2 => self.load_memory(&sender),
                    3 => self.load_plugins(&sender),
                    _ => {}
                }
            }
            AgentsMenuWidgetInput::Search(q) => {
                self.search_gen += 1;
                let generation = self.search_gen;
                if q.is_empty() {
                    let now = chrono::Utc::now().timestamp_millis();
                    self.sessions_page.set_matches("", &[], now);
                } else {
                    sender.oneshot_command(async move {
                        let query = q.clone();
                        let hits = tokio::task::spawn_blocking(move || {
                            crate::agents::search_saved(&query, SEARCH_LIMIT)
                        })
                        .await
                        .unwrap_or_default();
                        AgentsMenuWidgetCommandOutput::SearchDone(generation, q, hits)
                    });
                }
            }
            AgentsMenuWidgetInput::ToggleNew => {
                if self.tab != 1 {
                    sender.input(AgentsMenuWidgetInput::SelectTab(1));
                }
                self.sessions_page.toggle_new();
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
                HISTORY_POLL.refresh_now();
                match self.tab {
                    2 => self.load_memory(&sender),
                    3 => self.load_plugins(&sender),
                    _ => {}
                }
            }
            AgentsMenuWidgetInput::OpenDashboard => open_url(Provider::Claude.dashboard_url()),
            AgentsMenuWidgetInput::ParentRevealChanged(visible) => {
                self.poll.set_active(visible);
                self.sessions_poll.set_active(visible);
                self.history_poll.set_active(visible);
                if !visible {
                    self.notice = None;
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
            AgentsMenuWidgetCommandOutput::Usage(s) => {
                if self.snapshot.as_ref() != Some(&s) {
                    let plan = s
                        .claude
                        .usage
                        .as_ref()
                        .map(|u| u.plan.as_str())
                        .unwrap_or("");
                    self.plan_label.set_label(plan);
                    rebuild_usage(&self.usage_box, Some(&s), self.history.as_ref());
                    self.snapshot = Some(s);
                }
            }
            AgentsMenuWidgetCommandOutput::History(h) => {
                if self.history.as_ref() != Some(&h) {
                    self.history = Some(h);
                    if self.snapshot.is_some() {
                        rebuild_usage(
                            &self.usage_box,
                            self.snapshot.as_ref(),
                            self.history.as_ref(),
                        );
                    }
                }
            }
            AgentsMenuWidgetCommandOutput::SearchDone(generation, q, hits) => {
                if generation == self.search_gen {
                    let now = chrono::Utc::now().timestamp_millis();
                    self.sessions_page.set_matches(&q, &hits, now);
                }
            }
            AgentsMenuWidgetCommandOutput::Memory(files) => {
                rebuild_memory(&self.memory_box, Some(&files), self.open_handler(&sender));
            }
            AgentsMenuWidgetCommandOutput::Plugins(inv) => {
                rebuild_plugins(&self.plugins_box, Some(&inv), self.open_handler(&sender));
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

/// Most full-text search hits listed.
const SEARCH_LIMIT: usize = 30;

impl AgentsMenuWidgetModel {
    /// Read the CLAUDE.md list off the GTK thread.
    fn load_memory(&self, sender: &ComponentSender<Self>) {
        rebuild_memory(&self.memory_box, None, self.open_handler(sender));
        sender.oneshot_command(async {
            let files = tokio::task::spawn_blocking(crate::agents::memory_files_now)
                .await
                .unwrap_or_default();
            AgentsMenuWidgetCommandOutput::Memory(files)
        });
    }

    /// Read the plugin / skill inventory off the GTK thread.
    fn load_plugins(&self, sender: &ComponentSender<Self>) {
        rebuild_plugins(&self.plugins_box, None, self.open_handler(sender));
        sender.oneshot_command(async {
            let inv = tokio::task::spawn_blocking(crate::agents::plugins_now)
                .await
                .unwrap_or_default();
            AgentsMenuWidgetCommandOutput::Plugins(inv)
        });
    }

    fn open_handler(&self, sender: &ComponentSender<Self>) -> std::rc::Rc<dyn Fn(String)> {
        let s = sender.clone();
        std::rc::Rc::new(move |path| {
            s.input(AgentsMenuWidgetInput::Session(SessionAction::Open(path)));
        })
    }

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
        self.tabs.set_label(1, &format!("Sessions · {live}"));
        self.tabs.set_active(usize::from(self.tab));
    }
}

/// Run a row's action (blocking: tmux / kitty / margo IPC); `Some` is the
/// reason it didn't work.
fn run_session_action(action: SessionAction) -> Option<String> {
    match action {
        SessionAction::Focus(pid) => (!sessions::focus_session(pid))
            .then(|| "Couldn't find that session's terminal window".to_string()),
        SessionAction::Resume {
            session_id,
            cwd,
            config_dir,
        } => {
            let dir = (!config_dir.as_os_str().is_empty()).then_some(config_dir);
            sessions::resume(&session_id, &cwd, dir.as_deref()).err()
        }
        SessionAction::New { cwd, config_dir } => {
            let dir = (!config_dir.as_os_str().is_empty()).then_some(config_dir);
            sessions::resume("", &cwd, dir.as_deref()).err()
        }
        SessionAction::Open(target) => mshell_process::desktop::open(&target)
            .err()
            .map(|e| format!("Couldn't open it: {e}")),
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

/// Rebuild the Usage page from `s` (`None` = still loading) and the
/// usage history `h` (once read).
fn rebuild_usage(container: &gtk::Box, s: Option<&UsageSnapshot>, h: Option<&HistorySnapshot>) {
    clear(container);
    let Some(s) = s else {
        container.append(&label("Loading…", &["label-small", "dim-label"]));
        return;
    };
    provider_card(container, &s.claude, s.claude_label.as_deref(), s);
    for a in &s.others {
        provider_card(container, &a.limits, Some(&a.label), s);
    }
    if let Some(codex) = &s.codex {
        provider_card(container, codex, None, s);
    }
    today_card(container, s);
    if let Some(h) = h {
        history_card(container, h);
    }
}

/// Most projects listed under "where it goes".
const TOP_PROJECTS: usize = 5;

/// Last 30 days: tokens (and estimated cost) per project, then a
/// tokens-by-hour-of-day chart.
fn history_card(container: &gtk::Box, h: &HistorySnapshot) {
    let prices = pricing::current();
    let currency = crate::agents::settings::currency();
    let value = |t: &crate::agents::history::Totals| {
        let mut v = compact_count(t.tokens());
        if let Some(c) = t
            .cost(&prices)
            .and_then(|c| pricing::money(c, currency, prices.eur))
        {
            v.push_str(&format!(" · ~{c}"));
        }
        v
    };
    container.append(&section("Tokens & estimated cost · Claude Code"));
    let list = boxed_list();
    for (title, t) in [
        ("Today", &h.today),
        ("This week", &h.week),
        ("This month", &h.month),
        ("All time", &h.all),
    ] {
        list.append(&stat_row(title, &value(t)));
    }
    list.append(&stat_row(
        "All-time activity",
        &format!(
            "{} sessions · {} messages",
            h.all_sessions,
            compact_count(u64::from(h.all.messages))
        ),
    ));
    container.append(&list);
    container.append(&section("This week"));
    let chart = boxed_list();
    let days: Vec<String> = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .iter()
        .map(|d| d.to_string())
        .collect();
    chart.append(&bar_chart(&h.weekdays, &days));
    container.append(&chart);
    if h.projects.is_empty() {
        return;
    }
    container.append(&section(&format!("Last {HISTORY_DAYS} days · by project")));
    let list = boxed_list();
    for (project, t) in h.projects.iter().take(TOP_PROJECTS) {
        list.append(&stat_row(&sessions::tilde(project), &value(t)));
    }
    container.append(&list);
    container.append(&section(&format!("Last {HISTORY_DAYS} days · by hour")));
    let chart = boxed_list();
    chart.append(&bar_chart(&h.hours, &hour_labels()));
    container.append(&chart);
}

/// `0`, `6`, `12`, `18` under their hours, blank elsewhere.
fn hour_labels() -> Vec<String> {
    (0..24)
        .map(|h| {
            if h % 6 == 0 {
                h.to_string()
            } else {
                String::new()
            }
        })
        .collect()
}

/// One bar per value, scaled to the largest, with a label under each and
/// the value in its tooltip.
pub(crate) fn bar_chart(values: &[u64], labels: &[String]) -> gtk::Box {
    let max = values.iter().copied().max().unwrap_or(0).max(1) as f64;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    row.add_css_class("agents-chart");
    row.set_homogeneous(true);
    for (i, v) in values.iter().enumerate() {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let bar = gtk::ProgressBar::new();
        bar.set_orientation(gtk::Orientation::Vertical);
        bar.set_inverted(true);
        bar.set_css_classes(&["agents-chart-bar"]);
        bar.set_fraction(*v as f64 / max);
        bar.set_halign(gtk::Align::Center);
        bar.set_tooltip_text(Some(&compact_count(*v)));
        col.append(&bar);
        let l = gtk::Label::new(Some(labels.get(i).map_or("", String::as_str)));
        l.set_css_classes(&["label-small", "dim-label", "agents-chart-label"]);
        col.append(&l);
        row.append(&col);
    }
    row
}

/// One provider's (or Claude account's) plan windows. `account` names the
/// account when there are several.
fn provider_card(
    container: &gtk::Box,
    p: &ProviderLimits,
    account: Option<&str>,
    s: &UsageSnapshot,
) {
    let mut title = p
        .usage
        .as_ref()
        .map_or_else(|| p.provider.name().to_string(), |u| u.plan.clone());
    if let Some(a) = account {
        title.push_str(&format!(" · {a}"));
    }
    container.append(&section(&title));
    let list = boxed_list();
    if let Some(u) = &p.usage {
        for w in &u.windows {
            // The recent burn rate is sampled for the primary account.
            let primary = std::ptr::eq(p, &s.claude);
            let rate = (primary && w.kind == WindowKind::Session)
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
