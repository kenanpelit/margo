//! UFW firewall bar pill.
//!
//! Shows the firewall's state at a glance, read without root and without
//! a prompt ([`service`]): the saved configuration (`ufw.conf`, the rule
//! files) and the `ufw` systemd unit.
//!
//!   * green `shield-check` — on and enforced;
//!   * red `firewall` — off (`ENABLED=no`);
//!   * amber `firewall-error` — on, but the unit failed / isn't running,
//!     or doesn't start at boot (the tooltip names the fix, the menu
//!     offers it);
//!   * muted `firewall-error` — ufw missing, or its state unreadable.
//!
//! Optionally (`bars.widgets.ufw.show_count`) the number of incoming
//! allow / limit rules sits next to the icon while the firewall is on.
//! Click emits `UfwOutput::Clicked`; `frame.rs` toggles `MenuType::Ufw`.

pub(crate) mod service;

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, UfwBarWidgetStoreFields,
};
use mshell_privileged::ufw::state::{self, Level, ReadError, Snapshot};
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct UfwModel {
    snap: Arc<Snapshot>,
    show_count: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum UfwInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum UfwOutput {
    Clicked,
}

pub(crate) struct UfwInit {}

#[derive(Debug)]
pub(crate) enum UfwCommandOutput {
    State(Arc<Snapshot>),
    ShowCount(bool),
}

#[relm4::component(pub)]
impl Component for UfwModel {
    type CommandOutput = UfwCommandOutput;
    type Input = UfwInput;
    type Output = UfwOutput;
    type Init = UfwInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "ufw-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(UfwInput::Clicked);
                },

                gtk::Box {
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,

                    #[name="image"]
                    gtk::Image {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                    },

                    #[name="count"]
                    gtk::Label {
                        add_css_class: "ufw-bar-count",
                        set_valign: gtk::Align::Center,
                        set_visible: false,
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // The pill is always shown: it holds the shared watcher's
        // subscription for as long as it lives.
        sender.command(|out, shutdown| async move {
            let mut rx = service::subscribe();
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let snap = rx.borrow_and_update().clone();
                let _ = out.send(UfwCommandOutput::State(snap));
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = rx.changed() => if changed.is_err() { break },
                }
            }
        });

        let mut effects = EffectScope::new();
        let cs = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .ufw()
                .show_count()
                .get();
            let _ = cs.command_sender().send(UfwCommandOutput::ShowCount(v));
        });

        let model = UfwModel {
            snap: service::current(),
            show_count: config_manager()
                .config()
                .bars()
                .widgets()
                .ufw()
                .show_count()
                .get_untracked(),
            _effects: effects,
        };

        let widgets = view_output!();
        apply_visual(&widgets, &root, &model);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            UfwInput::Clicked => {
                let _ = sender.output(UfwOutput::Clicked);
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
            UfwCommandOutput::State(snap) => self.snap = snap,
            UfwCommandOutput::ShowCount(v) => self.show_count = v,
        }
        apply_visual(widgets, root, self);
    }
}

/// What the pill shows for a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PillView {
    pub icon: &'static str,
    /// The state class on the pill root (`_ufw.scss` tints the icon).
    pub class: &'static str,
    /// The allowed-in rule count, when shown.
    pub count: Option<usize>,
    pub tooltip: String,
}

/// Every state class [`pill_view`] may set (toggled one at a time).
const STATE_CLASSES: [&str; 4] = ["active", "inactive", "warn", "unknown"];

pub(crate) fn pill_view(snap: &Snapshot, show_count: bool) -> PillView {
    let health = state::classify(snap);
    let (icon, class) = match health.level {
        Level::Green => ("shield-check-symbolic", "active"),
        Level::Red => ("firewall-symbolic", "inactive"),
        Level::Amber => ("firewall-error-symbolic", "warn"),
        Level::Muted => ("firewall-error-symbolic", "unknown"),
    };
    let allowed = snap.rules.as_ref().ok().map(|r| r.allowed_in());
    let mut lines = vec![format!("UFW firewall: {}", health.headline)];
    if health.enforcing()
        && let Some(n) = allowed
    {
        lines.push(match n {
            0 => "No rule allows incoming traffic".to_string(),
            1 => "1 rule allows incoming traffic".to_string(),
            n => format!("{n} rules allow incoming traffic"),
        });
    }
    if !health.detail.is_empty() {
        lines.push(health.detail.clone());
    }
    if !health.fixes.is_empty() {
        let fixes: Vec<&str> = health.fixes.iter().map(|f| f.label()).collect();
        lines.push(format!("Fix in the menu: {}", fixes.join(" · ")));
    }
    if snap.installed && snap.rules == Err(ReadError::Denied) {
        lines.push("The rules need root to read: open the menu → Load rules".to_string());
    }
    lines.push("Click: open the firewall menu".to_string());
    PillView {
        icon,
        class,
        count: allowed.filter(|_| show_count && health.enforcing()),
        tooltip: lines.join("\n"),
    }
}

fn apply_visual(widgets: &UfwModelWidgets, root: &gtk::Box, model: &UfwModel) {
    let v = pill_view(&model.snap, model.show_count);
    widgets.image.set_icon_name(Some(v.icon));
    match v.count {
        Some(n) => {
            widgets.count.set_label(&n.to_string());
            widgets.count.set_visible(true);
        }
        None => widgets.count.set_visible(false),
    }
    root.set_tooltip_text(Some(&v.tooltip));
    for c in STATE_CLASSES {
        if c != v.class && root.has_css_class(c) {
            root.remove_css_class(c);
        }
    }
    if !root.has_css_class(v.class) {
        root.add_css_class(v.class);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_privileged::ufw::rules::{Conf, parse_rules};
    use mshell_privileged::ufw::state::{UnitRead, parse_show};

    fn snap(enabled: bool, show: &str) -> Snapshot {
        Snapshot {
            installed: true,
            conf: Ok(Conf {
                enabled: Some(enabled),
                loglevel: None,
            }),
            defaults: None,
            unit: UnitRead::Unit(parse_show(show)),
            rules: Ok(parse_rules(
                "### tuple ### allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 in\n\
                 ### tuple ### deny udp 53 0.0.0.0/0 any 0.0.0.0/0 out\n\
                 ### tuple ### limit tcp 2222 0.0.0.0/0 any 0.0.0.0/0 in\n### END RULES ###\n",
                None,
                false,
            )),
        }
    }

    const ON: &str = "LoadState=loaded\nActiveState=active\nUnitFileState=enabled\n";

    #[test]
    fn green_shows_the_count_only_when_asked() {
        let v = pill_view(&snap(true, ON), true);
        assert_eq!(
            (v.icon, v.class, v.count),
            ("shield-check-symbolic", "active", Some(2))
        );
        assert!(
            v.tooltip
                .starts_with("UFW firewall: On\n2 rules allow incoming traffic"),
            "{}",
            v.tooltip
        );
        assert_eq!(pill_view(&snap(true, ON), false).count, None);
    }

    #[test]
    fn red_amber_and_muted() {
        let off = pill_view(&snap(false, ON), true);
        assert_eq!((off.class, off.count), ("inactive", None));
        let amber = pill_view(
            &snap(
                true,
                "LoadState=loaded\nActiveState=inactive\nUnitFileState=disabled\n",
            ),
            true,
        );
        assert_eq!(
            (amber.icon, amber.class, amber.count),
            ("firewall-error-symbolic", "warn", None)
        );
        assert!(
            amber
                .tooltip
                .contains("Fix in the menu: Start ufw · Start at boot"),
            "{}",
            amber.tooltip
        );
        let mut missing = snap(true, ON);
        missing.installed = false;
        let m = pill_view(&missing, true);
        assert_eq!(m.class, "unknown");
        assert!(m.tooltip.contains("ufw isn't installed"));
        let mut denied = snap(true, ON);
        denied.rules = Err(ReadError::Denied);
        let d = pill_view(&denied, true);
        assert_eq!((d.class, d.count), ("active", None));
        assert!(d.tooltip.contains("Load rules"));
        assert_eq!(pill_view(&Snapshot::loading(), true).class, "unknown");
    }
}
