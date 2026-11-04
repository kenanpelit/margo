//! GitHub bar pill — unread notification count via the `gh` CLI.
//!
//! The GitHub mark with the shared read/unread corner dot (DESIGN.md §5
//! "Read / unread markers", the `.notif-dot` the notifications bell uses)
//! and the unread count beside it. The data comes from the shared
//! [`GITHUB_POLL`], polled every two minutes while a pill or the menu is
//! mapped. Click opens `MenuType::Github`; right-click re-polls.

use crate::github::{self, GithubSnapshot};
use mshell_utils::shared_poll::{PollSubscription, SharedPoll};
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

pub(crate) struct GithubModel {
    snapshot: Option<GithubSnapshot>,
    _poll: Rc<PollSubscription<GithubSnapshot>>,
}

#[derive(Debug)]
pub(crate) enum GithubInput {
    Clicked,
    Refresh,
}

#[derive(Debug)]
pub(crate) enum GithubOutput {
    Clicked,
}

pub(crate) struct GithubInit {}

#[derive(Debug)]
pub(crate) enum GithubCommandOutput {
    Snapshot(GithubSnapshot),
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
        let poll = Rc::new(GITHUB_POLL.subscribe());
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
            _poll: poll,
        };
        let widgets = view_output!();
        apply_visual(&widgets, &root, None);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            GithubInput::Clicked => {
                let _ = sender.output(GithubOutput::Clicked);
            }
            GithubInput::Refresh => GITHUB_POLL.refresh_now(),
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
                    apply_visual(widgets, root, Some(&s));
                    self.snapshot = Some(s);
                }
            }
        }
    }
}

fn apply_visual(widgets: &GithubModelWidgets, root: &gtk::Box, s: Option<&GithubSnapshot>) {
    let unread = s.map_or(0, GithubSnapshot::unread);
    widgets.dot.set_visible(unread > 0);
    widgets.count.set_visible(unread > 0);
    widgets.count.set_label(&unread.to_string());
    root.remove_css_class("active");
    root.remove_css_class("error");
    if unread > 0 {
        root.add_css_class("active");
    }
    if matches!(s.map(|s| &s.notifications), Some(Err(_))) {
        root.add_css_class("error");
    }
    root.set_tooltip_text(Some(&tooltip(s)));
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
                let mut line = format!("My open PRs: {}", prs.len());
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
        };
        assert!(tooltip(Some(&s)).contains("github-cli"));
        let s = GithubSnapshot {
            notifications: Ok(vec![]),
            prs: Ok(vec![]),
        };
        assert!(tooltip(Some(&s)).starts_with("GitHub: no unread"));
    }
}
