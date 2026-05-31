//! Alarm Clock menu widget — the panel content for
//! `MenuType::AlarmClock`. Ports the DMS `alarmClock` plugin as a
//! two-tab panel:
//!
//! * **Alarms** — a reactive list of the alarms stored in
//!   `config.alarm.alarms` (per-row enable switch, time, repeat-day
//!   summary, delete) plus an add row (hour/minute, name, repeat-day
//!   toggles). A "Stop ringing" banner appears whenever the tone is
//!   sounding.
//! * **Stopwatch** — a start / pause / reset stopwatch backed by the
//!   transient [`crate::stopwatch`] global.
//!
//! The alarm engine (scheduler + tone + Stop/Snooze notification)
//! lives in `mshell-core`'s IPC service; this widget is purely the
//! editor + stopwatch UI. Edits go through `config_manager().
//! update_config`, which persists + reloads, so the reactive effect
//! below repaints the list — including the one-shot alarms the
//! scheduler disables after they fire.

use crate::stopwatch::{self, StopwatchState};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{Alarm, AlarmConfigStoreFields, ConfigStoreFields};
use mshell_sounds::{alarm_is_ringing, stop_alarm};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, ToggleButtonExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

/// Single-letter day labels in repeat-mask bit order (bit 0 = Sunday,
/// matching the scheduler's `weekday = day_of_week() % 7`).
const DAY_LETTERS: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];
/// Short day names for the per-alarm repeat summary, same bit order.
const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

pub(crate) struct AlarmClockMenuWidgetModel {
    /// Vertical container the alarm rows are rebuilt into.
    alarm_list: gtk::Box,
    /// Shown in place of the list when there are no alarms.
    empty_hint: gtk::Label,
    /// "Stop ringing" banner — visible only while the tone sounds.
    ringing_banner: gtk::Button,
    /// Header subtitle ("3 alarms · 1 on").
    status_label: gtk::Label,
    // ── add row ──
    add_hour: gtk::SpinButton,
    add_minute: gtk::SpinButton,
    add_name: gtk::Entry,
    day_toggles: Vec<gtk::ToggleButton>,
    // ── stopwatch ──
    sw_time: gtk::Label,
    sw_start_btn: gtk::Button,
    sw_pause_btn: gtk::Button,
    sw_reset_btn: gtk::Button,
    /// Keeps the reactive subscription on `config.alarm.alarms` alive.
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum AlarmClockMenuWidgetInput {
    /// Reactive: the alarms vector changed — rebuild the list + header.
    AlarmsChanged,
    ToggleAlarm(usize, bool),
    DeleteAlarm(usize),
    AddAlarm,
    StopRinging,
    SwStart,
    SwPause,
    SwReset,
}

pub(crate) struct AlarmClockMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for AlarmClockMenuWidgetModel {
    type CommandOutput = ();
    type Input = AlarmClockMenuWidgetInput;
    type Output = ();
    type Init = AlarmClockMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "alarm-clock-menu-widget",
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
                    set_icon_name: Some("alarm-symbolic"),
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    gtk::Label {
                        add_css_class: "panel-title",
                        set_halign: gtk::Align::Start,
                        set_label: "Alarm Clock",
                    },
                    #[local_ref]
                    status_label_widget -> gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                    },
                },
            },

            // ── "Stop ringing" banner ──
            #[local_ref]
            ringing_banner_widget -> gtk::Button {
                add_css_class: "alarm-ringing-banner",
                set_visible: false,
                connect_clicked[sender] => move |_| {
                    sender.input(AlarmClockMenuWidgetInput::StopRinging);
                },
                gtk::Box {
                    set_spacing: 8,
                    set_halign: gtk::Align::Center,
                    gtk::Image { set_icon_name: Some("alarm-symbolic") },
                    gtk::Label { set_label: "Ringing — Stop" },
                },
            },

            // ── tab switcher ──
            gtk::StackSwitcher {
                add_css_class: "alarm-clock-tabs",
                set_stack: Some(&tabs),
                set_halign: gtk::Align::Fill,
            },

            #[name = "tabs"]
            gtk::Stack {
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                set_vhomogeneous: false,

                // ───────────── Alarms tab ─────────────
                add_titled[Some("alarms"), "Alarms"] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 10,

                    // alarm rows live here (rebuilt imperatively)
                    #[local_ref]
                    alarm_list_widget -> gtk::Box {
                        add_css_class: "alarm-list",
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 6,
                    },
                    #[local_ref]
                    empty_hint_widget -> gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Center,
                        set_label: "No alarms yet — add one below.",
                    },

                    gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

                    // ── add row ──
                    gtk::Box {
                        add_css_class: "alarm-add-row",
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 8,

                        gtk::Box {
                            set_spacing: 8,
                            set_halign: gtk::Align::Center,
                            #[local_ref]
                            add_hour_widget -> gtk::SpinButton {
                                add_css_class: "alarm-spin",
                                set_tooltip_text: Some("Hour (0–23)"),
                            },
                            gtk::Label { set_label: ":" },
                            #[local_ref]
                            add_minute_widget -> gtk::SpinButton {
                                add_css_class: "alarm-spin",
                                set_tooltip_text: Some("Minute (0–59)"),
                            },
                            #[local_ref]
                            add_name_widget -> gtk::Entry {
                                set_hexpand: true,
                                set_placeholder_text: Some("Label (optional)"),
                            },
                        },

                        // repeat-day toggles
                        #[name = "day_row"]
                        gtk::Box {
                            add_css_class: "alarm-day-row",
                            set_spacing: 4,
                            set_halign: gtk::Align::Center,
                            set_homogeneous: true,
                        },

                        gtk::Button {
                            add_css_class: "ok-button-surface",
                            add_css_class: "ok-button-cell",
                            set_label: "Add alarm",
                            connect_clicked[sender] => move |_| {
                                sender.input(AlarmClockMenuWidgetInput::AddAlarm);
                            },
                        },
                    },
                },

                // ───────────── Stopwatch tab ─────────────
                add_titled[Some("stopwatch"), "Stopwatch"] = &gtk::Box {
                    add_css_class: "alarm-stopwatch",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 16,
                    set_valign: gtk::Align::Center,

                    #[local_ref]
                    sw_time_widget -> gtk::Label {
                        add_css_class: "alarm-stopwatch-time",
                        set_halign: gtk::Align::Center,
                        set_label: "00:00.00",
                    },

                    gtk::Box {
                        set_spacing: 8,
                        set_homogeneous: true,
                        #[local_ref]
                        sw_start_widget -> gtk::Button {
                            add_css_class: "ok-button-surface",
                            add_css_class: "ok-button-cell",
                            set_label: "Start",
                            connect_clicked[sender] => move |_| {
                                sender.input(AlarmClockMenuWidgetInput::SwStart);
                            },
                        },
                        #[local_ref]
                        sw_pause_widget -> gtk::Button {
                            add_css_class: "ok-button-surface",
                            add_css_class: "ok-button-cell",
                            set_label: "Pause",
                            connect_clicked[sender] => move |_| {
                                sender.input(AlarmClockMenuWidgetInput::SwPause);
                            },
                        },
                        #[local_ref]
                        sw_reset_widget -> gtk::Button {
                            add_css_class: "ok-button-surface",
                            add_css_class: "ok-button-cell",
                            set_label: "Reset",
                            connect_clicked[sender] => move |_| {
                                sender.input(AlarmClockMenuWidgetInput::SwReset);
                            },
                        },
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
        let status_label_widget = gtk::Label::new(None);
        let ringing_banner_widget = gtk::Button::new();
        let alarm_list_widget = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let empty_hint_widget = gtk::Label::new(None);
        let add_hour_widget = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
        let add_minute_widget = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
        let add_name_widget = gtk::Entry::new();
        let sw_time_widget = gtk::Label::new(None);
        let sw_start_widget = gtk::Button::new();
        let sw_pause_widget = gtk::Button::new();
        let sw_reset_widget = gtk::Button::new();

        // Default the add row to 07:00 — the plugin's default alarm.
        add_hour_widget.set_value(7.0);
        add_minute_widget.set_value(0.0);

        // Reactive subscription: repaint the list whenever the alarms
        // vector changes (user edits here, or the scheduler disabling a
        // one-shot after it fires).
        let mut effects = EffectScope::new();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let _ = config_manager().config().alarm().alarms().get();
            sender_clone.input(AlarmClockMenuWidgetInput::AlarmsChanged);
        });

        let model = AlarmClockMenuWidgetModel {
            alarm_list: alarm_list_widget.clone(),
            empty_hint: empty_hint_widget.clone(),
            ringing_banner: ringing_banner_widget.clone(),
            status_label: status_label_widget.clone(),
            add_hour: add_hour_widget.clone(),
            add_minute: add_minute_widget.clone(),
            add_name: add_name_widget.clone(),
            day_toggles: Vec::new(),
            sw_time: sw_time_widget.clone(),
            sw_start_btn: sw_start_widget.clone(),
            sw_pause_btn: sw_pause_widget.clone(),
            sw_reset_btn: sw_reset_widget.clone(),
            _effects: effects,
        };
        let widgets = view_output!();

        // Build the repeat-day toggle chips for the add row.
        let mut day_toggles = Vec::with_capacity(7);
        for letter in DAY_LETTERS {
            let t = gtk::ToggleButton::with_label(letter);
            t.add_css_class("alarm-day-toggle");
            t.set_focusable(false);
            widgets.day_row.append(&t);
            day_toggles.push(t);
        }

        let mut model = model;
        model.day_toggles = day_toggles;

        rebuild_list(&model, &sender);
        sync_stopwatch(&model);
        sync_ringing(&model);

        // Heartbeat — only relayouts GTK while something is moving
        // (stopwatch running or tone ringing), so a parked menu costs
        // nothing.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            // Send one trailing tick after activity stops so the
            // readout settles (e.g. the banner hides when the tone
            // stops from the notification rather than the banner).
            let mut was_active = false;
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tick.tick() => {
                        let active =
                            stopwatch::state() == StopwatchState::Running || alarm_is_ringing();
                        if active || was_active {
                            let _ = out.send(());
                        }
                        was_active = active;
                    }
                }
            }
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AlarmClockMenuWidgetInput::AlarmsChanged => {
                rebuild_list(self, &sender);
            }
            AlarmClockMenuWidgetInput::ToggleAlarm(idx, on) => {
                config_manager().update_config(move |c| {
                    if let Some(a) = c.alarm.alarms.get_mut(idx) {
                        a.enabled = on;
                    }
                });
            }
            AlarmClockMenuWidgetInput::DeleteAlarm(idx) => {
                config_manager().update_config(move |c| {
                    if idx < c.alarm.alarms.len() {
                        c.alarm.alarms.remove(idx);
                    }
                });
            }
            AlarmClockMenuWidgetInput::AddAlarm => {
                let hour = self.add_hour.value() as u8;
                let minutes = self.add_minute.value() as u8;
                let name = self.add_name.text().trim().to_string();
                let mut repeat_mask = 0u8;
                for (bit, toggle) in self.day_toggles.iter().enumerate() {
                    if toggle.is_active() {
                        repeat_mask |= 1 << bit;
                    }
                }
                config_manager().update_config(move |c| {
                    c.alarm.alarms.push(Alarm {
                        hour,
                        minutes,
                        name,
                        enabled: true,
                        repeat_mask,
                    });
                });
                // Reset the form for the next entry.
                self.add_name.set_text("");
                for toggle in &self.day_toggles {
                    toggle.set_active(false);
                }
            }
            AlarmClockMenuWidgetInput::StopRinging => {
                stop_alarm();
                sync_ringing(self);
            }
            AlarmClockMenuWidgetInput::SwStart => {
                stopwatch::start();
                sync_stopwatch(self);
            }
            AlarmClockMenuWidgetInput::SwPause => {
                stopwatch::pause();
                sync_stopwatch(self);
            }
            AlarmClockMenuWidgetInput::SwReset => {
                stopwatch::reset();
                sync_stopwatch(self);
            }
        }
    }

    fn update_cmd(
        &mut self,
        _message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        // Cheap per-tick refresh of the live readouts only.
        self.sw_time
            .set_label(&stopwatch::format_elapsed(stopwatch::elapsed()));
        sync_ringing(self);
    }
}

/// Rebuild the alarm rows from the persisted config + refresh the
/// header subtitle + empty-state hint.
fn rebuild_list(
    model: &AlarmClockMenuWidgetModel,
    sender: &ComponentSender<AlarmClockMenuWidgetModel>,
) {
    while let Some(child) = model.alarm_list.first_child() {
        model.alarm_list.remove(&child);
    }

    let alarms = config_manager().config().alarm().alarms().get_untracked();
    let enabled = alarms.iter().filter(|a| a.enabled).count();
    model.status_label.set_label(&format!(
        "{} alarm{} · {} on",
        alarms.len(),
        if alarms.len() == 1 { "" } else { "s" },
        enabled
    ));
    model.empty_hint.set_visible(alarms.is_empty());

    for (idx, alarm) in alarms.iter().enumerate() {
        model.alarm_list.append(&alarm_row(idx, alarm, sender));
    }

    sync_ringing(model);
}

/// One alarm row: enable switch · time · label/repeat · delete.
fn alarm_row(
    idx: usize,
    alarm: &Alarm,
    sender: &ComponentSender<AlarmClockMenuWidgetModel>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("alarm-row");
    if !alarm.enabled {
        row.add_css_class("alarm-row-off");
    }

    let toggle = gtk::Switch::new();
    toggle.set_active(alarm.enabled);
    toggle.set_valign(gtk::Align::Center);
    {
        let sender = sender.clone();
        toggle.connect_state_set(move |_, state| {
            sender.input(AlarmClockMenuWidgetInput::ToggleAlarm(idx, state));
            glib::Propagation::Proceed
        });
    }
    row.append(&toggle);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    let time = gtk::Label::new(Some(&format!("{:02}:{:02}", alarm.hour, alarm.minutes)));
    time.add_css_class("alarm-row-time");
    time.set_halign(gtk::Align::Start);
    text.append(&time);

    let subtitle = match (alarm.name.trim(), repeat_summary(alarm.repeat_mask)) {
        ("", repeat) => repeat,
        (name, repeat) => format!("{name} · {repeat}"),
    };
    let sub = gtk::Label::new(Some(&subtitle));
    sub.add_css_class("label-small");
    sub.set_halign(gtk::Align::Start);
    sub.set_xalign(0.0);
    text.append(&sub);
    row.append(&text);

    let delete = gtk::Button::from_icon_name("user-trash-symbolic");
    delete.add_css_class("alarm-row-delete");
    delete.set_valign(gtk::Align::Center);
    delete.set_tooltip_text(Some("Delete alarm"));
    {
        let sender = sender.clone();
        delete.connect_clicked(move |_| {
            sender.input(AlarmClockMenuWidgetInput::DeleteAlarm(idx));
        });
    }
    row.append(&delete);

    row
}

/// Human-readable repeat description from a weekday bitmask.
fn repeat_summary(mask: u8) -> String {
    if mask == 0 {
        return "Once".to_string();
    }
    if mask & 0b0111_1111 == 0b0111_1111 {
        return "Every day".to_string();
    }
    // Mon–Fri = bits 1..=5.
    if mask & 0b0111_1111 == 0b0011_1110 {
        return "Weekdays".to_string();
    }
    // Sat + Sun = bits 6 and 0.
    if mask & 0b0111_1111 == 0b0100_0001 {
        return "Weekends".to_string();
    }
    (0..7)
        .filter(|b| mask & (1 << b) != 0)
        .map(|b| DAY_NAMES[b])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Refresh the stopwatch readout + button states from the global.
fn sync_stopwatch(model: &AlarmClockMenuWidgetModel) {
    let state = stopwatch::state();
    model
        .sw_time
        .set_label(&stopwatch::format_elapsed(stopwatch::elapsed()));
    model
        .sw_start_btn
        .set_visible(state != StopwatchState::Running);
    model.sw_start_btn.set_label(match state {
        StopwatchState::Paused => "Resume",
        _ => "Start",
    });
    model
        .sw_pause_btn
        .set_visible(state == StopwatchState::Running);
    model
        .sw_reset_btn
        .set_sensitive(state != StopwatchState::Stopped);
}

/// Toggle the "Stop ringing" banner to match the tone state.
fn sync_ringing(model: &AlarmClockMenuWidgetModel) {
    model.ringing_banner.set_visible(alarm_is_ringing());
}
