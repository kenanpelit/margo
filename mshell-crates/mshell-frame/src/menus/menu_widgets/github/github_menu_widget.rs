//! GitHub menu — the panel for `MenuType::Github`.
//!
//! Three tabs on a §12 segmented control:
//! - **Inbox**: unread notification threads, newest first. Clicking a row
//!   opens it in the browser and marks it read; the trailing check marks
//!   it read without opening; the header's check marks everything read.
//! - **PRs**: the open pull requests the search rules match (Settings →
//!   Widgets → GitHub; default `author:@me`), grouped by repository with
//!   their CI rollup and review decision as chips; clicking opens the PR.
//!   A failed or truncated rule is noted under the list.
//! - **Activity**: the contribution heat-map for the last year with
//!   today / current streak / best streak / year total
//!   ([`super::activity`]); the calendar ([`CONTRIB_POLL`]) is fetched
//!   only while this tab is shown.
//!
//! Reads the shared [`GITHUB_POLL`] (the pill's poll), active only while
//! the menu is revealed. The tabs and rows are built once and updated in
//! place on each poll (rows keyed by thread id / PR URL, see
//! [`KeyedList`]), so keyboard focus and the scroll position survive it.

use super::activity::ActivityView;
use crate::bars::bar_widgets::github::{CONTRIB_POLL, ContribResult, GITHUB_POLL};
use crate::common_widgets::segmented::{Segmented, SegmentedStyle};
use crate::github::{self, CiState, GhError, GithubSnapshot, Notification, PullRequest, Review};
use crate::menus::menu_widgets::agents::agents_menu_widget::open_url;
use crate::menus::menu_widgets::agents::keyed_rows::{KeyedList, KeyedRow};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use mshell_config::schema::menu_widgets::MenuWidget;
use mshell_utils::shared_poll::PollSubscription;
use relm4::gtk::prelude::{BoxExt, ButtonExt, Cast, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Inbox,
    Prs,
    Activity,
}

pub(crate) struct GithubMenuWidgetModel {
    tab: Tab,
    snapshot: Option<GithubSnapshot>,
    /// Threads marked read here whose refresh hasn't landed yet; hidden
    /// so the row goes away at once.
    marked: HashSet<String>,
    /// Inbox | PRs | Activity, built once.
    tabs: Segmented,
    meta: gtk::Label,
    /// "Loading…" / "All caught up" / "No open pull requests".
    hint: gtk::Label,
    error: gtk::Label,
    inbox_box: gtk::Box,
    inbox: KeyedList<NotificationRow>,
    prs_box: gtk::Box,
    prs: KeyedList<PrListRow>,
    /// Partial-failure / truncation notes for the PR rules.
    pr_notes: gtk::Label,
    poll: PollSubscription<GithubSnapshot>,
    contrib: Option<ContribResult>,
    contrib_poll: PollSubscription<ContribResult>,
    activity: ActivityView,
    revealed: bool,
    footer: gtk::Button,
    _effects: mshell_common::scoped_effects::EffectScope,
}

#[derive(Debug)]
pub(crate) enum GithubMenuWidgetInput {
    SelectTab(u8),
    Refresh,
    Open(String),
    MarkRead(String),
    MarkAllRead,
    ParentRevealChanged(bool),
    /// The footer: the inbox, or the profile on the Activity tab.
    OpenFooter,
}

#[derive(Debug)]
pub(crate) enum GithubMenuWidgetOutput {}

pub(crate) struct GithubMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum GithubMenuWidgetCommandOutput {
    Snapshot(GithubSnapshot),
    Contrib(ContribResult),
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

            #[local_ref]
            header_root -> gtk::Box {},

            #[local_ref]
            tabs_root -> gtk::Box {},

            #[name = "fill_list"]
            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_max_content_height: 520,
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
                #[local_ref]
                footer -> gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Open on GitHub",
                    connect_clicked[sender] => move |_| {
                        sender.input(GithubMenuWidgetInput::OpenFooter);
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
        let mut effects = mshell_common::scoped_effects::EffectScope::new();
        crate::bars::bar_widgets::github::watch_pr_rules(&mut effects);
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

        let contrib_poll = CONTRIB_POLL.subscribe();
        contrib_poll.set_active(false);
        let mut crx = contrib_poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let c = crx.borrow_and_update().clone();
                if let Some(c) = c {
                    let _ = out.send(GithubMenuWidgetCommandOutput::Contrib(c));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = crx.changed() => if changed.is_err() { break },
                }
            }
        });

        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let s = sender.clone();
        let tabs = Segmented::new(
            &["Inbox", "PRs", "Activity"],
            0,
            SegmentedStyle::Capsule,
            move |i| {
                s.input(GithubMenuWidgetInput::SelectTab(i as u8));
            },
        );
        let tabs_root = tabs.root().clone();
        let (s_mark, s_refresh) = (sender.clone(), sender.clone());
        let header = PanelHeader::new("github-symbolic", MenuWidget::Github.display_name())
            .with_meta()
            .action("object-select-symbolic", "Mark all as read", move || {
                s_mark.input(GithubMenuWidgetInput::MarkAllRead);
            })
            .action("view-refresh-symbolic", "Refresh", move || {
                s_refresh.input(GithubMenuWidgetInput::Refresh);
            });
        let header_root = header.root().clone();
        let meta = header.meta().cloned().unwrap_or_default();
        let status = hint("");
        let error = gtk::Label::new(None);
        error.set_css_classes(&["label-small-error"]);
        error.set_halign(gtk::Align::Start);
        error.set_xalign(0.0);
        error.set_wrap(true);
        let inbox_box = boxed_list();
        let prs_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        prs_box.add_css_class("github-pr-groups");
        let pr_notes = hint("");
        pr_notes.set_wrap(true);
        pr_notes.set_xalign(0.0);
        let activity = ActivityView::new();
        for w in [
            status.upcast_ref::<gtk::Widget>(),
            error.upcast_ref(),
            inbox_box.upcast_ref(),
            prs_box.upcast_ref(),
            pr_notes.upcast_ref(),
            activity.root.upcast_ref(),
        ] {
            content.append(w);
        }
        let footer = gtk::Button::new();
        let mut model = GithubMenuWidgetModel {
            tab: Tab::Inbox,
            snapshot: None,
            marked: HashSet::new(),
            tabs,
            meta: meta.clone(),
            hint: status,
            error,
            inbox: KeyedList::new(inbox_box.clone()),
            inbox_box,
            prs: KeyedList::new(prs_box.clone()),
            prs_box,
            pr_notes,
            poll,
            contrib: None,
            contrib_poll,
            activity,
            revealed: false,
            footer: footer.clone(),
            _effects: effects,
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 520);
        model.render(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            GithubMenuWidgetInput::SelectTab(i) => {
                self.tab = match i {
                    0 => Tab::Inbox,
                    1 => Tab::Prs,
                    _ => Tab::Activity,
                };
                self.sync_contrib_poll();
            }
            GithubMenuWidgetInput::Refresh => {
                GITHUB_POLL.refresh_now();
                if self.tab == Tab::Activity {
                    CONTRIB_POLL.refresh_now();
                }
            }
            GithubMenuWidgetInput::OpenFooter => {
                let url = match (&self.tab, &self.contrib) {
                    (Tab::Activity, Some(Ok(cal))) if !cal.login.is_empty() => {
                        format!("https://github.com/{}", cal.login)
                    }
                    _ => "https://github.com/notifications".to_string(),
                };
                if github::is_github_url(&url) {
                    open_url(&url);
                }
                return;
            }
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
                self.poll.set_active(visible);
                self.revealed = visible;
                self.sync_contrib_poll();
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
            GithubMenuWidgetCommandOutput::Contrib(c) => {
                self.contrib = Some(c);
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
    fn sync_contrib_poll(&self) {
        self.contrib_poll
            .set_active(self.revealed && self.tab == Tab::Activity);
    }

    fn unread(&self) -> Vec<Notification> {
        match self.snapshot.as_ref().map(|s| &s.notifications) {
            Some(Ok(list)) => list
                .iter()
                .filter(|n| n.unread && !self.marked.contains(&n.id))
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    }

    fn render(&mut self, sender: &ComponentSender<Self>) {
        let unread = self.unread();
        let pr_count = match self.snapshot.as_ref().map(|s| &s.prs) {
            Some(Ok(p)) => p.len(),
            _ => 0,
        };
        self.meta.set_label(&match unread.len() {
            0 => String::new(),
            n => format!("{n} unread"),
        });
        // The §12 segmented control: one capsule, the active segment filled.
        self.tabs.set_label(0, &format!("Inbox · {}", unread.len()));
        self.tabs.set_label(1, &format!("PRs · {pr_count}"));
        self.tabs.set_active(match self.tab {
            Tab::Inbox => 0,
            Tab::Prs => 1,
            Tab::Activity => 2,
        });
        self.footer.set_label(if self.tab == Tab::Activity {
            "Open profile"
        } else {
            "Open on GitHub"
        });

        let now = chrono::Utc::now().timestamp_millis();
        // What the page shows: a hint, an error, or one of the lists.
        let mut hint = None;
        let mut error = None;
        let (mut show_inbox, mut show_prs, mut show_activity) = (false, false, false);
        match &self.snapshot {
            // The calendar has its own poll: don't wait on the inbox.
            _ if self.tab == Tab::Activity => match &self.contrib {
                None => hint = Some("Loading the contribution calendar…"),
                Some(Err(e)) => error = Some(e.message()),
                Some(Ok(cal)) => {
                    let today = chrono::Local::now().date_naive();
                    self.activity.set_calendar(cal, cal.stats(today));
                    show_activity = true;
                }
            },
            None => hint = Some("Loading…"),
            Some(s) => match self.tab {
                Tab::Inbox => match &s.notifications {
                    Err(e) => error = Some(e.message()),
                    Ok(_) if unread.is_empty() => {
                        hint = Some("All caught up — no unread notifications");
                    }
                    Ok(_) => {
                        let items: Vec<(Notification, i64)> =
                            unread.into_iter().map(|n| (n, now)).collect();
                        self.inbox
                            .sync(&items, |item| NotificationRow::new(item, sender));
                        show_inbox = true;
                    }
                },
                Tab::Activity => {}
                Tab::Prs => match &s.prs {
                    Err(e) => error = Some(e.message()),
                    Ok(prs) if prs.is_empty() => hint = Some("No open pull requests"),
                    Ok(prs) => {
                        let mut items: Vec<(PrItem, i64)> = Vec::new();
                        for (repo, group) in github::group_by_repo(prs) {
                            items.push((
                                PrItem::Repo {
                                    repo,
                                    count: group.len(),
                                },
                                now,
                            ));
                            items.extend(group.into_iter().map(|p| (PrItem::Pr(p.clone()), now)));
                        }
                        self.prs.sync(&items, |item| PrListRow::new(item, sender));
                        show_prs = true;
                    }
                },
            },
        }
        self.hint.set_label(hint.unwrap_or(""));
        self.hint.set_visible(hint.is_some());
        self.error.set_label(error.as_deref().unwrap_or(""));
        self.error.set_visible(error.is_some());
        self.inbox_box.set_visible(show_inbox);
        self.prs_box.set_visible(show_prs);
        let notes = match (&self.tab, &self.snapshot) {
            (Tab::Prs, Some(s)) => s.pr_notes.join("\n"),
            _ => String::new(),
        };
        self.pr_notes.set_label(&notes);
        self.pr_notes.set_visible(!notes.is_empty());
        self.activity.root.set_visible(show_activity);
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

fn boxed_list() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_css_classes(&["ok-boxed-list", "compact", "github-list"]);
    b
}

/// Title + dim subtitle, left-aligned, title wrapping to two lines.
/// Returns the box and both labels.
fn text_block() -> (gtk::Box, gtk::Label, gtk::Label) {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.set_hexpand(true);
    let t = gtk::Label::new(None);
    t.set_css_classes(&["label-medium"]);
    t.set_halign(gtk::Align::Start);
    t.set_xalign(0.0);
    t.set_wrap(true);
    t.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    t.set_lines(2);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&t);
    let s = gtk::Label::new(None);
    s.set_css_classes(&["label-small", "dim-label"]);
    s.set_halign(gtk::Align::Start);
    s.set_xalign(0.0);
    s.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&s);
    (b, t, s)
}

/// An unread thread: open it (and mark it read), or just mark it read.
struct NotificationRow {
    row: gtk::Box,
    icon: gtk::Image,
    title: gtk::Label,
    subtitle: gtk::Label,
    url: Rc<RefCell<String>>,
}

impl NotificationRow {
    fn new(item: &(Notification, i64), sender: &ComponentSender<GithubMenuWidgetModel>) -> Self {
        let n = &item.0;
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        row.add_css_class("github-row");

        let open = gtk::Button::new();
        open.set_css_classes(&["github-row-main"]);
        open.set_hexpand(true);
        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let icon = gtk::Image::new();
        icon.add_css_class("github-row-icon");
        icon.set_valign(gtk::Align::Start);
        inner.append(&icon);
        let (block, title, subtitle) = text_block();
        inner.append(&block);
        open.set_child(Some(&inner));
        let url = Rc::new(RefCell::new(String::new()));
        {
            let (s, url, id) = (sender.clone(), url.clone(), n.id.clone());
            open.connect_clicked(move |_| {
                s.input(GithubMenuWidgetInput::Open(url.borrow().clone()));
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
        let r = NotificationRow {
            row,
            icon,
            title,
            subtitle,
            url,
        };
        r.update(item);
        r
    }
}

impl KeyedRow for NotificationRow {
    type Item = (Notification, i64);
    type Key = String;

    fn key(item: &Self::Item) -> String {
        item.0.id.clone()
    }

    fn widget(&self) -> gtk::Widget {
        self.row.clone().upcast()
    }

    fn update(&self, (n, now): &Self::Item) {
        self.icon.set_icon_name(Some(n.icon()));
        self.title.set_label(&n.title);
        self.subtitle.set_label(&format!(
            "{} · {} · {}",
            n.repo,
            github::reason_label(&n.reason),
            github::ago(*now, n.updated_ms)
        ));
        self.url.replace(n.url.clone());
    }
}

fn chip(text: &str, state: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(&["github-chip", state]);
    l.set_valign(gtk::Align::Center);
    l
}

/// A PR-tab entry: a repository heading or one of its PRs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PrItem {
    Repo { repo: String, count: usize },
    Pr(PullRequest),
}

/// The keyed row for a [`PrItem`] (keys `repo:<name>` / the PR URL never
/// collide, so a key always maps to the same variant).
enum PrListRow {
    Repo(gtk::Label),
    Pr(PrRow),
}

impl PrListRow {
    fn new(item: &(PrItem, i64), sender: &ComponentSender<GithubMenuWidgetModel>) -> Self {
        match &item.0 {
            PrItem::Repo { .. } => {
                let l = gtk::Label::new(None);
                l.set_css_classes(&["github-group-label"]);
                l.set_halign(gtk::Align::Start);
                l.set_xalign(0.0);
                l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                let r = PrListRow::Repo(l);
                r.update(item);
                r
            }
            PrItem::Pr(p) => PrListRow::Pr(PrRow::new(&(p.clone(), item.1), sender)),
        }
    }
}

impl KeyedRow for PrListRow {
    type Item = (PrItem, i64);
    type Key = String;

    fn key(item: &Self::Item) -> String {
        match &item.0 {
            PrItem::Repo { repo, .. } => format!("repo:{repo}"),
            PrItem::Pr(p) => p.url.clone(),
        }
    }

    fn widget(&self) -> gtk::Widget {
        match self {
            PrListRow::Repo(l) => l.clone().upcast(),
            PrListRow::Pr(r) => r.widget(),
        }
    }

    fn update(&self, (item, now): &Self::Item) {
        match (self, item) {
            (PrListRow::Repo(l), PrItem::Repo { repo, count }) => {
                l.set_label(&format!("{repo} · {count}"));
            }
            (PrListRow::Pr(r), PrItem::Pr(p)) => r.update(&(p.clone(), *now)),
            _ => {}
        }
    }
}

/// One open PR, with CI and review chips; click opens it.
struct PrRow {
    row: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    chips: gtk::Box,
    /// What `chips` shows, so an unchanged set isn't rebuilt.
    chip_state: RefCell<Vec<(&'static str, &'static str)>>,
}

impl PrRow {
    fn new(item: &(PullRequest, i64), sender: &ComponentSender<GithubMenuWidgetModel>) -> Self {
        let p = &item.0;
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        row.set_css_classes(&["github-row", "github-pr-card"]);
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
        let (block, title, subtitle) = text_block();
        col.append(&block);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        col.append(&chips);
        inner.append(&col);
        open.set_child(Some(&inner));
        {
            // The key is the URL, so it never changes for this row.
            let (s, url) = (sender.clone(), p.url.clone());
            open.connect_clicked(move |_| s.input(GithubMenuWidgetInput::Open(url.clone())));
        }
        row.append(&open);
        let r = PrRow {
            row,
            title,
            subtitle,
            chips,
            chip_state: RefCell::new(Vec::new()),
        };
        // An empty chip set differs from nothing only by visibility.
        r.chips.set_visible(false);
        r.update(item);
        r
    }
}

impl KeyedRow for PrRow {
    type Item = (PullRequest, i64);
    type Key = String;

    fn key(item: &Self::Item) -> String {
        item.0.url.clone()
    }

    fn widget(&self) -> gtk::Widget {
        self.row.clone().upcast()
    }

    fn update(&self, (p, now): &Self::Item) {
        self.title.set_label(&p.title);
        // The repository is the group heading above.
        self.subtitle.set_label(&format!(
            "#{} · {}",
            p.number,
            github::ago(*now, p.updated_ms)
        ));
        let state = pr_chips(p);
        if *self.chip_state.borrow() != state {
            clear(&self.chips);
            for (text, class) in &state {
                self.chips.append(&chip(text, class));
            }
            self.chips.set_visible(!state.is_empty());
            self.chip_state.replace(state);
        }
    }
}

/// The chips a PR shows: `(label, severity class)`.
fn pr_chips(p: &PullRequest) -> Vec<(&'static str, &'static str)> {
    let mut v = Vec::new();
    if p.draft {
        v.push(("Draft", "neutral"));
    }
    let ci_state = match p.ci {
        CiState::Failure => "danger",
        CiState::Pending => "warn",
        CiState::Success => "positive",
        CiState::None => "",
    };
    if !ci_state.is_empty() {
        v.push((p.ci.label(), ci_state));
    }
    let review_state = match p.review {
        Review::Approved => "positive",
        Review::ChangesRequested => "danger",
        Review::Required => "neutral",
        Review::None => "",
    };
    if !review_state.is_empty() {
        v.push((p.review.label(), review_state));
    }
    v
}
