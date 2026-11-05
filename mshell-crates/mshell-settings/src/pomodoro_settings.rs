//! Settings → Widgets → Alarm Clock → Pomodoro: the focus / break cycle
//! rules (`config.pomodoro`). Shell-owned, through the reactive store; the
//! running timer picks edits up live (a phase that hasn't started takes the
//! new length, a running one keeps its time). Composed with the generic
//! Alarm Clock menu page in `settings/widget_pages.rs`.

use crate::row::Row;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, PomodoroStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct PomodoroSettingsModel {
    auto_start_breaks: bool,
    auto_start_work: bool,
    notifications: bool,
    sound: bool,
    work_minutes: u32,
    short_break_minutes: u32,
    long_break_minutes: u32,
    sessions_before_long_break: u32,
    _effects: EffectScope,
}

reactive_settings! {
    model: PomodoroSettingsModel,
    input: PomodoroSettingsInput,
    group: pomodoro,
    fields {
        AutoBreaks => auto_start_breaks: bool => auto_start_breaks,
        AutoWork => auto_start_work: bool => auto_start_work,
        Notifications => notifications: bool => notifications,
        Sound => sound: bool => sound,
    }
    clamped {
        Work => work_minutes: u32 => work_minutes in 1 ..= 600,
        ShortBreak => short_break_minutes: u32 => short_break_minutes in 1 ..= 600,
        LongBreak => long_break_minutes: u32 => long_break_minutes in 1 ..= 600,
        Sessions => sessions_before_long_break: u32 => sessions_before_long_break in 1 ..= 12,
    }
}

#[derive(Debug)]
pub(crate) enum PomodoroSettingsOutput {}

pub(crate) struct PomodoroSettingsInit {}

#[relm4::component(pub)]
impl Component for PomodoroSettingsModel {
    type CommandOutput = ();
    type Input = PomodoroSettingsInput;
    type Output = PomodoroSettingsOutput;
    type Init = PomodoroSettingsInit;

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
                        set_icon_name: Some("timer-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Pomodoro",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Focus sessions with short breaks and a long break every few rounds. Run it from the Alarm Clock menu's Pomodoro tab, the Pomodoro bar pill, or `mshellctl pomodoro`.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Lengths",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[template]
                    Row {
                        set_widget_name: "setting:/pomodoro/work_minutes",
                        #[template_child] title { set_label: "Focus session" },
                        #[template_child] desc { set_label: "Minutes of work per session." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(1.0, 1.0, 600.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(work_minutes_handler)]
                            set_value: model.work_minutes as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(PomodoroSettingsInput::WorkChanged(sb.value() as u32));
                            } @work_minutes_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/pomodoro/short_break_minutes",
                        #[template_child] title { set_label: "Short break" },
                        #[template_child] desc { set_label: "Minutes of rest after each focus session." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(1.0, 1.0, 600.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(short_break_minutes_handler)]
                            set_value: model.short_break_minutes as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(PomodoroSettingsInput::ShortBreakChanged(sb.value() as u32));
                            } @short_break_minutes_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/pomodoro/long_break_minutes",
                        #[template_child] title { set_label: "Long break" },
                        #[template_child] desc { set_label: "Minutes of rest after the last session of a round." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(1.0, 1.0, 600.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(long_break_minutes_handler)]
                            set_value: model.long_break_minutes as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(PomodoroSettingsInput::LongBreakChanged(sb.value() as u32));
                            } @long_break_minutes_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/pomodoro/sessions_before_long_break",
                        #[template_child] title { set_label: "Sessions per round" },
                        #[template_child] desc { set_label: "Focus sessions before the long break." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_numeric: true,
                            set_adjustment: &gtk::Adjustment::new(1.0, 1.0, 12.0, 1.0, 5.0, 0.0),
                            #[watch]
                            #[block_signal(sessions_before_long_break_handler)]
                            set_value: model.sessions_before_long_break as f64,
                            connect_value_changed[sender] => move |sb| {
                                sender.input(PomodoroSettingsInput::SessionsChanged(sb.value() as u32));
                            } @sessions_before_long_break_handler,
                        },
                    },
                },
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Flow",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    #[template]
                    Row {
                        set_widget_name: "setting:/pomodoro/auto_start_breaks",
                        #[template_child] title { set_label: "Start breaks automatically" },
                        #[template_child] desc { set_label: "When a focus session ends, its break starts by itself." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(auto_start_breaks_handler)]
                            set_active: model.auto_start_breaks,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(PomodoroSettingsInput::AutoBreaksChanged(v));
                                glib::Propagation::Proceed
                            } @auto_start_breaks_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/pomodoro/auto_start_work",
                        #[template_child] title { set_label: "Start focus automatically" },
                        #[template_child] desc { set_label: "When a break ends, the next focus session starts by itself." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(auto_start_work_handler)]
                            set_active: model.auto_start_work,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(PomodoroSettingsInput::AutoWorkChanged(v));
                                glib::Propagation::Proceed
                            } @auto_start_work_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/pomodoro/notifications",
                        #[template_child] title { set_label: "Notify when a phase ends" },
                        #[template_child] desc { set_label: "A desktop notification says what ended and what comes next." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(notifications_handler)]
                            set_active: model.notifications,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(PomodoroSettingsInput::NotificationsChanged(v));
                                glib::Propagation::Proceed
                            } @notifications_handler,
                        },
                    },
                    #[template]
                    Row {
                        set_widget_name: "setting:/pomodoro/sound",
                        #[template_child] title { set_label: "Chime when a phase ends" },
                        #[template_child] desc { set_label: "Plays the timer chime (the notification itself stays silent)." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(sound_handler)]
                            set_active: model.sound,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(PomodoroSettingsInput::SoundChanged(v));
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
