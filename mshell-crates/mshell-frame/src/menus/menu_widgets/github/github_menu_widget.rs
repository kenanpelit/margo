//! GitHub menu — the panel for `MenuType::Github`.
//!
//! Two tabs on a §12 segmented control:
//! - **Inbox**: unread notification threads, newest first. Clicking a row
//!   opens it in the browser and marks it read; the trailing check marks
//!   it read without opening; the header's check marks everything read.
//! - **My PRs**: the user's open pull requests with their CI rollup and
//!   review decision as chips; clicking opens the PR.
//!
//! Reads the shared [`GITHUB_POLL`] (the pill's poll), active only while
//! the menu is revealed.

use crate::bars::bar_widgets::github::GITHUB_POLL;
use crate::github::{self, CiState, GhError, GithubSnapshot, Notification, PullRequest, Review};
use crate::menus::list_fit::fit_scroller;
use crate::menus::menu_widgets::agents::agents_menu_widget::open_url;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};
use mshell_utils::shared_poll::PollSubscription;
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Inbox,
    Prs,
}

pub(crate) struct GithubMenuWidgetModel {
    tab: Tab,
    snapshot: Option<GithubSnapshot>,
    /// Threads marked read here whose refresh hasn't landed yet; hidden
    /// so the row goes away at once.
    marked: HashSet<String>,
    content: gtk::Box,
    tabs: gtk::Box,
    meta: gtk::Label,
    poll: PollSubscription<GithubSnapshot>,
    /// The Inbox / PRs list scroller, sized against the menu height.
    list_scroll: Option<gtk::ScrolledWindow>,
}

/// Built-in height cap of the list when the menu has no configured height.
const LIST_CAP: i32 = 520;

fn fit_list(sw: &gtk::ScrolledWindow) {
    let h = config_manager()
        .config()
        .menus()
        .github_menu()
        .maximum_height()
        .get_untracked();
    fit_scroller(sw, h, LIST_CAP);
}

#[derive(Debug)]
pub(crate) enum GithubMenuWidgetInput {
    SelectTab(u8),
    Refresh,
    Open(String),
    MarkRead(String),
    MarkAllRead,
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum GithubMenuWidgetOutput {}

pub(crate) struct GithubMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum GithubMenuWidgetCommandOutput {
    Snapshot(GithubSnapshot),
    ActionDone(Result<(), GhError>),
}

#[relm4::component(pub(crate))]
impl Component for GithubMenuWidgetModel {
    type CommandOutput = GithubMenuWidgetCommandOutput;
    type Input = GithubMenuWidgetInput;
    type Output = GithubMenuWidgetOutput;
    type Init = GithubMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "github-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("github-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_xalign: 0.0,
                    set_label: "GitHub",
                },
                #[local_ref]
                meta -> gtk::Label {
                    add_css_class: "panel-header-meta",
                    set_valign: gtk::Align::Center,
                },
                #[name = "mark_all_btn"]
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_icon_name: "object-select-symbolic",
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input(GithubMenuWidgetInput::MarkAllRead);
                    },
                },
                #[name = "refresh_btn"]
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_icon_name: "view-refresh-symbolic",
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input(GithubMenuWidgetInput::Refresh);
                    },
                },
            },

            #[local_ref]
            tabs -> gtk::Box {
                add_css_class: "github-tabs",
                set_orientation: gtk::Orientation::Horizontal,
                set_homogeneous: true,
                set_spacing: 4,
            },

            #[name = "list_scroll"]
            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_max_content_height: LIST_CAP,
                set_propagate_natural_height: true,

                #[local_ref]
                content -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::End,
                set_margin_top: 6,
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Open on GitHub",
                    connect_clicked[sender] => move |_| {
                        sender.input(GithubMenuWidgetInput::Open(
                            "https://github.com/notifications".into(),
                        ));
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
        let poll = GITHUB_POLL.subscribe();
        let mut rx = poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let snap = rx.borrow_and_update().clone();
                if let Some(s) = snap {
                    let _ = out.send(GithubMenuWidgetCommandOutput::Snapshot(s));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = rx.changed() => if changed.is_err() { break },
                }
            }
        });

        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let meta = gtk::Label::new(None);
        let mut model = GithubMenuWidgetModel {
            tab: Tab::Inbox,
            snapshot: None,
            marked: HashSet::new(),
            content: content.clone(),
            tabs: tabs.clone(),
            meta: meta.clone(),
            poll,
            list_scroll: None,
        };
        let widgets = view_output!();
        fit_list(&widgets.list_scroll);
        model.list_scroll = Some(widgets.list_scroll.clone());
        mshell_common::a11y::name(&widgets.mark_all_btn, "Mark all as read");
        mshell_common::a11y::name(&widgets.refresh_btn, "Refresh");
        model.render(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            GithubMenuWidgetInput::SelectTab(i) => {
                self.tab = if i == 0 { Tab::Inbox } else { Tab::Prs };
            }
            GithubMenuWidgetInput::Refresh => GITHUB_POLL.refresh_now(),
            GithubMenuWidgetInput::Open(url) => {
                if github::is_github_url(&url) {
                    open_url(&url);
                }
                return;
            }
            GithubMenuWidgetInput::MarkRead(id) => {
                self.marked.insert(id.clone());
                sender.oneshot_command(async move {
                    GithubMenuWidgetCommandOutput::ActionDone(github::mark_read(&id).await)
                });
            }
            GithubMenuWidgetInput::MarkAllRead => {
                if let Some(Ok(list)) = self.snapshot.as_ref().map(|s| &s.notifications) {
                    self.marked.extend(list.iter().map(|n| n.id.clone()));
                }
                sender.oneshot_command(async {
                    GithubMenuWidgetCommandOutput::ActionDone(github::mark_all_read().await)
                });
            }
            GithubMenuWidgetInput::ParentRevealChanged(visible) => {
                if visible && let Some(sw) = &self.list_scroll {
                    // Pick up a Settings height change without a restart.
                    fit_list(sw);
                }
                self.poll.set_active(visible);
                return;
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
            GithubMenuWidgetCommandOutput::Snapshot(s) => {
                // Forget local marks the server now agrees with.
                if let Ok(list) = &s.notifications {
                    self.marked
                        .retain(|id| list.iter().any(|n| &n.id == id && n.unread));
                }
                self.snapshot = Some(s);
                self.render(&sender);
            }
            GithubMenuWidgetCommandOutput::ActionDone(res) => {
                if let Err(e) = res {
                    tracing::warn!(error = %e.message(), "github: mark read failed");
                    self.marked.clear();
                    self.render(&sender);
                }
                GITHUB_POLL.refresh_now();
            }
        }
    }
}

impl GithubMenuWidgetModel {
    fn unread(&self) -> Vec<&Notification> {
        match self.snapshot.as_ref().map(|s| &s.notifications) {
            Some(Ok(list)) => list
                .iter()
                .filter(|n| n.unread && !self.marked.contains(&n.id))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn render(&self, sender: &ComponentSender<Self>) {
        let unread = self.unread();
        let pr_count = match self.snapshot.as_ref().map(|s| &s.prs) {
            Some(Ok(p)) => p.len(),
            _ => 0,
        };
        self.meta.set_label(&match unread.len() {
            0 => String::new(),
            n => format!("{n} unread"),
        });
        build_tabs(
            &self.tabs,
            &[
                format!("Inbox · {}", unread.len()),
                format!("My PRs · {pr_count}"),
            ],
            if self.tab == Tab::Inbox { 0 } else { 1 },
            sender,
        );
        clear(&self.content);
        let Some(s) = &self.snapshot else {
            self.content.append(&hint("Loading…"));
            return;
        };
        match self.tab {
            Tab::Inbox => match &s.notifications {
                Err(e) => self.content.append(&error(e)),
                Ok(_) if unread.is_empty() => self
                    .content
                    .append(&hint("All caught up — no unread notifications")),
                Ok(_) => {
                    let list = boxed_list();
                    for n in unread {
                        list.append(&notification_row(n, sender));
                    }
                    self.content.append(&list);
                }
            },
            Tab::Prs => match &s.prs {
                Err(e) => self.content.append(&error(e)),
                Ok(prs) if prs.is_empty() => self.content.append(&hint("No open pull requests")),
                Ok(prs) => {
                    let list = boxed_list();
                    for p in prs {
                        list.append(&pr_row(p, sender));
                    }
                    self.content.append(&list);
                }
            },
        }
    }
}

/// The §12 segmented control: one capsule, the active segment filled.
fn build_tabs(
    container: &gtk::Box,
    labels: &[String],
    active: usize,
    sender: &ComponentSender<GithubMenuWidgetModel>,
) {
    clear(container);
    for (i, text) in labels.iter().enumerate() {
        let b = gtk::Button::with_label(text);
        b.set_css_classes(if i == active {
            &["github-tab", "active"]
        } else {
            &["github-tab"]
        });
        let s = sender.clone();
        b.connect_clicked(move |_| s.input(GithubMenuWidgetInput::SelectTab(i as u8)));
        container.append(&b);
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn hint(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(&["label-small", "dim-label"]);
    l.set_halign(gtk::Align::Start);
    l
}

fn error(e: &GhError) -> gtk::Label {
    let l = gtk::Label::new(Some(&e.message()));
    l.set_css_classes(&["label-small-error"]);
    l.set_halign(gtk::Align::Start);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l
}

fn boxed_list() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_css_classes(&["ok-boxed-list", "compact", "github-list"]);
    b
}

/// Title + dim subtitle, left-aligned, title wrapping to two lines.
fn text_block(title: &str, subtitle: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.set_css_classes(&["label-medium"]);
    t.set_halign(gtk::Align::Start);
    t.set_xalign(0.0);
    t.set_wrap(true);
    t.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    t.set_lines(2);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&t);
    let s = gtk::Label::new(Some(subtitle));
    s.set_css_classes(&["label-small", "dim-label"]);
    s.set_halign(gtk::Align::Start);
    s.set_xalign(0.0);
    s.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&s);
    b
}

fn notification_row(n: &Notification, sender: &ComponentSender<GithubMenuWidgetModel>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.add_css_class("github-row");

    let open = gtk::Button::new();
    open.set_css_classes(&["github-row-main"]);
    open.set_hexpand(true);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = gtk::Image::from_icon_name(n.icon());
    icon.add_css_class("github-row-icon");
    icon.set_valign(gtk::Align::Start);
    inner.append(&icon);
    let now = chrono::Utc::now().timestamp_millis();
    inner.append(&text_block(
        &n.title,
        &format!(
            "{} · {} · {}",
            n.repo,
            github::reason_label(&n.reason),
            github::ago(now, n.updated_ms)
        ),
    ));
    open.set_child(Some(&inner));
    {
        let (s, url, id) = (sender.clone(), n.url.clone(), n.id.clone());
        open.connect_clicked(move |_| {
            s.input(GithubMenuWidgetInput::Open(url.clone()));
            s.input(GithubMenuWidgetInput::MarkRead(id.clone()));
        });
    }
    row.append(&open);

    let mark = gtk::Button::from_icon_name("object-select-symbolic");
    mark.set_css_classes(&["ok-button-flat", "github-row-action"]);
    mark.set_valign(gtk::Align::Center);
    mshell_common::a11y::name(&mark, "Mark as read");
    {
        let (s, id) = (sender.clone(), n.id.clone());
        mark.connect_clicked(move |_| s.input(GithubMenuWidgetInput::MarkRead(id.clone())));
    }
    row.append(&mark);
    row
}

fn chip(text: &str, state: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(&["github-chip", state]);
    l.set_valign(gtk::Align::Center);
    l
}

fn pr_row(p: &PullRequest, sender: &ComponentSender<GithubMenuWidgetModel>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.add_css_class("github-row");
    let open = gtk::Button::new();
    open.set_css_classes(&["github-row-main"]);
    open.set_hexpand(true);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = gtk::Image::from_icon_name("vcs-pull-request-symbolic");
    icon.add_css_class("github-row-icon");
    icon.set_valign(gtk::Align::Start);
    inner.append(&icon);

    let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
    col.set_hexpand(true);
    let now = chrono::Utc::now().timestamp_millis();
    col.append(&text_block(
        &p.title,
        &format!(
            "{} #{} · {}",
            p.repo,
            p.number,
            github::ago(now, p.updated_ms)
        ),
    ));
    let chips = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    if p.draft {
        chips.append(&chip("Draft", "neutral"));
    }
    let ci_state = match p.ci {
        CiState::Failure => "danger",
        CiState::Pending => "warn",
        CiState::Success => "positive",
        CiState::None => "",
    };
    if !ci_state.is_empty() {
        chips.append(&chip(p.ci.label(), ci_state));
    }
    let review_state = match p.review {
        Review::Approved => "positive",
        Review::ChangesRequested => "danger",
        Review::Required => "neutral",
        Review::None => "",
    };
    if !review_state.is_empty() {
        chips.append(&chip(p.review.label(), review_state));
    }
    if chips.first_child().is_some() {
        col.append(&chips);
    }
    inner.append(&col);
    open.set_child(Some(&inner));
    {
        let (s, url) = (sender.clone(), p.url.clone());
        open.connect_clicked(move |_| s.input(GithubMenuWidgetInput::Open(url.clone())));
    }
    row.append(&open);
    row
}
