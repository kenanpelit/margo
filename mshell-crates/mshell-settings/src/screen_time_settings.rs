//! Settings → Widgets → Screen Time (`config.screen_time`). The owner task in
//! `mshell-frame/src/screen_time` starts, stops and re-reads these live.

use crate::row::Row;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ScreenTimeConfigStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct ScreenTimeSettingsModel {
    enabled: bool,
    away_seconds: u32,
    hide_under_minutes: u32,
    retention_days: u32,
    _effects: EffectScope,
}

reactive_settings! {
    model: ScreenTimeSettingsModel,
    input: ScreenTimeSettingsInput,
    group: screen_time,
    fields {
        Enabled => enabled: bool => enabled,
    }
    clamped {
        AwaySeconds => away_seconds: u32 => away_seconds in 30 ..= 3600,
        HideUnderMinutes => hide_under_minutes: u32 => hide_under_minutes in 0 ..= 120,
        RetentionDays => retention_days: u32 => retention_days in 1 ..= 3650,
    }
}

#[derive(Debug)]
pub(crate) enum ScreenTimeSettingsOutput {}

pub(crate) struct ScreenTimeSettingsInit {}

#[relm4::component(pub)]
impl Component for ScreenTimeSettingsModel {
    type CommandOutput = ();
    type Input = ScreenTimeSettingsInput;
    type Output = ScreenTimeSettingsOutput;
    type Init = ScreenTimeSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("preferences-system-time-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Screen Time",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "How long each app has the keyboard focus, per day. Time pauses while you are away or the session is locked. Add the Screen Time pill to a bar, or open `mshellctl menu screen-time`.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Recording",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[template]
                    Row {
                        set_widget_name: "setting:/screen_time/enabled",
                        #[template_child] title { set_label: "Record screen time" },
                        #[template_child] desc { set_label: "Off by default. Nothing is recorded until this is on; history stays on this machine ($XDG_STATE_HOME/margo/screen-time.json)." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(enabled_handler)]
                            set_active: model.enabled,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(ScreenTimeSettingsInput::EnabledChanged(v));
                                glib::Propagation::Proceed
                            } @enabled_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/screen_time/away_seconds",
                        #[template_child] title { set_label: "Away after" },
                        #[template_child] desc { set_label: "Seconds without keyboard or mouse input after which time stops counting. A playing video keeps you present." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_digits: 0,
                            set_adjustment: &gtk::Adjustment::new(30.0, 30.0, 3600.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(away_seconds_handler)]
                            set_value: model.away_seconds as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(ScreenTimeSettingsInput::AwaySecondsChanged(sb.value() as u32));
                            } @away_seconds_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/screen_time/hide_under_minutes",
                        #[template_child] title { set_label: "Hide apps under" },
                        #[template_child] desc { set_label: "Minutes: apps used less than this on the shown range are left out of the lists (they still count in the total). 0 lists every app." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_digits: 0,
                            set_adjustment: &gtk::Adjustment::new(0.0, 0.0, 120.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(hide_under_minutes_handler)]
                            set_value: model.hide_under_minutes as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(ScreenTimeSettingsInput::HideUnderMinutesChanged(sb.value() as u32));
                            } @hide_under_minutes_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/screen_time/retention_days",
                        #[template_child] title { set_label: "Keep history for" },
                        #[template_child] desc { set_label: "Days. Older days are dropped from the history file." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_digits: 0,
                            set_adjustment: &gtk::Adjustment::new(1.0, 1.0, 3650.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(retention_days_handler)]
                            set_value: model.retention_days as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(ScreenTimeSettingsInput::RetentionDaysChanged(sb.value() as u32));
                            } @retention_days_handler,
                        },
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
        let model = Self::from_config_store(&sender);
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        self.apply_reactive(message);
    }
}
