//! Valent — bar pill showing the main paired phone's connection
//! state + battery level. Left click opens the Valent Connect panel;
//! right click kicks a device-discovery refresh. The probing +
//! actions live in [`crate::valent`] so the pill and the panel share
//! one implementation.

use crate::valent::{self, ValentReport};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ValentStoreFields};
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tokio::sync::Notify;

/// First probe lands shortly after launch.
const STARTUP_DELAY: Duration = Duration::from_secs(4);
/// Poll cadence — matches the plugin's 5 s timer.
const INTERVAL: Duration = Duration::from_secs(5);

pub(crate) struct ValentModel {
    report: Option<ValentReport>,
    _orientation: Orientation,
    /// Wakes the polling task for an immediate re-probe (right-click).
    refresh_notify: std::sync::Arc<Notify>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum ValentInput {
    /// Left click → open the Valent panel.
    Clicked,
    /// Right click → discovery refresh + immediate re-probe.
    ManualRefresh,
}

#[derive(Debug)]
pub(crate) enum ValentOutput {
    /// Left click — the frame opens the Valent panel.
    Clicked,
}

pub(crate) struct ValentInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ValentCommandOutput {
    Refreshed(ValentReport),
}

#[relm4::component(pub)]
impl Component for ValentModel {
    type CommandOutput = ValentCommandOutput;
    type Input = ValentInput;
    type Output = ValentOutput;
    type Init = ValentInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "valent-bar-widget"],
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
                    sender.input(ValentInput::Clicked);
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
                        add_css_class: "valent-bar-label",
                        #[watch]
                        set_label: &battery_label(model.report.as_ref()),
                        #[watch]
                        set_visible: !battery_label(model.report.as_ref()).is_empty(),
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
                    let delay = if first { STARTUP_DELAY } else { INTERVAL };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                        _ = notify.notified() => {}
                    }
                    let report = valent::probe().await;
                    let _ = out.send(ValentCommandOutput::Refreshed(report));
                }
            }
        });

        // Repaint the icon/label when the sticky device id changes
        // (the panel's device switcher writes it).
        let mut effects = EffectScope::new();
        effects.push(|_| {
            let _ = config_manager().config().valent().main_device_id().get();
        });

        let model = ValentModel {
            report: None,
            _orientation: params.orientation,
            refresh_notify,
            _effects: effects,
        };
        let widgets = view_output!();

        // Right-click → discovery refresh, wired on the root so the
        // whole pill responds (the Button eats left clicks).
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let refresh_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            refresh_sender.input(ValentInput::ManualRefresh);
        });
        widgets.root.add_controller(gesture);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ValentInput::Clicked => {
                let _ = sender.output(ValentOutput::Clicked);
            }
            ValentInput::ManualRefresh => {
                let notify = self.refresh_notify.clone();
                relm4::spawn(async move {
                    valent::refresh_discovery().await;
                    notify.notify_one();
                });
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
            ValentCommandOutput::Refreshed(report) => self.report = Some(report),
        }
    }
}

// ── View helpers ────────────────────────────────────────────────

fn preferred_id() -> String {
    config_manager()
        .config()
        .valent()
        .main_device_id()
        .get_untracked()
}

fn icon_for(report: Option<&ValentReport>) -> &'static str {
    match report {
        Some(r) => r.pill_icon(&preferred_id()),
        None => "phone-symbolic",
    }
}

fn battery_label(report: Option<&ValentReport>) -> String {
    let Some(r) = report else {
        return String::new();
    };
    match r.main_device(&preferred_id()) {
        Some(d) => d
            .battery_charge
            .filter(|_| d.reachable)
            .map(|c| format!("{c}%"))
            .unwrap_or_default(),
        None => String::new(),
    }
}

fn tooltip(report: Option<&ValentReport>) -> String {
    let footer = "\n\nClick: open panel  ·  Right-click: refresh";
    let Some(r) = report else {
        return format!("Valent: checking…{footer}");
    };
    if !r.daemon_available {
        return format!("Valent daemon not running{footer}");
    }
    let Some(d) = r.main_device(&preferred_id()) else {
        return format!("No paired devices{footer}");
    };
    let mut lines = vec![d.name.clone()];
    if !d.reachable {
        lines.push("Disconnected".to_string());
    } else {
        if let Some(c) = d.battery_charge {
            let chg = if d.battery_charging {
                " (charging)"
            } else {
                ""
            };
            lines.push(format!("Battery: {c}%{chg}"));
        }
        if !d.network_type.is_empty() {
            lines.push(format!("Network: {}", d.network_type));
        }
    }
    format!("{}{footer}", lines.join("\n"))
}
