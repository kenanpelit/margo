//! Settings → Widgets → Alarm Clock → Timers & ringing: how alarms and the
//! one-shot timers ring (`alarm.notifications`, `alarm.snooze_minutes`) and
//! whether the AlarmClock bar pill counts the next timer down
//! (`alarm.show_timer_on_pill`). Shell-owned, through the reactive store;
//! the scheduler and the pill read the values live. Composed with the
//! Pomodoro rules and the menu geometry in `settings/widget_pages.rs`.

use crate::row::Row;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{AlarmConfigStoreFields, ConfigStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct TimerSettingsModel {
    notifications: bool,
    show_timer_on_pill: bool,
    snooze_minutes: u32,
    _effects: EffectScope,
}

reactive_settings! {
    model: TimerSettingsModel,
    input: TimerSettingsInput,
    group: alarm,
    fields {
        Notifications => notifications: bool => notifications,
        ShowOnPill => show_timer_on_pill: bool => show_timer_on_pill,
    }
    clamped {
        Snooze => snooze_minutes: u32 => snooze_minutes in 1 ..= 120,
    }
}

#[derive(Debug)]
pub(crate) enum TimerSettingsOutput {}

pub(crate) struct TimerSettingsInit {}

#[relm4::component(pub)]
impl Component for TimerSettingsModel {
    type CommandOutput = ();
    type Input = TimerSettingsInput;
    type Output = TimerSettingsOutput;
    type Init = TimerSettingsInit;

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

            gtk::Label {
                add_css_class: "label-large-bold",
                set_label: "Timers & ringing",
                set_halign: gtk::Align::Start,
            },
            gtk::Label {
                add_css_class: "label-small",
                set_label: "One-line timers and reminders (\"10m tea\", \"14:30 meeting\", \"10 dk sonra çay\") from the Alarm Clock menu's Timers tab, the launcher's `timer` prefix or `mshellctl timer add`. They ring like alarms.",
                set_halign: gtk::Align::Start,
                set_xalign: 0.0,
                set_wrap: true,
            },
            gtk::Box {
                add_css_class: "boxed-list",
                set_orientation: gtk::Orientation::Vertical,
                #[template]
                Row {
                    set_widget_name: "setting:/alarm/show_timer_on_pill",
                    #[template_child] title { set_label: "Count down on the bar pill" },
                    #[template_child] desc { set_label: "While a timer runs, the Alarm Clock pill shows the next one's time left. Without a timer the pill is unchanged." },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(show_timer_on_pill_handler)]
                        set_active: model.show_timer_on_pill,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(TimerSettingsInput::ShowOnPillChanged(v));
                            glib::Propagation::Proceed
                        } @show_timer_on_pill_handler,
                    },
                },
                #[template]
                Row {
                    set_widget_name: "setting:/alarm/notifications",
                    #[template_child] title { set_label: "Stop / Snooze notification" },
                    #[template_child] desc { set_label: "When an alarm or timer rings, a notification with Stop and Snooze. Off: only the tone, stopped from the pill or the menu." },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(notifications_handler)]
                        set_active: model.notifications,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(TimerSettingsInput::NotificationsChanged(v));
                            glib::Propagation::Proceed
                        } @notifications_handler,
                    },
                },
                #[template]
                Row {
                    set_widget_name: "setting:/alarm/snooze_minutes",
                    #[template_child] title { set_label: "Snooze length" },
                    #[template_child] desc { set_label: "Minutes until a snoozed alarm or timer rings again." },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_numeric: true,
                        set_adjustment: &gtk::Adjustment::new(5.0, 1.0, 120.0, 1.0, 5.0, 0.0),
                        #[watch]
                        #[block_signal(snooze_minutes_handler)]
                        set_value: model.snooze_minutes as f64,
                        connect_value_changed[sender] => move |sb| {
                            sender.input(TimerSettingsInput::SnoozeChanged(sb.value() as u32));
                        } @snooze_minutes_handler,
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
