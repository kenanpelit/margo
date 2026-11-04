//! System Updates menu widget — the panel content for
//! `MenuType::SystemUpdate`. Ports the noctalia arch-updater panel:
//! a table of pending updates grouped by source (Repo / AUR /
//! Flatpak), each row showing `name  old → new`, plus Refresh and
//! Update buttons. Probing lives in [`crate::system_update`].

use crate::system_update::news::{self, NewsItem};
use crate::system_update::{self, ProbeConfig, Source, UpdateReport};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, SystemUpdateBarWidgetStoreFields,
};
use mshell_process::SpawnReaped;
use reactive_graph::traits::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
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

            gtk::ScrolledWindow {
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

            // Update button — runs the upgrade in a terminal.
            gtk::Button {
                add_css_class: "ok-button-primary",
                set_halign: gtk::Align::Fill,
                #[watch]
                set_sensitive: model.report.as_ref().is_some_and(|r| !r.is_empty()),
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
        };
        let widgets = view_output!();
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
            SystemUpdateMenuWidgetInput::Update => {
                self.news_confirm = false;
                relm4::spawn(async {
                    system_update::launch_terminal_upgrade(ProbeConfig::from_config()).await;
                });
                let _ = sender.output(SystemUpdateMenuWidgetOutput::CloseMenu);
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
            SystemUpdateMenuWidgetCommandOutput::News(items) => {
                rebuild_news(&widgets.news_list, &items, &sender);
                self.news = items;
            }
        }
        self.update_view(widgets, sender);
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
    [Source::Repo, Source::Aur, Source::Flatpak]
        .into_iter()
        .filter_map(|s| {
            let c = r.count(s);
            (c > 0).then(|| format!("{}: {c}", s.label()))
        })
        .collect::<Vec<_>>()
        .join("   ·   ")
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
