//! Settings → Widgets → Eye Care: the 20-20-20 reminder (`config.eyecare`).
//! Shell-owned, through the reactive store; the reminder service in
//! `mshell-frame/src/eyecare` starts, stops and re-times itself live from
//! these values.

use crate::row::Row;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, EyeCareStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct EyeCareSettingsModel {
    enabled: bool,
    notifications: bool,
    sound: bool,
    active_minutes: u32,
    break_seconds: u32,
    away_seconds: u32,
    _effects: EffectScope,
}

reactive_settings! {
    model: EyeCareSettingsModel,
    input: EyeCareSettingsInput,
    group: eyecare,
    fields {
        Enabled => enabled: bool => enabled,
        Notifications => notifications: bool => notifications,
        Sound => sound: bool => sound,
    }
    clamped {
        Active => active_minutes: u32 => active_minutes in 1 ..= 240,
        Break => break_seconds: u32 => break_seconds in 5 ..= 600,
        Away => away_seconds: u32 => away_seconds in 30 ..= 3600,
    }
}

#[derive(Debug)]
pub(crate) enum EyeCareSettingsOutput {}

pub(crate) struct EyeCareSettingsInit {}

#[relm4::component(pub)]
impl Component for EyeCareSettingsModel {
    type CommandOutput = ();
    type Input = EyeCareSettingsInput;
    type Output = EyeCareSettingsOutput;
    type Init = EyeCareSettingsInit;

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
                        set_icon_name: Some("eye-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Eye Care",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The 20-20-20 rule: every 20 minutes of screen time, look at something about 6 m (20 ft) away for 20 seconds. Only time at the keyboard counts; stepping away resets the count. Add the Eye Care pill to a bar, or use `mshellctl eyecare`.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Reminder",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[template]
                    Row {
                        set_widget_name: "setting:/eyecare/enabled",
                        #[template_child] title { set_label: "Eye care reminders" },
                        #[template_child] desc { set_label: "Off by default. Nothing runs until this is on." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(enabled_handler)]
                            set_active: model.enabled,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(EyeCareSettingsInput::EnabledChanged(v));
                                glib::Propagation::Proceed
                            } @enabled_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/eyecare/active_minutes",
                        #[template_child] title { set_label: "Screen time between breaks" },
                        #[template_child] desc { set_label: "Minutes of active screen time before a break." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(1.0, 1.0, 240.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(active_minutes_handler)]
                            set_value: model.active_minutes as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(EyeCareSettingsInput::ActiveChanged(sb.value() as u32));
                            } @active_minutes_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/eyecare/break_seconds",
                        #[template_child] title { set_label: "Break length" },
                        #[template_child] desc { set_label: "Seconds to look away." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(5.0, 5.0, 600.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(break_seconds_handler)]
                            set_value: model.break_seconds as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(EyeCareSettingsInput::BreakChanged(sb.value() as u32));
                            } @break_seconds_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/eyecare/away_seconds",
                        #[template_child] title { set_label: "Away after" },
                        #[template_child] desc { set_label: "Seconds without keyboard or mouse input that count as stepping away (a natural break). Never shorter than a break. A playing video keeps you present." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(30.0, 30.0, 3600.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(away_seconds_handler)]
                            set_value: model.away_seconds as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(EyeCareSettingsInput::AwayChanged(sb.value() as u32));
                            } @away_seconds_handler,
                        },
                    },
                },
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Alerts",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[template]
                    Row {
                        set_widget_name: "setting:/eyecare/notifications",
                        #[template_child] title { set_label: "Notify on break start and end" },
                        #[template_child] desc { set_label: "A desktop notification when a break is due and when it is over." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(notifications_handler)]
                            set_active: model.notifications,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(EyeCareSettingsInput::NotificationsChanged(v));
                                glib::Propagation::Proceed
                            } @notifications_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/eyecare/sound",
                        #[template_child] title { set_label: "Chime on break start and end" },
                        #[template_child] desc { set_label: "Skipped while Do Not Disturb is on." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(sound_handler)]
                            set_active: model.sound,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(EyeCareSettingsInput::SoundChanged(v));
                                glib::Propagation::Proceed
                            } @sound_handler,
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
