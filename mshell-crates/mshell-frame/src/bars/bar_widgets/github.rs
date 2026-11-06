//! GitHub bar pill — unread notification count (or a contribution
//! figure) via the `gh` CLI.
//!
//! The GitHub mark with the shared read/unread corner dot (DESIGN.md §5
//! "Read / unread markers", the `.notif-dot` the notifications bell uses)
//! and a count beside it: unread notifications by default, or — per
//! Settings → Widgets → GitHub → *Pill shows* — today's contributions, the
//! current streak, the year total, or the open PRs the search rules
//! match. *Hide when zero* hides the pill while that count is 0. The data
//! comes from the shared
//! [`GITHUB_POLL`] (every two minutes) and [`CONTRIB_POLL`] (every 30
//! minutes, only while a contribution metric or the Activity tab is
//! shown), active while a pill or the menu is mapped. Click opens
//! `MenuType::Github`; right-click re-polls.

use crate::github::{self, ContribCalendar, GhError, GithubSnapshot};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GithubConfigStoreFields, GithubPillMetric};
use mshell_utils::shared_poll::{PollSubscription, SharedPoll};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

pub(crate) static GITHUB_POLL: LazyLock<SharedPoll<GithubSnapshot>> = LazyLock::new(|| {
    SharedPoll::new(
        || Duration::from_secs(github::POLL_SECS),
        || Box::pin(github::fetch()),
    )
});

pub(crate) type ContribResult = Result<ContribCalendar, GhError>;

/// The contribution calendar, shared by the pill and the menu's Activity
/// tab.
pub(crate) static CONTRIB_POLL: LazyLock<SharedPoll<ContribResult>> = LazyLock::new(|| {
    SharedPoll::new(
        || Duration::from_secs(github::CONTRIB_POLL_SECS),
        || Box::pin(github::fetch_contributions()),
    )
});

pub(crate) struct GithubModel {
    snapshot: Option<GithubSnapshot>,
    contrib: Option<ContribResult>,
    metric: GithubPillMetric,
    hide_when_zero: bool,
    mapped: Rc<std::cell::Cell<bool>>,
    /// The pill hid itself (zero count): its unmap must not stop the polls
    /// that will bring it back.
    self_hidden: Rc<std::cell::Cell<bool>>,
    _poll: Rc<PollSubscription<GithubSnapshot>>,
    contrib_poll: PollSubscription<ContribResult>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum GithubInput {
    Clicked,
    Refresh,
    MetricChanged,
    Mapped(bool),
}

#[derive(Debug)]
pub(crate) enum GithubOutput {
    Clicked,
}

pub(crate) struct GithubInit {}

#[derive(Debug)]
pub(crate) enum GithubCommandOutput {
    Snapshot(GithubSnapshot),
    Contrib(ContribResult),
}

#[relm4::component(pub)]
impl Component for GithubModel {
    type CommandOutput = GithubCommandOutput;
    type Input = GithubInput;
    type Output = GithubOutput;
    type Init = GithubInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "github-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(GithubInput::Clicked);
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Overlay {
                        gtk::Image {
                            set_icon_name: Some("github-symbolic"),
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,
                        },
                        // Corner dot — decorative, never a click target.
                        #[name = "dot"]
                        add_overlay = &gtk::Box {
                            set_halign: gtk::Align::End,
                            set_valign: gtk::Align::Start,
                            set_can_target: false,
                            set_visible: false,
                            set_css_classes: &["notif-dot", "unread"],
                        },
                    },
                    #[name = "count"]
                    gtk::Label {
                        add_css_class: "github-bar-label",
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
        let mut effects = EffectScope::new();
        // Rules first, so the very first poll already uses them.
        watch_pr_rules(&mut effects);
        let poll = Rc::new(GITHUB_POLL.subscribe());
        let mapped = Rc::new(std::cell::Cell::new(false));
        let self_hidden = Rc::new(std::cell::Cell::new(false));
        {
            let (p, s) = (poll.clone(), sender.clone());
            root.connect_map(move |_| {
                p.set_active(true);
                s.input(GithubInput::Mapped(true));
            });
            let (p, s, hidden) = (poll.clone(), sender.clone(), self_hidden.clone());
            root.connect_unmap(move |_| {
                if hidden.get() {
                    return;
                }
                p.set_active(false);
                s.input(GithubInput::Mapped(false));
            });
        }
        let contrib_poll = CONTRIB_POLL.subscribe();
        let mut crx = contrib_poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let c = crx.borrow_and_update().clone();
                if let Some(c) = c {
                    let _ = out.send(GithubCommandOutput::Contrib(c));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = crx.changed() => if changed.is_err() { break },
                }
            }
        });
        let s = sender.clone();
        effects.push(move |_| {
            let gh = config_manager().config().github();
            let _ = (gh.clone().pill_metric().get(), gh.hide_when_zero().get());
            s.input(GithubInput::MetricChanged);
        });

        let mut rx = poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let snap = rx.borrow_and_update().clone();
                if let Some(s) = snap {
                    let _ = out.send(GithubCommandOutput::Snapshot(s));
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
            right.connect_released(move |_, _, _, _| s.input(GithubInput::Refresh));
        }
        root.add_controller(right);

        let model = GithubModel {
            snapshot: None,
            contrib: None,
            metric: config_manager()
                .config()
                .github()
                .pill_metric()
                .get_untracked(),
            hide_when_zero: config_manager()
                .config()
                .github()
                .hide_when_zero()
                .get_untracked(),
            mapped,
            self_hidden,
            _poll: poll,
            contrib_poll,
            _effects: effects,
        };
        let widgets = view_output!();
        model.apply_visual(&widgets, &root);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            GithubInput::Clicked => {
                let _ = sender.output(GithubOutput::Clicked);
            }
            GithubInput::Refresh => {
                GITHUB_POLL.refresh_now();
                if self.metric.uses_contributions() {
                    CONTRIB_POLL.refresh_now();
                }
            }
            GithubInput::MetricChanged => {
                let gh = config_manager().config().github().get_untracked();
                self.metric = gh.pill_metric;
                self.hide_when_zero = gh.hide_when_zero;
                self.sync_contrib_poll();
                self.apply_visual(widgets, root);
            }
            GithubInput::Mapped(on) => {
                self.mapped.set(on);
                self.sync_contrib_poll();
            }
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
            GithubCommandOutput::Snapshot(s) => {
                if self.snapshot.as_ref() != Some(&s) {
                    self.snapshot = Some(s);
                    self.apply_visual(widgets, root);
                }
            }
            GithubCommandOutput::Contrib(c) => {
                if self.contrib.as_ref() != Some(&c) {
                    self.contrib = Some(c);
                    self.apply_visual(widgets, root);
                }
            }
        }
    }
}

impl GithubModel {
    /// The calendar is only fetched while a contribution metric is shown.
    fn sync_contrib_poll(&self) {
        self.contrib_poll
            .set_active(self.mapped.get() && self.metric.uses_contributions());
    }

    fn apply_visual(&self, widgets: &GithubModelWidgets, root: &gtk::Box) {
        let s = self.snapshot.as_ref();
        let today = chrono::Local::now().date_naive();
        let value = pill_value(self.metric, s, self.contrib.as_ref(), today);
        let unread_metric = self.metric == GithubPillMetric::Unread;
        let shown = value.filter(|v| *v > 0 || !unread_metric);
        widgets.dot.set_visible(unread_metric && shown.is_some());
        widgets.count.set_visible(shown.is_some());
        widgets
            .count
            .set_label(&shown.map(|v| v.to_string()).unwrap_or_default());
        root.remove_css_class("active");
        root.remove_css_class("error");
        if shown.is_some_and(|v| v > 0) {
            root.add_css_class("active");
        }
        let failed = if self.metric.uses_contributions() {
            matches!(self.contrib, Some(Err(_)))
        } else {
            matches!(s.map(|s| &s.notifications), Some(Err(_)))
        };
        if failed {
            root.add_css_class("error");
        }
        let mut text = tooltip(s);
        if let Some(Ok(cal)) = &self.contrib {
            let st = cal.stats(today);
            text = text.replacen(
                "\nClick:",
                &format!(
                    "\nContributions: {} today · {}-day streak · {} this year\nClick:",
                    st.today, st.current_streak, st.total
                ),
                1,
            );
        }
        root.set_tooltip_text(Some(&text));
        // Hide on a known zero only — never while loading or on an error.
        let hide = self.hide_when_zero && value == Some(0) && !failed;
        self.self_hidden.set(hide);
        root.set_visible(!hide);
    }
}

/// Publish the PR rules now and whenever Settings changes them (a change
/// re-polls at once). Every pill and menu calls it; duplicates are no-ops.
pub(crate) fn watch_pr_rules(effects: &mut EffectScope) {
    publish_pr_rules();
    effects.push(move |_| {
        let gh = config_manager().config().github();
        let _ = (gh.clone().pr_rules().get(), gh.excluded_repos().get());
        if publish_pr_rules() {
            GITHUB_POLL.refresh_now();
        }
    });
}

/// Copy the PR rules from the config into the poll; true when they changed.
pub(crate) fn publish_pr_rules() -> bool {
    let gh = config_manager().config().github().get_untracked();
    github::set_pr_rules(github::PrRules {
        rules: gh.pr_rules,
        excluded: gh.excluded_repos,
    })
}

/// The number the pill shows for `metric`, `None` while its data hasn't
/// arrived (or failed).
pub(crate) fn pill_value(
    metric: GithubPillMetric,
    snap: Option<&GithubSnapshot>,
    contrib: Option<&ContribResult>,
    today: chrono::NaiveDate,
) -> Option<u32> {
    let stats = || match contrib {
        Some(Ok(cal)) => Some(cal.stats(today)),
        _ => None,
    };
    match metric {
        GithubPillMetric::Unread => snap.map(|s| s.unread() as u32),
        GithubPillMetric::OpenPrs => snap
            .and_then(|s| s.prs.as_ref().ok())
            .map(|p| p.len() as u32),
        GithubPillMetric::ContribToday => stats().map(|s| s.today),
        GithubPillMetric::ContribStreak => stats().map(|s| s.current_streak),
        GithubPillMetric::ContribTotal => stats().map(|s| s.total),
    }
}

pub(crate) fn tooltip(s: Option<&GithubSnapshot>) -> String {
    let mut lines = Vec::new();
    match s {
        None => lines.push("GitHub: loading…".to_string()),
        Some(s) => {
            match &s.notifications {
                Ok(_) => lines.push(match s.unread() {
                    0 => "GitHub: no unread notifications".to_string(),
                    1 => "GitHub: 1 unread notification".to_string(),
                    n => format!("GitHub: {n} unread notifications"),
                }),
                Err(e) => lines.push(format!("GitHub: {}", e.message())),
            }
            if let Ok(prs) = &s.prs {
                let failing = prs
                    .iter()
                    .filter(|p| p.ci == github::CiState::Failure)
                    .count();
                let mut line = format!("Open PRs: {}", prs.len());
                if failing > 0 {
                    line.push_str(&format!(" ({failing} failing CI)"));
                }
                lines.push(line);
            }
        }
    }
    lines.push("Click: open inbox  ·  Right-click: refresh".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::GhError;

    #[test]
    fn tooltip_reports_unread_and_errors() {
        let s = GithubSnapshot {
            notifications: Err(GhError::Missing),
            prs: Err(GhError::Missing),
            pr_notes: vec![],
        };
        assert!(tooltip(Some(&s)).contains("github-cli"));
        let s = GithubSnapshot {
            notifications: Ok(vec![]),
            prs: Ok(vec![]),
            pr_notes: vec![],
        };
        assert!(tooltip(Some(&s)).starts_with("GitHub: no unread"));
    }

    #[test]
    fn pill_value_follows_the_metric() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let cal = ContribCalendar {
            login: "me".into(),
            total: 900,
            weeks: vec![vec![
                github::ContribDay {
                    date: day.pred_opt().unwrap(),
                    count: 3,
                    level: 1,
                },
                github::ContribDay {
                    date: day,
                    count: 7,
                    level: 2,
                },
            ]],
        };
        let contrib = Ok(cal);
        let snap = GithubSnapshot {
            notifications: Ok(vec![]),
            prs: Ok(vec![]),
            pr_notes: vec![],
        };
        let v = |m| pill_value(m, Some(&snap), Some(&contrib), day);
        assert_eq!(v(GithubPillMetric::Unread), Some(0));
        assert_eq!(v(GithubPillMetric::OpenPrs), Some(0));
        assert_eq!(v(GithubPillMetric::ContribToday), Some(7));
        assert_eq!(v(GithubPillMetric::ContribStreak), Some(2));
        assert_eq!(v(GithubPillMetric::ContribTotal), Some(900));
        let failed: ContribResult = Err(GhError::LoggedOut);
        assert_eq!(
            pill_value(GithubPillMetric::ContribToday, None, Some(&failed), day),
            None
        );
        assert_eq!(pill_value(GithubPillMetric::Unread, None, None, day), None);
    }
}
