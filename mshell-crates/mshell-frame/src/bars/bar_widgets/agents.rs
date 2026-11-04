//! AI agents bar pill — plan usage of Claude Code (and Codex when signed
//! in), replacing the `model-usage` WASM plugin.
//!
//! Shows the window closest to its limit as a percentage, tinted on the
//! DESIGN.md §2 ladder by where the current burn rate takes it (warn when
//! on course for the limit, danger when it is minutes/hours away or hit).
//! Beside it, one lamp per running Claude Code session (needs-you in the
//! error tint, working in primary, idle dim; `+N` past eight). Data comes
//! from the shared [`USAGE_POLL`] and [`SESSIONS_POLL`], which run only
//! while a pill or the menu is mapped. Click opens `MenuType::Agents`;
//! right-click asks for fresh limits.

use crate::agents::limits::{self, Pace, PaceStatus};
use crate::agents::local_stats::compact_count;
use crate::agents::sessions::SessionStatus;
use crate::agents::{SESSIONS_POLL, SessionsSnapshot, USAGE_POLL, UsageSnapshot, request_refresh};
use mshell_utils::shared_poll::PollSubscription;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::rc::Rc;

pub(crate) struct AgentsModel {
    snapshot: Option<UsageSnapshot>,
    sessions: Option<SessionsSnapshot>,
    _poll: Rc<PollSubscription<UsageSnapshot>>,
    _sessions_poll: Rc<PollSubscription<SessionsSnapshot>>,
}

#[derive(Debug)]
pub(crate) enum AgentsInput {
    Clicked,
    Refresh,
}

#[derive(Debug)]
pub(crate) enum AgentsOutput {
    Clicked,
}

pub(crate) struct AgentsInit {}

#[derive(Debug)]
pub(crate) enum AgentsCommandOutput {
    Usage(UsageSnapshot),
    Sessions(SessionsSnapshot),
}

#[relm4::component(pub)]
impl Component for AgentsModel {
    type CommandOutput = AgentsCommandOutput;
    type Input = AgentsInput;
    type Output = AgentsOutput;
    type Init = AgentsInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "agents-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(AgentsInput::Clicked);
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    // The cluster carries the severity class so icon + label
                    // tint together (DESIGN.md §4.4).
                    #[name = "cluster"]
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 4,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        add_css_class: "agents-bar-cluster",

                        gtk::Image {
                            set_icon_name: Some("ai-agents-symbolic"),
                            add_css_class: "agents-bar-icon",
                        },
                        #[name = "label"]
                        gtk::Label {
                            add_css_class: "agents-bar-label",
                            set_visible: false,
                        },
                    },
                    // Session lamps sit outside the severity cluster so a
                    // limit warning never recolours them.
                    #[name = "lamps"]
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 3,
                        set_valign: gtk::Align::Center,
                        set_margin_start: 6,
                        set_visible: false,
                        add_css_class: "agents-bar-lamps",
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
        let poll = Rc::new(USAGE_POLL.subscribe());
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
                    let _ = out.send(AgentsCommandOutput::Usage(s));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = rx.changed() => if changed.is_err() { break },
                }
            }
        });

        let sessions_poll = Rc::new(SESSIONS_POLL.subscribe());
        {
            let p = sessions_poll.clone();
            root.connect_map(move |_| p.set_active(true));
            let p = sessions_poll.clone();
            root.connect_unmap(move |_| p.set_active(false));
        }
        let mut srx = sessions_poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let snap = srx.borrow_and_update().clone();
                if let Some(s) = snap {
                    let _ = out.send(AgentsCommandOutput::Sessions(s));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = srx.changed() => if changed.is_err() { break },
                }
            }
        });

        let right = gtk::GestureClick::new();
        right.set_button(gtk::gdk::BUTTON_SECONDARY);
        {
            let s = sender.clone();
            right.connect_released(move |_, _, _, _| s.input(AgentsInput::Refresh));
        }
        root.add_controller(right);

        let model = AgentsModel {
            snapshot: None,
            sessions: None,
            _poll: poll,
            _sessions_poll: sessions_poll,
        };
        let widgets = view_output!();
        apply_visual(&widgets, &root, None, None);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AgentsInput::Clicked => {
                let _ = sender.output(AgentsOutput::Clicked);
            }
            AgentsInput::Refresh => request_refresh(),
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
            AgentsCommandOutput::Usage(s) => {
                if self.snapshot.as_ref() != Some(&s) {
                    self.snapshot = Some(s);
                    apply_visual(
                        widgets,
                        root,
                        self.snapshot.as_ref(),
                        self.sessions.as_ref(),
                    );
                }
            }
            AgentsCommandOutput::Sessions(s) => {
                if self.sessions.as_ref() != Some(&s) {
                    self.sessions = Some(s);
                    apply_lamps(&widgets.lamps, self.sessions.as_ref());
                    root.set_tooltip_text(Some(&tooltip(
                        self.snapshot.as_ref(),
                        self.sessions.as_ref(),
                    )));
                }
            }
        }
    }
}

/// The pill's percentage and severity class, from the headline window.
pub(crate) fn headline(s: &UsageSnapshot) -> Option<(String, &'static str)> {
    let (p, w) = s.headline()?;
    let rate = (p.provider == limits::Provider::Claude && w.kind == limits::WindowKind::Session)
        .then_some(s.session_rate)
        .flatten();
    let pace = limits::pace(w, s.now_ms, rate);
    Some((format!("{:.0}%", w.percent), pace.status.severity_class()))
}

/// Most lamps drawn; the rest collapse into a `+N` label.
const MAX_LAMPS: usize = 8;

/// One CSS state class per lamp (sessions are already needs-you first),
/// and how many didn't fit.
pub(crate) fn lamps(s: &SessionsSnapshot) -> (Vec<&'static str>, usize) {
    let classes: Vec<_> = s
        .live
        .iter()
        .take(MAX_LAMPS)
        .map(|x| x.status.css_class())
        .collect();
    (classes, s.live.len().saturating_sub(MAX_LAMPS))
}

fn apply_lamps(container: &gtk::Box, s: Option<&SessionsSnapshot>) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let Some(s) = s.filter(|s| !s.live.is_empty()) else {
        container.set_visible(false);
        return;
    };
    let (classes, more) = lamps(s);
    for class in classes {
        let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        lamp.set_css_classes(&["agents-lamp", class]);
        lamp.set_valign(gtk::Align::Center);
        container.append(&lamp);
    }
    if more > 0 {
        let l = gtk::Label::new(Some(&format!("+{more}")));
        l.add_css_class("agents-bar-label");
        container.append(&l);
    }
    container.set_visible(true);
}

fn apply_visual(
    widgets: &AgentsModelWidgets,
    root: &gtk::Box,
    s: Option<&UsageSnapshot>,
    sessions: Option<&SessionsSnapshot>,
) {
    let cluster = &widgets.cluster;
    for c in ["calm", "warn", "danger", "stale"] {
        cluster.remove_css_class(c);
    }
    match s.and_then(|s| headline(s).map(|h| (s, h))) {
        Some((s, (text, severity))) => {
            widgets.label.set_label(&text);
            widgets.label.set_visible(true);
            cluster.add_css_class(severity);
            if s.claude.error.is_some() {
                cluster.add_css_class("stale");
            }
        }
        None => {
            widgets.label.set_visible(false);
            cluster.add_css_class("calm");
        }
    }
    root.set_tooltip_text(Some(&tooltip(s, sessions)));
}

fn tooltip(s: Option<&UsageSnapshot>, sessions: Option<&SessionsSnapshot>) -> String {
    let mut lines = Vec::new();
    if let Some(ss) = sessions.filter(|ss| !ss.live.is_empty()) {
        let (w, b, i) = (
            ss.count(SessionStatus::Waiting),
            ss.count(SessionStatus::Busy),
            ss.count(SessionStatus::Idle),
        );
        lines.push(format!(
            "Claude Code sessions: {w} need you · {b} working · {i} idle"
        ));
        for x in ss
            .live
            .iter()
            .filter(|x| x.status == SessionStatus::Waiting)
        {
            lines.push(format!("  {} needs you", x.name));
        }
    }
    match s {
        None => lines.push("AI agents: loading…".to_string()),
        Some(s) => {
            for p in std::iter::once(&s.claude).chain(s.codex.as_ref()) {
                match &p.usage {
                    Some(u) => {
                        lines.push(u.plan.clone());
                        for w in &u.windows {
                            let rate = (p.provider == limits::Provider::Claude
                                && w.kind == limits::WindowKind::Session)
                                .then_some(s.session_rate)
                                .flatten();
                            let pace: Pace = limits::pace(w, s.now_ms, rate);
                            let mut line = format!("  {} {:.0}%", w.label, w.percent);
                            if let Some(rem) = pace.remaining_ms
                                && pace.status != PaceStatus::Idle
                            {
                                line.push_str(&format!(" · resets in {}", limits::span(rem)));
                            }
                            lines.push(line);
                        }
                    }
                    None => lines.push(p.provider.name().to_string()),
                }
                if let Some(e) = &p.error {
                    lines.push(format!("  {}", e.message(p.provider)));
                }
            }
            let l = &s.local;
            lines.push(format!(
                "Today: {} prompts · {} tokens",
                l.prompts,
                compact_count(l.tokens.total())
            ));
        }
    }
    lines.push("Click: open AI agents  ·  Right-click: refresh limits".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::ProviderLimits;
    use crate::agents::limits::{LimitWindow, PlanUsage, Provider, Severity, WindowKind};
    use crate::agents::local_stats::LocalStats;

    fn snap(windows: Vec<(WindowKind, f64)>) -> UsageSnapshot {
        UsageSnapshot {
            claude: ProviderLimits {
                provider: Provider::Claude,
                usage: Some(PlanUsage {
                    provider: Provider::Claude,
                    plan: "Claude Max 5x".into(),
                    windows: windows
                        .into_iter()
                        .map(|(kind, percent)| LimitWindow {
                            kind,
                            label: String::new(),
                            percent,
                            resets_at: None,
                            period_ms: limits::HOUR_MS,
                            severity: Severity::Normal,
                        })
                        .collect(),
                }),
                error: None,
                fetched_at: Some(0),
            },
            codex: None,
            local: LocalStats::default(),
            session_rate: None,
            now_ms: 0,
        }
    }

    #[test]
    fn pill_shows_the_window_closest_to_its_limit() {
        let s = snap(vec![
            (WindowKind::Session, 12.0),
            (WindowKind::Weekly, 64.4),
            (WindowKind::Other, 99.0),
        ]);
        assert_eq!(headline(&s), Some(("64%".to_string(), "calm")));
        let s = snap(vec![(WindowKind::Session, 100.0)]);
        assert_eq!(headline(&s), Some(("100%".to_string(), "danger")));
        assert_eq!(headline(&snap(vec![])), None);
    }

    #[test]
    fn one_lamp_per_session_then_overflow() {
        use crate::agents::sessions::{SessionStatus, test_session};
        let mut s = SessionsSnapshot {
            live: vec![
                test_session(SessionStatus::Waiting, 0),
                test_session(SessionStatus::Busy, 0),
            ],
            ..SessionsSnapshot::default()
        };
        assert_eq!(lamps(&s), (vec!["waiting", "busy"], 0));
        s.live = (0..11)
            .map(|_| test_session(SessionStatus::Idle, 0))
            .collect();
        let (l, more) = lamps(&s);
        assert_eq!((l.len(), more), (8, 3));
    }
}
