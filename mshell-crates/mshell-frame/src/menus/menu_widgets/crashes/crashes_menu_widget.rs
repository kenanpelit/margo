//! Crashes menu — the content for `MenuType::Crashes`.
//!
//! Two views behind a segmented control: **Crashes** (your own processes'
//! core dumps from systemd-coredump, newest first, each row with a
//! per-program notification mute) and **Errors** (this boot's journal at
//! `crashes.journal_priority` and above). Data is fetched when the menu is
//! revealed and on the header's refresh button — never while hidden. Every
//! row has a diagnose button that opens Claude Code / Codex in a terminal
//! on it ([`crate::crashes::diagnose`]). See [`crate::crashes`] for the
//! parsers and the notification watcher.

use std::collections::HashSet;

use crate::crashes::diagnose::{self, AgentSettings, Target};
use crate::crashes::{
    self, Crash, Fetch, JournalEntry, format_time, is_muted, journal_priority, signal_name,
};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, CrashesConfigStoreFields};
use mshell_config::schema::menu_widgets::MenuWidget;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrashesTab {
    Crashes,
    Errors,
}

pub(crate) struct CrashesMenuWidgetModel {
    tab: CrashesTab,
    crashes: Option<Fetch<Vec<Crash>>>,
    journal: Option<Fetch<Vec<JournalEntry>>>,
    visible: bool,
    content: gtk::Box,
    /// Rows whose diagnosis is being launched (button disabled meanwhile).
    diagnosing: HashSet<String>,
}

#[derive(Debug)]
pub(crate) enum CrashesMenuWidgetInput {
    ParentRevealChanged(bool),
    SelectTab(CrashesTab),
    Refresh,
    ToggleMute(String),
    Diagnose(String, Target),
}

#[derive(Debug)]
pub(crate) enum CrashesMenuWidgetOutput {}

pub(crate) struct CrashesMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum CrashesMenuWidgetCommandOutput {
    Crashes(Fetch<Vec<Crash>>),
    Journal(Fetch<Vec<JournalEntry>>),
    Diagnosed(String, Result<(), String>),
}

#[relm4::component(pub(crate))]
impl Component for CrashesMenuWidgetModel {
    type CommandOutput = CrashesMenuWidgetCommandOutput;
    type Input = CrashesMenuWidgetInput;
    type Output = CrashesMenuWidgetOutput;
    type Init = CrashesMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "crashes-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            // ── §12 panel header ──
            #[local_ref]
            header_root -> gtk::Box {},

            // ── §12 segmented control (Crashes · Errors) ──
            gtk::Box {
                add_css_class: crate::common_widgets::segmented::TRACK_CLASS,
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                set_homogeneous: true,
                gtk::Button {
                    #[watch]
                    set_css_classes: crate::common_widgets::segmented::segment_classes(
                        crate::common_widgets::segmented::SegmentedStyle::Capsule,
                        model.tab == CrashesTab::Crashes,
                    ),
                    set_hexpand: true,
                    set_label: "Crashes",
                    connect_clicked[sender] => move |_| {
                        sender.input(CrashesMenuWidgetInput::SelectTab(CrashesTab::Crashes));
                    },
                },
                gtk::Button {
                    #[watch]
                    set_css_classes: crate::common_widgets::segmented::segment_classes(
                        crate::common_widgets::segmented::SegmentedStyle::Capsule,
                        model.tab == CrashesTab::Errors,
                    ),
                    set_hexpand: true,
                    set_label: "Journal errors",
                    connect_clicked[sender] => move |_| {
                        sender.input(CrashesMenuWidgetInput::SelectTab(CrashesTab::Errors));
                    },
                },
            },

            #[name = "fill_list"]
            gtk::ScrolledWindow {
                set_vexpand: true,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_max_content_height: 480,
                set_propagate_natural_height: true,

                #[local_ref]
                content -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let s = sender.clone();
        let header = PanelHeader::new(
            "dialog-warning-symbolic",
            MenuWidget::Crashes.display_name(),
        )
        .action("view-refresh-symbolic", "Refresh", move || {
            s.input(CrashesMenuWidgetInput::Refresh)
        });
        let header_root = header.root().clone();
        let model = CrashesMenuWidgetModel {
            tab: CrashesTab::Crashes,
            crashes: None,
            journal: None,
            visible: false,
            content: content.clone(),
            diagnosing: HashSet::new(),
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 480);
        model.render(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            CrashesMenuWidgetInput::ParentRevealChanged(visible) => {
                let opened = visible && !self.visible;
                self.visible = visible;
                if opened {
                    self.fetch(&sender);
                }
            }
            CrashesMenuWidgetInput::SelectTab(tab) => {
                self.tab = tab;
                if tab == CrashesTab::Errors && self.journal.is_none() {
                    self.fetch(&sender);
                }
                self.render(&sender);
            }
            CrashesMenuWidgetInput::Refresh => self.fetch(&sender),
            CrashesMenuWidgetInput::ToggleMute(program) => {
                config_manager().update_config(move |c| {
                    let muted = &mut c.crashes.muted;
                    if is_muted(muted, &program) {
                        muted.retain(|m| !m.trim().eq_ignore_ascii_case(&program));
                    } else {
                        muted.push(program);
                    }
                });
                self.render(&sender);
            }
            CrashesMenuWidgetInput::Diagnose(key, target) => {
                if !self.diagnosing.insert(key.clone()) {
                    return;
                }
                let settings = AgentSettings::from_config();
                sender.oneshot_command(async move {
                    let r = diagnose::diagnose(target, settings).await;
                    CrashesMenuWidgetCommandOutput::Diagnosed(key, r)
                });
                self.render(&sender);
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
            CrashesMenuWidgetCommandOutput::Crashes(f) => self.crashes = Some(f),
            CrashesMenuWidgetCommandOutput::Journal(f) => self.journal = Some(f),
            CrashesMenuWidgetCommandOutput::Diagnosed(key, r) => {
                self.diagnosing.remove(&key);
                if let Err(e) = r {
                    tracing::warn!("crashes: diagnosis: {e}");
                    mshell_config::toast::push_toast(mshell_config::toast::ToastEvent {
                        icon: "ai-agents-symbolic".to_string(),
                        title: "Couldn't start the diagnosis".to_string(),
                        body: Some(e),
                        severity: mshell_config::toast::ToastSeverity::Warn,
                    });
                }
            }
        }
        self.render(&sender);
    }
}

impl CrashesMenuWidgetModel {
    /// Fetch the current tab's data (the journal only once its tab is
    /// opened — `journalctl` is the heavier call).
    fn fetch(&self, sender: &ComponentSender<Self>) {
        match self.tab {
            CrashesTab::Crashes => sender.oneshot_command(async {
                CrashesMenuWidgetCommandOutput::Crashes(crashes::fetch_crashes().await)
            }),
            CrashesTab::Errors => {
                let level = journal_priority(
                    &config_manager()
                        .config()
                        .crashes()
                        .journal_priority()
                        .get_untracked(),
                );
                sender.oneshot_command(async move {
                    CrashesMenuWidgetCommandOutput::Journal(crashes::fetch_journal(level).await)
                })
            }
        }
    }

    fn render(&self, sender: &ComponentSender<Self>) {
        let list = &self.content;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        list.set_css_classes(&[]);
        let agents = AgentSettings::from_config().agents;
        let can_diagnose = diagnose::any_agent_installed(&agents);
        match self.tab {
            CrashesTab::Crashes => match &self.crashes {
                None => status_line(list, "Loading…"),
                Some(Fetch::Missing(tool)) => status_line(
                    list,
                    &format!("{tool} not found — install systemd to see crashes"),
                ),
                Some(Fetch::Failed(e)) => error_line(list, &format!("Couldn't read crashes: {e}")),
                Some(Fetch::Ok(v)) if v.is_empty() => status_line(list, "No crashes recorded"),
                Some(Fetch::Ok(v)) => {
                    list.set_css_classes(&["ok-boxed-list", "compact", "crashes-list"]);
                    let muted = config_manager().config().crashes().muted().get_untracked();
                    for c in v {
                        let key = crash_key(c);
                        let diag = DiagnoseButton {
                            available: can_diagnose,
                            busy: self.diagnosing.contains(&key),
                            key,
                        };
                        list.append(&crash_row(c, is_muted(&muted, c.program()), diag, sender));
                    }
                }
            },
            CrashesTab::Errors => match &self.journal {
                None => status_line(list, "Loading…"),
                Some(Fetch::Missing(tool)) => status_line(list, &format!("{tool} not found")),
                Some(Fetch::Failed(e)) => {
                    error_line(list, &format!("Couldn't read the journal: {e}"))
                }
                Some(Fetch::Ok(v)) if v.is_empty() => {
                    status_line(list, "No errors in this boot's journal")
                }
                Some(Fetch::Ok(v)) => {
                    list.set_css_classes(&["ok-boxed-list", "compact", "crashes-list"]);
                    for e in v {
                        let key = journal_key(e);
                        let diag = DiagnoseButton {
                            available: can_diagnose,
                            busy: self.diagnosing.contains(&key),
                            key,
                        };
                        list.append(&journal_row(e, diag, sender));
                    }
                }
            },
        }
    }
}

/// Calm, dim one-liner for loading / empty / no-service (§17).
fn status_line(list: &gtk::Box, text: &str) {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("label-small");
    l.add_css_class("crashes-status");
    l.set_wrap(true);
    l.set_xalign(0.0);
    list.append(&l);
}

/// Danger-tinted inline error (§17), with the header's refresh as retry.
fn error_line(list: &gtk::Box, text: &str) {
    status_line(list, text);
    if let Some(l) = list.last_child() {
        l.add_css_class("danger");
    }
}

/// Title + dim subtitle column used by both row kinds.
fn text_column(title: &str, subtitle: &str) -> gtk::Box {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("crashes-row-title");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    col.append(&t);
    let s = gtk::Label::new(Some(subtitle));
    s.add_css_class("crashes-row-subtitle");
    s.set_xalign(0.0);
    s.set_wrap(true);
    s.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    s.set_lines(3);
    s.set_ellipsize(gtk::pango::EllipsizeMode::End);
    col.append(&s);
    col
}

/// Diagnosis state for one row's button.
struct DiagnoseButton {
    key: String,
    /// An agent is installed.
    available: bool,
    /// Launching right now.
    busy: bool,
}

fn crash_key(c: &Crash) -> String {
    format!("c{}-{}", c.time_us, c.pid)
}

fn journal_key(e: &JournalEntry) -> String {
    format!("j{}-{}", e.time_us, e.ident)
}

/// The diagnose button (`what` names the row for the tooltip / a11y).
fn diagnose_button(
    diag: DiagnoseButton,
    what: &str,
    target: Target,
    sender: &ComponentSender<CrashesMenuWidgetModel>,
) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_css_classes(&["ok-button-surface", "crashes-diagnose"]);
    b.set_valign(gtk::Align::Center);
    b.set_child(Some(&gtk::Image::from_icon_name("ai-agents-symbolic")));
    let label = format!("Diagnose {what} with an AI agent");
    let tip = if !diag.available {
        "Install Claude Code or Codex to diagnose".to_string()
    } else if diag.busy {
        "Opening the agent…".to_string()
    } else {
        format!("{label} (opens a terminal)")
    };
    b.set_tooltip_text(Some(&tip));
    mshell_common::a11y::name(&b, &label);
    b.set_sensitive(diag.available && !diag.busy);
    let sender = sender.clone();
    let key = diag.key;
    b.connect_clicked(move |_| {
        sender.input(CrashesMenuWidgetInput::Diagnose(
            key.clone(),
            target.clone(),
        ));
    });
    b
}

fn crash_row(
    c: &Crash,
    muted: bool,
    diag: DiagnoseButton,
    sender: &ComponentSender<CrashesMenuWidgetModel>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("crashes-row");
    row.set_tooltip_text(Some(&c.exe));

    let mut sub = format!(
        "{} · PID {} · {}",
        signal_name(c.sig),
        c.pid,
        format_time(c.time_us)
    );
    if !c.core_present {
        sub.push_str(" · core file gone");
    }
    row.append(&text_column(c.program(), &sub));
    row.append(&diagnose_button(
        diag,
        &format!("the {} crash", c.program()),
        Target::Crash(c.clone()),
        sender,
    ));

    let program = c.program().to_string();
    let mute = gtk::Button::new();
    mute.set_css_classes(if muted {
        &["ok-button-surface", "crashes-mute", "selected"]
    } else {
        &["ok-button-surface", "crashes-mute"]
    });
    mute.set_valign(gtk::Align::Center);
    mute.set_child(Some(&gtk::Image::from_icon_name(if muted {
        "notification-disabled-symbolic"
    } else {
        "notification-symbolic"
    })));
    let label = if muted {
        format!("Unmute crash notifications for {program}")
    } else {
        format!("Mute crash notifications for {program}")
    };
    mute.set_tooltip_text(Some(&label));
    mshell_common::a11y::name(&mute, &label);
    let sender = sender.clone();
    mute.connect_clicked(move |_| {
        sender.input(CrashesMenuWidgetInput::ToggleMute(program.clone()));
    });
    row.append(&mute);
    row
}

/// §2 severity for a syslog priority: crit and worse = danger, err = warn.
fn priority_class(priority: u8) -> &'static str {
    match priority {
        0..=2 => "danger",
        3 => "warn",
        _ => "calm",
    }
}

fn priority_label(priority: u8) -> &'static str {
    crashes::PRIORITIES
        .get(priority as usize)
        .copied()
        .unwrap_or("info")
}

fn journal_row(
    e: &JournalEntry,
    diag: DiagnoseButton,
    sender: &ComponentSender<CrashesMenuWidgetModel>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("crashes-row");
    let title = format!("{} · {}", e.ident, format_time(e.time_us));
    row.append(&text_column(&title, &e.message));
    let chip = gtk::Label::new(Some(priority_label(e.priority)));
    chip.add_css_class("crashes-priority");
    chip.add_css_class(priority_class(e.priority));
    chip.set_valign(gtk::Align::Center);
    row.append(&chip);
    row.append(&diagnose_button(
        diag,
        &format!("this {} message", e.ident),
        Target::Journal(e.clone(), priority_label(e.priority)),
        sender,
    ));
    row
}

#[cfg(test)]
mod tests {
    use super::{priority_class, priority_label};

    #[test]
    fn priorities_map_to_the_severity_ladder_and_names() {
        assert_eq!(priority_class(0), "danger");
        assert_eq!(priority_class(2), "danger");
        assert_eq!(priority_class(3), "warn");
        assert_eq!(priority_class(4), "calm");
        assert_eq!(priority_label(3), "err");
        assert_eq!(priority_label(4), "warning");
        assert_eq!(priority_label(6), "info");
    }
}
