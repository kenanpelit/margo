//! System Updates menu widget — the panel content for
//! `MenuType::SystemUpdate`. Ports the noctalia arch-updater panel:
//! a table of pending updates grouped by source (Repo / AUR /
//! Flatpak), each row showing `name  old → new`, plus Refresh and
//! Update buttons. Probing lives in [`crate::system_update`].

use crate::system_update::history::{self, HistorySource, Transaction};
use crate::system_update::news::{self, NewsItem};
use crate::system_update::pacnew::{self, ConfigFile, DiffLine, Kind, Op, Prefer, Review};
use crate::system_update::run::{self, RunStatus};
use crate::system_update::{self, ProbeConfig, Source, UpdateReport};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, SystemUpdateBarWidgetStoreFields,
    SystemUpdateMode,
};
use mshell_process::SpawnReaped;
use reactive_graph::traits::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::{AccessibleExtManual, BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct SystemUpdateMenuWidgetModel {
    report: Option<UpdateReport>,
    refreshing: bool,
    /// Which sources the panel probes — mirrors the live
    /// `bars.widgets.system_update` toggles. Edited in-panel via
    /// the source switches; the change persists to config and
    /// triggers a re-probe.
    sources: ProbeConfig,
    /// Arch news posts published since the last full upgrade (empty
    /// when the feed is off, unreachable, or everything is read).
    news: Vec<NewsItem>,
    /// Update was pressed while news was unread: the news card is
    /// raised and the button now reads "Upgrade anyway".
    news_confirm: bool,
    /// Background upgrade state (shared by every panel instance).
    run: RunStatus,
    /// Why the last background start failed (no pkexec, …).
    run_error: Option<String>,
    /// Update history (newest first) + where it was read from.
    history: Option<(HistorySource, Vec<Transaction>)>,
    /// Cache file names, to tell which old versions can be restored.
    cache_files: Vec<String>,
    /// Row armed for rollback (first click arms, second confirms).
    armed_rollback: Option<String>,
    /// `.pacnew` / `.pacsave` / `.pacorig` files under /etc.
    configs: Vec<ConfigFile>,
    /// The file open in the review view.
    review: Option<(ConfigFile, Review)>,
    /// A privileged config action is in flight.
    config_busy: bool,
    /// Last config action error.
    config_error: Option<String>,
}

/// What the review view's buttons do.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ConfigAction {
    /// Write the 3-way merge (conflicts resolved to one side).
    Merge(Prefer),
    /// Replace the live file with the .pacnew / restore the .pacsave.
    TakeFile,
    /// Delete the review file, keep the live config.
    Delete,
}

#[derive(Debug)]
pub(crate) enum SystemUpdateMenuWidgetInput {
    /// Re-probe every enabled source.
    Refresh,
    /// Open a terminal and run the full upgrade.
    Update,
    /// Enable / disable a source (repo / AUR / Flatpak). Persists
    /// to config, then re-probes.
    ToggleSource(Source, bool),
    /// The panel was revealed (`true`) / hidden (`false`). On reveal
    /// we re-probe — that's the *only* automatic probe, so the AUR
    /// helper (and its sudo) runs when the user opens the panel, not
    /// on every shell start.
    ParentRevealChanged(bool),
    /// Mark every fetched news post read (hides the card).
    MarkNewsRead,
    /// Open a news post (or the news page) in the browser.
    OpenUrl(String),
    /// Hide a finished background run's card.
    DismissRun,
    /// (Re)load the History page.
    LoadHistory,
    /// Rollback button on a history row: first press arms it, the
    /// second (on the same package) runs `pkexec pacman -U <file>`.
    Rollback {
        name: String,
        file: String,
    },
    /// Re-scan /etc for review files.
    ScanConfigs,
    /// Open the review (diff) view for one file.
    OpenReview(usize),
    CloseReview,
    ConfigAction(ConfigAction),
}

#[derive(Debug)]
pub(crate) enum SystemUpdateMenuWidgetOutput {
    CloseMenu,
}

pub(crate) struct SystemUpdateMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum SystemUpdateMenuWidgetCommandOutput {
    Loaded(UpdateReport),
    /// Unread Arch news (empty on a fetch error — the heads-up is
    /// advisory, a failed fetch must not block the upgrade).
    News(Vec<NewsItem>),
    /// Background-run status changed.
    Run(RunStatus),
    /// A background start failed.
    LaunchFailed(String),
    History(HistorySource, Vec<Transaction>, Vec<String>),
    Configs(Vec<ConfigFile>),
    Reviewed(ConfigFile, Review),
    ConfigDone(Result<(), String>),
}

#[relm4::component(pub(crate))]
impl Component for SystemUpdateMenuWidgetModel {
    type CommandOutput = SystemUpdateMenuWidgetCommandOutput;
    type Input = SystemUpdateMenuWidgetInput;
    type Output = SystemUpdateMenuWidgetOutput;
    type Init = SystemUpdateMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "system-update-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("software-update-available-symbolic"),
                },

                gtk::Label {
                    add_css_class: "panel-title",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    #[watch]
                    set_label: &header_label(model.report.as_ref(), model.refreshing),
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Refresh"),
                    #[watch]
                    set_sensitive: !model.refreshing,
                    connect_clicked[sender] => move |_| {
                        sender.input(SystemUpdateMenuWidgetInput::Refresh);
                    },
                    gtk::Image { set_icon_name: Some("view-refresh-symbolic") },
                },
            },

            // Per-source summary chips.
            gtk::Label {
                add_css_class: "label-small",
                set_halign: gtk::Align::Start,
                set_xalign: 0.0,
                #[watch]
                set_visible: model.report.as_ref().is_some_and(|r| !r.is_empty()),
                #[watch]
                set_label: &summary_line(model.report.as_ref()),
            },

            // Arch news heads-up — posts since the last full upgrade.
            // Raised to the warn tier when Update is pressed with
            // unread posts (DESIGN.md §2), so they are read first.
            gtk::Box {
                #[watch]
                set_css_classes: &news_card_classes(model.news_confirm),
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
                #[watch]
                set_visible: !model.news.is_empty(),

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    gtk::Image {
                        add_css_class: "system-update-news-icon",
                        set_icon_name: Some("dialog-information-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_hexpand: true,
                        set_xalign: 0.0,
                        set_wrap: true,
                        #[watch]
                        set_label: &news_title(model.news.len(), model.news_confirm),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-flat",
                        set_valign: gtk::Align::Center,
                        set_label: "Mark read",
                        connect_clicked[sender] => move |_| {
                            sender.input(SystemUpdateMenuWidgetInput::MarkNewsRead);
                        },
                    },
                },

                #[name = "news_list"]
                gtk::Box {
                    add_css_class: "ok-boxed-list",
                    add_css_class: "compact",
                    set_orientation: gtk::Orientation::Vertical,
                },
            },

            // Source toggles — enable / disable each probe source
            // right in the panel (DESIGN.md: surfaces over borders,
            // pill chips). Toggling persists to config + re-probes.
            gtk::Box {
                add_css_class: "system-update-sources",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 14,
                set_halign: gtk::Align::Start,

                gtk::Box {
                    add_css_class: "system-update-source",
                    set_spacing: 6,
                    gtk::Label { set_label: "Repo" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.sources.repo,
                        connect_state_set[sender] => move |_, active| {
                            sender.input(SystemUpdateMenuWidgetInput::ToggleSource(Source::Repo, active));
                            glib::Propagation::Proceed
                        },
                    },
                },
                gtk::Box {
                    add_css_class: "system-update-source",
                    set_spacing: 6,
                    gtk::Label { set_label: "AUR" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.sources.aur,
                        connect_state_set[sender] => move |_, active| {
                            sender.input(SystemUpdateMenuWidgetInput::ToggleSource(Source::Aur, active));
                            glib::Propagation::Proceed
                        },
                    },
                },
                gtk::Box {
                    add_css_class: "system-update-source",
                    set_spacing: 6,
                    gtk::Label { set_label: "Flatpak" },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        set_active: model.sources.flatpak,
                        connect_state_set[sender] => move |_, active| {
                            sender.input(SystemUpdateMenuWidgetInput::ToggleSource(Source::Flatpak, active));
                            glib::Propagation::Proceed
                        },
                    },
                },
            },

            // ── Tabs (the podman panel's StackSwitcher pattern) ──
            #[name = "stack_switcher"]
            gtk::StackSwitcher {
                set_stack: Some(&stack),
                set_halign: gtk::Align::Start,
            },

            #[name = "stack"]
            gtk::Stack {
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                set_vhomogeneous: false,
                connect_visible_child_name_notify[sender] => move |st| {
                    if st.visible_child_name().as_deref() == Some("history") {
                        sender.input(SystemUpdateMenuWidgetInput::LoadHistory);
                    }
                },

                add_titled[Some("updates"), "Updates"] = &gtk::ScrolledWindow {
                    set_vscrollbar_policy: gtk::PolicyType::Automatic,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,
                    set_min_content_height: 80,

                    #[name = "list"]
                    gtk::Box {
                        add_css_class: "system-update-list",
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 2,
                    },
                },

                add_titled[Some("configs"), "Config files"] = &gtk::ScrolledWindow {
                    set_vscrollbar_policy: gtk::PolicyType::Automatic,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,
                    set_min_content_height: 80,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 8,

                        gtk::Label {
                            add_css_class: "label-small",
                            add_css_class: "danger",
                            set_xalign: 0.0,
                            set_wrap: true,
                            #[watch]
                            set_visible: model.config_error.is_some(),
                            #[watch]
                            set_label: model.config_error.as_deref().unwrap_or(""),
                        },

                        #[name = "config_list"]
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 2,
                            #[watch]
                            set_visible: model.review.is_none(),
                        },

                        // Review view: header, diff, actions.
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 8,
                            #[watch]
                            set_visible: model.review.is_some(),

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 8,
                                gtk::Button {
                                    add_css_class: "ok-button-flat",
                                    set_valign: gtk::Align::Center,
                                    set_icon_name: "go-previous-symbolic",
                                    set_tooltip_text: Some("All config files"),
                                    update_property: &[gtk::accessible::Property::Label("All config files")],
                                    connect_clicked[sender] => move |_| {
                                        sender.input(SystemUpdateMenuWidgetInput::CloseReview);
                                    },
                                },
                                gtk::Box {
                                    set_orientation: gtk::Orientation::Vertical,
                                    set_hexpand: true,
                                    gtk::Label {
                                        add_css_class: "label-medium-bold",
                                        set_xalign: 0.0,
                                        set_ellipsize: gtk::pango::EllipsizeMode::Start,
                                        #[watch]
                                        set_label: &model.review.as_ref().map(|(f, _)| f.path.display().to_string()).unwrap_or_default(),
                                    },
                                    gtk::Label {
                                        add_css_class: "label-small",
                                        add_css_class: "dim-label",
                                        set_xalign: 0.0,
                                        set_wrap: true,
                                        #[watch]
                                        set_label: &model.review.as_ref().map(|(f, r)| review_summary(f, r)).unwrap_or_default(),
                                    },
                                },
                            },

                            #[name = "diff_view"]
                            gtk::Box {
                                add_css_class: "system-update-diff",
                                set_orientation: gtk::Orientation::Vertical,
                            },

                            gtk::FlowBox {
                                set_selection_mode: gtk::SelectionMode::None,
                                set_column_spacing: 6,
                                set_row_spacing: 6,
                                set_max_children_per_line: 3,
                                set_homogeneous: false,
                                #[watch]
                                set_sensitive: !model.config_busy,

                                gtk::Button {
                                    add_css_class: "ok-button-surface",
                                    #[watch]
                                    set_visible: review_can_merge(model.review.as_ref()),
                                    #[watch]
                                    set_label: merge_label(model.review.as_ref(), Prefer::Mine),
                                    set_tooltip_text: Some("Write the 3-way merge; conflicting lines keep your version"),
                                    connect_clicked[sender] => move |_| {
                                        sender.input(SystemUpdateMenuWidgetInput::ConfigAction(ConfigAction::Merge(Prefer::Mine)));
                                    },
                                },
                                gtk::Button {
                                    add_css_class: "ok-button-surface",
                                    #[watch]
                                    set_visible: review_has_conflicts(model.review.as_ref()),
                                    set_label: "Merge, take new lines",
                                    set_tooltip_text: Some("Write the 3-way merge; conflicting lines take the new default"),
                                    connect_clicked[sender] => move |_| {
                                        sender.input(SystemUpdateMenuWidgetInput::ConfigAction(ConfigAction::Merge(Prefer::Theirs)));
                                    },
                                },
                                gtk::Button {
                                    add_css_class: "ok-button-surface",
                                    #[watch]
                                    set_visible: model.review.as_ref().is_some_and(|(_, r)| !r.unreadable),
                                    #[watch]
                                    set_label: take_label(model.review.as_ref()),
                                    connect_clicked[sender] => move |_| {
                                        sender.input(SystemUpdateMenuWidgetInput::ConfigAction(ConfigAction::TakeFile));
                                    },
                                },
                                gtk::Button {
                                    add_css_class: "ok-button-error",
                                    #[watch]
                                    set_label: delete_label(model.review.as_ref()),
                                    connect_clicked[sender] => move |_| {
                                        sender.input(SystemUpdateMenuWidgetInput::ConfigAction(ConfigAction::Delete));
                                    },
                                },
                            },
                        },
                    },
                },

                add_titled[Some("history"), "History"] = &gtk::ScrolledWindow {
                    set_vscrollbar_policy: gtk::PolicyType::Automatic,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,
                    set_min_content_height: 80,

                    #[name = "history_list"]
                    gtk::Box {
                        add_css_class: "system-update-list",
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 2,
                    },
                },
            },

            // Background upgrade: status, progress, live log tail.
            gtk::Box {
                add_css_class: "system-update-run",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
                #[watch]
                set_visible: model.run != RunStatus::Idle || model.run_error.is_some(),

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    gtk::Label {
                        set_hexpand: true,
                        set_xalign: 0.0,
                        set_wrap: true,
                        #[watch]
                        set_css_classes: &run_title_classes(&model.run, model.run_error.is_some()),
                        #[watch]
                        set_label: &run_title(&model.run, model.run_error.as_deref()),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-flat",
                        set_valign: gtk::Align::Center,
                        set_label: "Open log",
                        #[watch]
                        set_visible: model.run != RunStatus::Idle,
                        connect_clicked[sender] => move |_| {
                            sender.input(SystemUpdateMenuWidgetInput::OpenUrl(
                                run::log_path().to_string_lossy().into_owned(),
                            ));
                        },
                    },
                    gtk::Button {
                        add_css_class: "ok-button-flat",
                        set_valign: gtk::Align::Center,
                        set_label: "Dismiss",
                        #[watch]
                        set_visible: !matches!(model.run, RunStatus::Running { .. }),
                        connect_clicked[sender] => move |_| {
                            sender.input(SystemUpdateMenuWidgetInput::DismissRun);
                        },
                    },
                },

                gtk::ProgressBar {
                    #[watch]
                    set_visible: model.run != RunStatus::Idle,
                    #[watch]
                    set_fraction: run_fraction(&model.run),
                },

                gtk::Label {
                    add_css_class: "system-update-log",
                    set_xalign: 0.0,
                    set_selectable: true,
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                    #[watch]
                    set_visible: model.run != RunStatus::Idle,
                    #[watch]
                    set_label: &run_tail(&model.run),
                },
            },

            // Update button — terminal or background, per update_mode.
            gtk::Button {
                add_css_class: "ok-button-primary",
                set_halign: gtk::Align::Fill,
                #[watch]
                set_sensitive: model.report.as_ref().is_some_and(|r| !r.is_empty())
                    && !matches!(model.run, RunStatus::Running { .. }),
                connect_clicked[sender] => move |_| {
                    sender.input(SystemUpdateMenuWidgetInput::Update);
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    gtk::Image { set_icon_name: Some("software-update-available-symbolic") },
                    gtk::Label {
                        #[watch]
                        set_label: update_label(model.news_confirm),
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
        // Show the cached report (written by the bar pill's interval
        // poll) and do NOT probe here. init() runs when the menu
        // content is built — which is at startup, once per monitor — so
        // probing here fired the AUR helper (and its sudo) on every
        // shell restart. The probe now happens only when the panel is
        // actually opened (ParentRevealChanged) or via the toggles.
        let model = SystemUpdateMenuWidgetModel {
            report: system_update::load_cache().map(|(_, report)| report),
            refreshing: false,
            sources: ProbeConfig::from_config(),
            news: Vec::new(),
            news_confirm: false,
            run: run::status(),
            run_error: None,
            history: None,
            cache_files: Vec::new(),
            armed_rollback: None,
            configs: Vec::new(),
            review: None,
            config_busy: false,
            config_error: None,
        };
        let widgets = view_output!();

        // Follow the (single, process-wide) background-run watcher and
        // the re-check it publishes when a run ends.
        run::ensure_watcher();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut status = run::subscribe();
            let mut fresh = system_update::subscribe_reports();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    Ok(()) = status.changed() => {
                        let s = status.borrow_and_update().clone();
                        let _ = out.send(SystemUpdateMenuWidgetCommandOutput::Run(s));
                    }
                    Ok(()) = fresh.changed() => {
                        let r = fresh.borrow_and_update().clone();
                        if let Some(r) = r {
                            let _ = out.send(SystemUpdateMenuWidgetCommandOutput::Loaded(r));
                        }
                    }
                    else => break,
                }
            }
        });
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
            SystemUpdateMenuWidgetInput::Refresh => {
                self.refreshing = true;
                sender.oneshot_command(async {
                    let report = system_update::probe(ProbeConfig::from_config()).await;
                    SystemUpdateMenuWidgetCommandOutput::Loaded(report)
                });
            }
            SystemUpdateMenuWidgetInput::ParentRevealChanged(visible) => {
                // Probe only when the panel actually opens (not on
                // build/startup). Hidden → nothing.
                if visible {
                    sender.input(SystemUpdateMenuWidgetInput::Refresh);
                    sender.input(SystemUpdateMenuWidgetInput::ScanConfigs);
                    if news_enabled() {
                        sender.oneshot_command(async {
                            SystemUpdateMenuWidgetCommandOutput::News(
                                news::unread_now().await.unwrap_or_default(),
                            )
                        });
                    }
                } else {
                    self.news_confirm = false;
                }
            }
            SystemUpdateMenuWidgetInput::Update if !self.news.is_empty() && !self.news_confirm => {
                // Heads-up: the first press with unread news only
                // raises the card; the second press upgrades.
                self.news_confirm = true;
            }
            SystemUpdateMenuWidgetInput::MarkNewsRead => {
                news::mark_read(chrono::Utc::now().timestamp());
                self.news.clear();
                self.news_confirm = false;
                rebuild_news(&widgets.news_list, &self.news, &sender);
            }
            SystemUpdateMenuWidgetInput::OpenUrl(url) => {
                let _ = std::process::Command::new("xdg-open")
                    .arg(url)
                    .spawn_reaped();
            }
            SystemUpdateMenuWidgetInput::LoadHistory => {
                sender.oneshot_command(async {
                    let (source, runs, files) = tokio::task::spawn_blocking(|| {
                        let (source, runs) = history::load_blocking();
                        let files = history::cache_listing()
                            .into_iter()
                            .map(|(f, _)| f)
                            .collect();
                        (source, runs, files)
                    })
                    .await
                    .unwrap_or((
                        HistorySource::LocalDb,
                        Vec::new(),
                        Vec::new(),
                    ));
                    SystemUpdateMenuWidgetCommandOutput::History(source, runs, files)
                });
            }
            SystemUpdateMenuWidgetInput::Rollback { name, file } => {
                if self.armed_rollback.as_deref() != Some(name.as_str()) {
                    self.armed_rollback = Some(name);
                } else {
                    self.armed_rollback = None;
                    self.run_error = None;
                    match history::rollback_step(&file) {
                        Some(step) => {
                            if let Err(e) = run::start(vec![step], 1) {
                                self.run_error = Some(e);
                            }
                        }
                        None => {
                            self.run_error = Some(format!("unexpected cache file name: {file}"))
                        }
                    }
                    self.run = run::status();
                }
                if let Some((source, runs)) = &self.history {
                    rebuild_history(
                        &widgets.history_list,
                        *source,
                        runs,
                        &self.cache_files,
                        self.armed_rollback.as_deref(),
                        &sender,
                    );
                }
            }
            SystemUpdateMenuWidgetInput::ScanConfigs => {
                sender.oneshot_command(async {
                    let found = tokio::task::spawn_blocking(|| {
                        pacnew::scan(std::path::Path::new(pacnew::SEARCH_ROOT))
                    })
                    .await
                    .unwrap_or_default();
                    SystemUpdateMenuWidgetCommandOutput::Configs(found)
                });
            }
            SystemUpdateMenuWidgetInput::OpenReview(i) => {
                if let Some(file) = self.configs.get(i).cloned() {
                    self.config_error = None;
                    sender.oneshot_command(async move {
                        let f = file.clone();
                        let review =
                            tokio::task::spawn_blocking(move || pacnew::review_blocking(&f))
                                .await
                                .unwrap_or_default();
                        SystemUpdateMenuWidgetCommandOutput::Reviewed(file, review)
                    });
                }
            }
            SystemUpdateMenuWidgetInput::CloseReview => self.review = None,
            SystemUpdateMenuWidgetInput::ConfigAction(action) => {
                if let Some((file, review)) = self.review.clone() {
                    let op = match action {
                        ConfigAction::Merge(prefer) => review
                            .merge
                            .as_ref()
                            .map(|segs| pacnew::resolve(segs, prefer))
                            .ok_or_else(|| "no 3-way merge available".to_string())
                            .and_then(|text| pacnew::stage_merged(&text))
                            .map(|src| Op::Install {
                                src,
                                live: file.live.clone(),
                                review: file.path.clone(),
                            }),
                        ConfigAction::TakeFile => Ok(Op::Install {
                            src: file.path.clone(),
                            live: file.live.clone(),
                            review: file.path.clone(),
                        }),
                        ConfigAction::Delete => Ok(Op::Remove {
                            review: file.path.clone(),
                        }),
                    };
                    match op {
                        Ok(op) => {
                            self.config_busy = true;
                            sender.oneshot_command(async move {
                                SystemUpdateMenuWidgetCommandOutput::ConfigDone(
                                    pacnew::apply(op).await,
                                )
                            });
                        }
                        Err(e) => self.config_error = Some(e),
                    }
                }
            }
            SystemUpdateMenuWidgetInput::DismissRun => {
                self.run_error = None;
                run::dismiss();
            }
            SystemUpdateMenuWidgetInput::Update => {
                self.news_confirm = false;
                self.run_error = None;
                let expected = self.report.as_ref().map_or(0, |r| r.total());
                let background = update_mode() == SystemUpdateMode::Background;
                sender.oneshot_command(async move {
                    match system_update::launch_upgrade(ProbeConfig::from_config(), expected).await
                    {
                        Ok(()) => SystemUpdateMenuWidgetCommandOutput::Run(run::status()),
                        Err(e) => SystemUpdateMenuWidgetCommandOutput::LaunchFailed(e),
                    }
                });
                // Terminal mode hands off to the terminal window; the
                // background mode keeps the panel open on the live log.
                if !background {
                    let _ = sender.output(SystemUpdateMenuWidgetOutput::CloseMenu);
                }
            }
            SystemUpdateMenuWidgetInput::ToggleSource(source, active) => {
                let current = match source {
                    Source::Repo => self.sources.repo,
                    Source::Aur => self.sources.aur,
                    Source::Flatpak => self.sources.flatpak,
                };
                // `set_active` at build time fires this signal once
                // with the value we already hold — ignore that no-op
                // so the panel doesn't persist + re-probe on open.
                if active != current {
                    match source {
                        Source::Repo => self.sources.repo = active,
                        Source::Aur => self.sources.aur = active,
                        Source::Flatpak => self.sources.flatpak = active,
                    }
                    config_manager().update_config(move |config| match source {
                        Source::Repo => config.bars.widgets.system_update.check_repo = active,
                        Source::Aur => config.bars.widgets.system_update.check_aur = active,
                        Source::Flatpak => config.bars.widgets.system_update.check_flatpak = active,
                    });
                    sender.input(SystemUpdateMenuWidgetInput::Refresh);
                }
            }
        }
        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SystemUpdateMenuWidgetCommandOutput::Loaded(report) => {
                self.refreshing = false;
                rebuild_list(&widgets.list, &report);
                self.report = Some(report);
            }
            SystemUpdateMenuWidgetCommandOutput::Run(status) => self.run = status,
            SystemUpdateMenuWidgetCommandOutput::LaunchFailed(e) => self.run_error = Some(e),
            SystemUpdateMenuWidgetCommandOutput::Configs(found) => {
                rebuild_configs(&widgets.config_list, &found, &sender);
                let title = if found.is_empty() {
                    "Config files".to_string()
                } else {
                    format!("Config files ({})", found.len())
                };
                if let Some(page) = widgets.stack.child_by_name("configs") {
                    widgets.stack.page(&page).set_title(&title);
                }
                self.configs = found;
            }
            SystemUpdateMenuWidgetCommandOutput::Reviewed(file, review) => {
                rebuild_diff(&widgets.diff_view, &review);
                self.review = Some((file, review));
            }
            SystemUpdateMenuWidgetCommandOutput::ConfigDone(result) => {
                self.config_busy = false;
                match result {
                    Ok(()) => {
                        self.review = None;
                        self.config_error = None;
                        sender.input(SystemUpdateMenuWidgetInput::ScanConfigs);
                    }
                    Err(e) => self.config_error = Some(format!("Couldn't apply: {e}")),
                }
            }
            SystemUpdateMenuWidgetCommandOutput::History(source, runs, files) => {
                self.armed_rollback = None;
                rebuild_history(&widgets.history_list, source, &runs, &files, None, &sender);
                self.history = Some((source, runs));
                self.cache_files = files;
            }
            SystemUpdateMenuWidgetCommandOutput::News(items) => {
                rebuild_news(&widgets.news_list, &items, &sender);
                self.news = items;
            }
        }
        self.update_view(widgets, sender);
    }
}

fn update_mode() -> SystemUpdateMode {
    config_manager()
        .config()
        .bars()
        .widgets()
        .system_update()
        .update_mode()
        .get_untracked()
}

fn run_title(run: &RunStatus, error: Option<&str>) -> String {
    if let Some(e) = error {
        return format!("Couldn't start the upgrade: {e}");
    }
    match run {
        RunStatus::Idle => String::new(),
        RunStatus::Running { log, expected } if *expected > 0 => {
            format!("Upgrading… {}/{expected}", log.done.min(*expected))
        }
        RunStatus::Running { .. } => "Upgrading…".to_string(),
        RunStatus::Finished { log, .. } => run::result_message(log.exit.unwrap_or(-1), log.done).0,
    }
}

fn run_title_classes(run: &RunStatus, error: bool) -> Vec<&'static str> {
    let failed = error || matches!(run, RunStatus::Finished { log, .. } if log.exit != Some(0));
    if failed {
        vec!["label-medium-bold", "danger"]
    } else {
        vec!["label-medium-bold"]
    }
}

fn run_fraction(run: &RunStatus) -> f64 {
    match run {
        RunStatus::Idle => 0.0,
        RunStatus::Running { log, expected } => run::fraction(log.done, *expected),
        RunStatus::Finished { .. } => 1.0,
    }
}

fn run_tail(run: &RunStatus) -> String {
    match run {
        RunStatus::Running { log, .. } | RunStatus::Finished { log, .. } => {
            let n = log.tail.len();
            log.tail[n.saturating_sub(6)..].join("\n")
        }
        RunStatus::Idle => String::new(),
    }
}

fn news_enabled() -> bool {
    config_manager()
        .config()
        .bars()
        .widgets()
        .system_update()
        .check_news()
        .get_untracked()
}

fn news_card_classes(confirm: bool) -> Vec<&'static str> {
    if confirm {
        vec!["system-update-news", "warn"]
    } else {
        vec!["system-update-news"]
    }
}

fn news_title(n: usize, confirm: bool) -> String {
    let posts = if n == 1 { "post" } else { "posts" };
    if confirm {
        format!("Read {n} Arch news {posts} before upgrading")
    } else {
        format!("{n} Arch news {posts} since your last upgrade")
    }
}

fn update_label(confirm: bool) -> &'static str {
    if confirm {
        "Upgrade anyway"
    } else {
        "Update everything"
    }
}

/// One row per unread post: the title, opening the post on click.
fn rebuild_news(
    list: &gtk::Box,
    items: &[NewsItem],
    sender: &ComponentSender<SystemUpdateMenuWidgetModel>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for item in items {
        let label = gtk::Label::builder()
            .label(&item.title)
            .xalign(0.0)
            .wrap(true)
            .build();
        label.add_css_class("label-small");
        let row = gtk::Button::builder().child(&label).build();
        row.add_css_class("ok-button-flat");
        row.set_tooltip_text(Some("Open in the browser"));
        let url = if item.link.is_empty() {
            news::NEWS_PAGE.to_string()
        } else {
            item.link.clone()
        };
        let s = sender.clone();
        row.connect_clicked(move |_| s.input(SystemUpdateMenuWidgetInput::OpenUrl(url.clone())));
        list.append(&row);
    }
}

fn header_label(report: Option<&UpdateReport>, refreshing: bool) -> String {
    if refreshing {
        return "System Updates — checking…".to_string();
    }
    match report {
        Some(r) if r.error.is_some() => "System Updates — error".to_string(),
        Some(r) if r.total() == 0 => "System Updates — up to date".to_string(),
        Some(r) => format!("System Updates — {} pending", r.total()),
        None => "System Updates".to_string(),
    }
}

fn summary_line(report: Option<&UpdateReport>) -> String {
    let Some(r) = report else {
        return String::new();
    };
    let mut parts: Vec<String> = [Source::Repo, Source::Aur, Source::Flatpak]
        .into_iter()
        .filter_map(|s| {
            let c = r.count(s);
            (c > 0).then(|| format!("{}: {c}", s.label()))
        })
        .collect();
    if let Some(bytes) = r.download_bytes.filter(|b| *b > 0) {
        parts.push(format!(
            "~{} to download",
            system_update::size::human(bytes)
        ));
    }
    parts.join("   ·   ")
}

/// Rebuild the grouped update table: a section header per source
/// followed by `name  old → new` rows.
fn rebuild_list(list: &gtk::Box, report: &UpdateReport) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    if let Some(err) = &report.error {
        list.append(&info_label(err));
        return;
    }
    if report.is_empty() {
        list.append(&info_label("Everything is up to date."));
        return;
    }

    for source in [Source::Repo, Source::Aur, Source::Flatpak] {
        let entries: Vec<_> = report
            .entries
            .iter()
            .filter(|e| e.source == source)
            .collect();
        if entries.is_empty() {
            continue;
        }

        let header = gtk::Label::builder()
            .label(format!("{} ({})", source.label(), entries.len()))
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .build();
        header.add_css_class("system-update-section-label");
        list.append(&header);

        for e in entries {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .build();
            row.add_css_class("system-update-row");

            let name = gtk::Label::builder()
                .label(&e.name)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .xalign(0.0)
                .build();
            name.add_css_class("system-update-name");
            row.append(&name);

            let ver = match (&e.old_version, &e.new_version) {
                (Some(old), Some(new)) => format!("{old} → {new}"),
                (None, Some(new)) => new.clone(),
                _ => String::new(),
            };
            if !ver.is_empty() {
                let ver_label = gtk::Label::builder()
                    .label(&ver)
                    .halign(gtk::Align::End)
                    .xalign(1.0)
                    .build();
                ver_label.add_css_class("system-update-version");
                row.append(&ver_label);
            }

            list.append(&row);
        }
    }
}

/// The History page: one titled boxed-list per transaction, rows
/// `name  old → new` with a rollback button when the old file is still
/// in the pacman cache.
fn rebuild_history(
    list: &gtk::Box,
    source: HistorySource,
    runs: &[Transaction],
    cache_files: &[String],
    armed: Option<&str>,
    sender: &ComponentSender<SystemUpdateMenuWidgetModel>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    if source == HistorySource::LocalDb {
        list.append(&info_label(
            "pacman.log isn't readable, so this is rebuilt from the package database: \
             packages grouped by install time, previous versions from the cache.",
        ));
    }
    if runs.is_empty() {
        list.append(&info_label("No update history found."));
        return;
    }
    for t in runs {
        let when = chrono::DateTime::from_timestamp(t.time, 0)
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%b %-d, %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        let n = t.changes.len();
        let header = gtk::Label::builder()
            .label(format!(
                "{when}  ·  {n} package{}",
                if n == 1 { "" } else { "s" }
            ))
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .build();
        header.add_css_class("system-update-section-label");
        list.append(&header);

        let group = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        group.add_css_class("ok-boxed-list");
        group.add_css_class("compact");
        for c in &t.changes {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .build();
            let name = gtk::Label::builder()
                .label(&c.name)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            name.add_css_class("system-update-name");
            row.append(&name);
            let ver = match (&c.old, &c.new) {
                (Some(o), Some(n)) => format!("{o} → {n}"),
                (None, Some(n)) => format!("new {n}"),
                (Some(o), None) => format!("removed {o}"),
                (None, None) => String::new(),
            };
            let ver_label = gtk::Label::builder()
                .label(&ver)
                .halign(gtk::Align::End)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build();
            ver_label.add_css_class("system-update-version");
            row.append(&ver_label);

            let restorable = c
                .old
                .as_deref()
                .and_then(|o| history::find_cached(&c.name, o, cache_files));
            if let (Some(file), Some(old)) = (restorable, c.old.as_deref()) {
                let btn = if armed == Some(c.name.as_str()) {
                    let b = gtk::Button::with_label("Roll back?");
                    b.add_css_class("ok-button-error");
                    b
                } else {
                    let b = gtk::Button::from_icon_name("edit-undo-symbolic");
                    b.add_css_class("ok-button-flat");
                    b
                };
                btn.set_valign(gtk::Align::Center);
                let tip = format!("Roll back {} to {old} from the pacman cache", c.name);
                btn.set_tooltip_text(Some(&tip));
                mshell_common::a11y::name(&btn, &tip);
                let s = sender.clone();
                let (name, file) = (c.name.clone(), file.to_string());
                btn.connect_clicked(move |_| {
                    s.input(SystemUpdateMenuWidgetInput::Rollback {
                        name: name.clone(),
                        file: file.clone(),
                    })
                });
                row.append(&btn);
            }
            group.append(&row);
        }
        list.append(&group);
    }
}

/// Config file list: one boxed-list row per review file.
fn rebuild_configs(
    list: &gtk::Box,
    files: &[ConfigFile],
    sender: &ComponentSender<SystemUpdateMenuWidgetModel>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    if files.is_empty() {
        list.append(&info_label("No .pacnew or .pacsave files under /etc."));
        return;
    }
    let group = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    group.add_css_class("ok-boxed-list");
    group.add_css_class("compact");
    for (i, f) in files.iter().enumerate() {
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        let path = gtk::Label::builder()
            .label(f.live.display().to_string())
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .build();
        path.add_css_class("system-update-name");
        inner.append(&path);
        let kind = gtk::Label::new(Some(f.kind.label()));
        kind.add_css_class("system-update-version");
        inner.append(&kind);
        inner.append(&gtk::Image::from_icon_name("go-next-symbolic"));
        let row = gtk::Button::builder().child(&inner).build();
        row.add_css_class("ok-button-flat");
        let s = sender.clone();
        row.connect_clicked(move |_| s.input(SystemUpdateMenuWidgetInput::OpenReview(i)));
        group.append(&row);
    }
    list.append(&group);
}

/// Most diff lines the view renders (the rest are summarised).
const MAX_DIFF_LINES: usize = 400;

fn rebuild_diff(view: &gtk::Box, review: &Review) {
    while let Some(child) = view.first_child() {
        view.remove(&child);
    }
    for (kind, text) in review.unified.iter().take(MAX_DIFF_LINES) {
        let l = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::Char)
            .build();
        l.add_css_class(match kind {
            DiffLine::Added => "diff-added",
            DiffLine::Removed => "diff-removed",
            DiffLine::Hunk => "diff-hunk",
            DiffLine::Context => "diff-context",
        });
        view.append(&l);
    }
    if review.unified.len() > MAX_DIFF_LINES {
        view.append(&info_label(&format!(
            "… {} more lines",
            review.unified.len() - MAX_DIFF_LINES
        )));
    }
}

fn review_summary(file: &ConfigFile, r: &Review) -> String {
    if r.unreadable {
        return "Needs root to read — only Delete is available here.".into();
    }
    if r.identical {
        return "Identical to the live file — safe to delete.".into();
    }
    match (&r.merge, &r.base_from) {
        (Some(segs), Some(base)) => match pacnew::conflicts(segs) {
            0 => format!("3-way merge with the default from {base}: merges cleanly."),
            n => format!(
                "3-way merge with the default from {base}: {n} conflict{}.",
                if n == 1 { "" } else { "s" }
            ),
        },
        _ if file.kind == Kind::Pacnew => {
            "2-way diff — the previous default isn't in the pacman cache.".into()
        }
        _ => format!("{} vs the live file.", file.kind.label()),
    }
}

fn review_can_merge(r: Option<&(ConfigFile, Review)>) -> bool {
    r.is_some_and(|(_, r)| r.merge.is_some() && !r.identical)
}

fn review_has_conflicts(r: Option<&(ConfigFile, Review)>) -> bool {
    r.is_some_and(|(_, r)| r.merge.as_ref().is_some_and(|m| pacnew::conflicts(m) > 0))
}

fn merge_label(r: Option<&(ConfigFile, Review)>, _prefer: Prefer) -> &'static str {
    if review_has_conflicts(r) {
        "Merge, keep my lines"
    } else {
        "Apply merge"
    }
}

fn take_label(r: Option<&(ConfigFile, Review)>) -> &'static str {
    match r.map(|(f, _)| f.kind) {
        Some(Kind::Pacnew) => "Replace with new",
        _ => "Restore",
    }
}

fn delete_label(r: Option<&(ConfigFile, Review)>) -> &'static str {
    match r.map(|(f, _)| f.kind) {
        Some(Kind::Pacnew) => "Delete .pacnew",
        Some(Kind::Pacsave) => "Delete .pacsave",
        _ => "Delete .pacorig",
    }
}

fn info_label(text: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .wrap(true)
        .build();
    l.add_css_class("label-small");
    l
}
