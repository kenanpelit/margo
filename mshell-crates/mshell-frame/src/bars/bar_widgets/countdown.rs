//! Countdown — bar pill surfacing the soonest countdown target.
//!
//! Shows "42 days remaining" (horizontal) / "42d" (vertical) for the
//! nearest upcoming `alarm.countdowns` entry, or the overdue form once
//! the date has passed. Click opens the Alarm Clock menu on its
//! Countdown tab. Hidden whenever no enabled, parseable target exists,
//! so an empty list costs no bar space.

use crate::countdown::{self, CountdownUnit};
use chrono::Local;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{AlarmConfigStoreFields, ConfigStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

pub(crate) struct CountdownModel {
    orientation: Orientation,
    root_box: gtk::Box,
    label: gtk::Label,
    /// Keeps the reactive subscription on `alarm.countdowns` alive.
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum CountdownInput {
    Clicked,
    Refresh,
}

#[derive(Debug)]
pub(crate) enum CountdownOutput {
    Clicked,
}

pub(crate) struct CountdownInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for CountdownModel {
    type CommandOutput = ();
    type Input = CountdownInput;
    type Output = CountdownOutput;
    type Init = CountdownInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "countdown-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_tooltip_text: Some("Countdown"),

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(CountdownInput::Clicked);
                },

                gtk::Box {
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    gtk::Image {
                        set_icon_name: Some("timer-symbolic"),
                    },
                    #[local_ref]
                    label_widget -> gtk::Label {
                        add_css_class: "countdown-bar-time",
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let label_widget = gtk::Label::new(None);

        // Repaint whenever the countdown list changes (menu edits).
        let mut effects = EffectScope::new();
        let refresh_sender = sender.clone();
        effects.push(move |_| {
            let _ = config_manager().config().alarm().countdowns().get();
            refresh_sender.input(CountdownInput::Refresh);
        });

        let model = CountdownModel {
            orientation: params.orientation,
            root_box: root.clone(),
            label: label_widget.clone(),
            _effects: effects,
        };
        let widgets = view_output!();

        refresh(&model);

        // Coarse tick so the readout advances as time passes (0.1-unit
        // resolution — a minute is plenty; a hidden pill just re-runs a
        // cheap calc).
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tick.tick() => {
                        let _ = out.send(());
                    }
                }
            }
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            CountdownInput::Clicked => {
                // Land the menu on the Countdown tab (see crate::countdown).
                countdown::request_countdown_tab();
                let _ = sender.output(CountdownOutput::Clicked);
            }
            CountdownInput::Refresh => refresh(self),
        }
    }

    fn update_cmd(
        &mut self,
        _message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        refresh(self);
    }
}

/// Recompute the soonest target and repaint (or hide) the pill.
fn refresh(model: &CountdownModel) {
    let items = config_manager()
        .config()
        .alarm()
        .countdowns()
        .get_untracked();
    let now = Local::now().naive_local();
    if let Some(idx) = countdown::soonest(&items, now) {
        let c = &items[idx];
        let unit = CountdownUnit::parse(&c.unit);
        if let Some(val) = countdown::remaining(&c.target, unit, now) {
            let text = if model.orientation == Orientation::Vertical {
                countdown::format_short(val, unit)
            } else {
                countdown::format_long(val, unit, &c.label)
            };
            model.label.set_label(&text);
            model.root_box.set_visible(true);
            return;
        }
    }
    model.root_box.set_visible(false);
}
