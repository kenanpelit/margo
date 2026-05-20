//! SystemUpdate — bar pill showing the count of pending updates
//! across the enabled sources (official repo / AUR / Flatpak).
//!
//! Polls every N minutes (default 180; configurable in Settings →
//! System Updates, alongside the per-source toggles). Left click
//! opens the System Updates panel listing every pending package;
//! right click forces an immediate re-probe. The actual probing +
//! upgrade logic lives in [`crate::system_update`] so the pill, the
//! panel, and the launcher all share one implementation.

use crate::system_update::{self, ProbeConfig, Source, UpdateReport};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, SystemUpdateBarWidgetStoreFields,
};
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tokio::sync::Notify;

/// First probe lands shortly after launch — long enough not to fight
/// cold-boot CPU, short enough to be meaningful by the time the user
/// looks at the bar.
const STARTUP_DELAY: Duration = Duration::from_secs(10);
/// Defensive floor on the configured interval so we never hammer the
/// repo mirrors.
const MIN_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) struct SystemUpdateModel {
    /// `None` until the first probe completes (rendered as
    /// "checking…"); then the latest report.
    report: Option<UpdateReport>,
    _orientation: Orientation,
    /// Wakes the polling task for an immediate re-probe (right-click).
    refresh_notify: std::sync::Arc<Notify>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum SystemUpdateInput {
    /// Left click → open the System Updates panel.
    Clicked,
    /// Right click → immediate manual re-probe.
    ManualRefresh,
}

#[derive(Debug)]
pub(crate) enum SystemUpdateOutput {
    /// Left click — the frame opens the System Updates panel.
    Clicked,
}

pub(crate) struct SystemUpdateInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum SystemUpdateCommandOutput {
    /// Background poll landed a fresh report.
    Refreshed(UpdateReport),
    /// Right-click hit — drop to "checking…" while the probe re-runs.
    Checking,
}

#[relm4::component(pub)]
impl Component for SystemUpdateModel {
    type CommandOutput = SystemUpdateCommandOutput;
    type Input = SystemUpdateInput;
    type Output = SystemUpdateOutput;
    type Init = SystemUpdateInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            #[watch]
            set_css_classes: &css_classes(model.report.as_ref()),
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(model.report.as_ref())),

            gtk::Button {
                set_css_classes: &["ok-button-flat", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(SystemUpdateInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        #[watch]
                        set_icon_name: Some(icon_for(model.report.as_ref())),
                    },
                    gtk::Label {
                        add_css_class: "system-update-bar-label",
                        #[watch]
                        set_label: &label_for(model.report.as_ref()),
                        #[watch]
                        set_visible: count_of(model.report.as_ref()) > 0,
                    },
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let refresh_notify = std::sync::Arc::new(Notify::new());

        let notify_for_task = refresh_notify.clone();
        sender.command(move |out, shutdown| {
            let notify = notify_for_task;
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut first = true;
                loop {
                    let delay = if first { STARTUP_DELAY } else { configured_interval() };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                        _ = notify.notified() => {}
                    }
                    let report = system_update::probe(ProbeConfig::from_config()).await;
                    let _ = out.send(SystemUpdateCommandOutput::Refreshed(report));
                }
            }
        });

        // Subscribe so a future "wake on config change" can plug in
        // without restructuring (interval + per-source toggles).
        let mut effects = EffectScope::new();
        effects.push(|_| {
            // Subscribe to the interval (a future migration can wake
            // the loop on change). Each reactive accessor consumes
            // the chain, so just track the one field.
            let _ = config_manager()
                .config()
                .bars()
                .widgets()
                .system_update()
                .check_interval_minutes()
                .get();
        });

        let model = SystemUpdateModel {
            report: None,
            _orientation: params.orientation,
            refresh_notify,
            _effects: effects,
        };
        let widgets = view_output!();

        // Right-click → manual probe, wired on the root so the whole
        // pill area responds (the Button eats left clicks).
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let refresh_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            refresh_sender.input(SystemUpdateInput::ManualRefresh);
        });
        widgets.root.add_controller(gesture);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SystemUpdateInput::Clicked => {
                // Open the System Updates panel; the panel's Update
                // button runs the upgrade.
                let _ = sender.output(SystemUpdateOutput::Clicked);
            }
            SystemUpdateInput::ManualRefresh => {
                let cmd_sender = sender.command_sender().clone();
                let _ = cmd_sender.send(SystemUpdateCommandOutput::Checking);
                self.refresh_notify.notify_one();
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
            SystemUpdateCommandOutput::Refreshed(report) => self.report = Some(report),
            SystemUpdateCommandOutput::Checking => self.report = None,
        }
    }
}

// ── Config helper ───────────────────────────────────────────────

fn configured_interval() -> Duration {
    let minutes = config_manager()
        .config()
        .bars()
        .widgets()
        .system_update()
        .check_interval_minutes()
        .get_untracked();
    let dur = Duration::from_secs((minutes as u64).saturating_mul(60));
    if dur < MIN_INTERVAL { MIN_INTERVAL } else { dur }
}

// ── View helpers ────────────────────────────────────────────────

fn count_of(report: Option<&UpdateReport>) -> usize {
    report.map(|r| r.total()).unwrap_or(0)
}

fn css_classes(report: Option<&UpdateReport>) -> Vec<&'static str> {
    let mut classes = vec!["ok-button-surface", "ok-bar-widget", "system-update-bar-widget"];
    match report {
        Some(r) if r.error.is_some() => classes.push("error"),
        Some(r) if r.total() > 0 => classes.push("has-updates"),
        _ => {}
    }
    classes
}

fn icon_for(report: Option<&UpdateReport>) -> &'static str {
    match report {
        Some(r) if r.error.is_some() => "software-update-urgent-symbolic",
        Some(r) if r.total() > 0 => "software-update-available-symbolic",
        _ => "package-symbolic",
    }
}

fn label_for(report: Option<&UpdateReport>) -> String {
    match count_of(report) {
        n if n > 0 => n.to_string(),
        _ => String::new(),
    }
}

fn tooltip(report: Option<&UpdateReport>) -> String {
    let footer = "\n\nClick: open panel  ·  Right-click: re-check";
    let Some(r) = report else {
        return format!("Updates: checking…{footer}");
    };
    if let Some(err) = &r.error {
        return format!("Updates: {err}{footer}");
    }
    if r.is_empty() {
        return format!("System is up to date{footer}");
    }
    let mut lines = vec![format!("{} update(s) pending", r.total())];
    for src in [Source::Repo, Source::Aur, Source::Flatpak] {
        let c = r.count(src);
        if c > 0 {
            lines.push(format!("  {}: {c}", src.label()));
        }
    }
    format!("{}{footer}", lines.join("\n"))
}
