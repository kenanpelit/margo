//! Tabbed calendar widget for the dashboard (mdash).
//!
//! Two tabs over a `gtk::Stack`:
//! * **Month** — the same `gtk::Calendar` month grid the full `Calendar` widget
//!   renders, with days that carry events marked. No hero band — the dashboard
//!   fills that role with a separate `Clock` widget.
//! * **Agenda** — the selected day's events (time · title · location).
//!
//! Both tabs are fed by the shared `calendar_data` loader (local `.ics` +
//! remote ICS subscriptions), so the dashboard shows the user's real events
//! without the full widget's duplicate date/time hero.

use super::calendar_data;
use chrono::{Local, NaiveDate};
use relm4::{
    Component, ComponentParts, ComponentSender,
    gtk::{
        self,
        glib::{self, SourceId},
        prelude::*,
    },
};
use time::{Date, OffsetDateTime};

#[derive(Debug)]
pub(crate) struct CalendarGridModel {
    timer_id: Option<SourceId>,
    current_date: Date,
    /// Loaded events (recurrence expanded) across the load window.
    events: Vec<mcal::Event>,
    /// The day the Agenda tab is showing.
    selected: NaiveDate,
    /// Agenda heading, e.g. "Friday, July 4".
    agenda_heading: String,
}

#[derive(Debug)]
pub(crate) enum CalendarGridInput {
    /// Day-rollover tick — re-anchor the grid + agenda on today.
    CheckDayRollover,
    /// Menu reveal state changed — stop/restart the tick and (re)load events.
    ParentRevealChanged(bool),
    /// A fresh event set arrived from the loader.
    EventsLoaded(Vec<mcal::Event>),
    /// The user picked a day on the grid — refill the agenda.
    DaySelected,
    /// The grid navigated to another month/year — re-apply day marks.
    VisibleMonthChanged,
}

#[derive(Debug)]
pub(crate) enum CalendarGridOutput {}

#[derive(Debug)]
pub(crate) enum CalendarGridCommandOutput {
    Loaded(Vec<mcal::Event>),
}

pub(crate) struct CalendarGridInit {}

#[relm4::component(pub)]
impl Component for CalendarGridModel {
    type Input = CalendarGridInput;
    type Output = CalendarGridOutput;
    type Init = CalendarGridInit;
    type CommandOutput = CalendarGridCommandOutput;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "calendar-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,

            // ── Tab switcher ─────────────────────────────────
            gtk::StackSwitcher {
                add_css_class: "calendar-tabs",
                set_halign: gtk::Align::Center,
                set_stack: Some(&tabs),
            },

            #[name = "tabs"]
            gtk::Stack {
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,

                // ───────────── Month tab ─────────────
                add_titled[Some("month"), "Month"] = &gtk::Box {
                    add_css_class: "calendar-grid-card",
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    // GTK4 won't clip the nested GtkCalendar to the card's
                    // border-radius without an explicit overflow.
                    set_overflow: gtk::Overflow::Hidden,

                    #[name = "calendar"]
                    gtk::Calendar {
                        set_can_focus: false,
                        set_focus_on_click: false,
                        set_show_heading: true,
                        set_show_day_names: true,
                        set_hexpand: true,
                        connect_day_selected[sender] => move |_| {
                            sender.input(CalendarGridInput::DaySelected);
                        },
                        connect_prev_month[sender] => move |_| {
                            sender.input(CalendarGridInput::VisibleMonthChanged);
                        },
                        connect_next_month[sender] => move |_| {
                            sender.input(CalendarGridInput::VisibleMonthChanged);
                        },
                        connect_prev_year[sender] => move |_| {
                            sender.input(CalendarGridInput::VisibleMonthChanged);
                        },
                        connect_next_year[sender] => move |_| {
                            sender.input(CalendarGridInput::VisibleMonthChanged);
                        },
                    },
                },

                // ───────────── Agenda tab ─────────────
                add_titled[Some("agenda"), "Agenda"] = &gtk::Box {
                    add_css_class: "calendar-agenda-card",
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    set_spacing: 6,

                    gtk::Label {
                        add_css_class: "calendar-agenda-heading",
                        #[watch]
                        set_label: model.agenda_heading.as_str(),
                        set_halign: gtk::Align::Start,
                    },

                    #[name = "agenda_list"]
                    gtk::Box {
                        add_css_class: "calendar-agenda-list",
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        set_spacing: 4,
                    },

                    #[name = "agenda_empty"]
                    gtk::Label {
                        add_css_class: "calendar-agenda-empty",
                        set_label: "No events",
                        set_halign: gtk::Align::Start,
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
        // 60 s tick — we only act on day rollover; cheap the rest of the time.
        let id = start_tick(&sender);

        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        let today = Local::now().date_naive();

        let model = CalendarGridModel {
            timer_id: Some(id),
            current_date: now.date(),
            events: Vec::new(),
            selected: today,
            agenda_heading: calendar_data::heading(today),
        };

        let widgets = view_output!();

        widgets.calendar.set_year(now.year());
        widgets.calendar.set_month(now.month() as i32 - 1);
        widgets.calendar.set_day(now.day() as i32);

        spawn_load(&sender);

        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            CalendarGridCommandOutput::Loaded(events) => {
                sender.input(CalendarGridInput::EventsLoaded(events));
            }
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            CalendarGridInput::CheckDayRollover => {
                let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
                if now.date() != self.current_date {
                    self.current_date = now.date();
                    widgets.calendar.set_year(now.year());
                    widgets.calendar.set_month(now.month() as i32 - 1);
                    widgets.calendar.set_day(now.day() as i32);
                    // Move the agenda to the new today as well.
                    self.selected = Local::now().date_naive();
                    self.agenda_heading = calendar_data::heading(self.selected);
                    calendar_data::refresh_marks(&widgets.calendar, &self.events);
                    calendar_data::rebuild_agenda(
                        &widgets.agenda_list,
                        &widgets.agenda_empty,
                        &self.events,
                        self.selected,
                    );
                }
            }
            CalendarGridInput::ParentRevealChanged(visible) => {
                if visible {
                    if self.timer_id.is_none() {
                        self.timer_id = Some(start_tick(&sender));
                    }
                    sender.input(CalendarGridInput::CheckDayRollover);
                    spawn_load(&sender);
                } else if let Some(id) = self.timer_id.take() {
                    id.remove();
                }
            }
            CalendarGridInput::EventsLoaded(events) => {
                self.events = events;
                calendar_data::refresh_marks(&widgets.calendar, &self.events);
                calendar_data::rebuild_agenda(
                    &widgets.agenda_list,
                    &widgets.agenda_empty,
                    &self.events,
                    self.selected,
                );
            }
            CalendarGridInput::DaySelected => {
                let date = widgets.calendar.date();
                if let Some(day) = NaiveDate::from_ymd_opt(
                    date.year(),
                    date.month() as u32,
                    date.day_of_month() as u32,
                ) {
                    self.selected = day;
                    self.agenda_heading = calendar_data::heading(day);
                    calendar_data::rebuild_agenda(
                        &widgets.agenda_list,
                        &widgets.agenda_empty,
                        &self.events,
                        self.selected,
                    );
                }
                calendar_data::refresh_marks(&widgets.calendar, &self.events);
            }
            CalendarGridInput::VisibleMonthChanged => {
                calendar_data::refresh_marks(&widgets.calendar, &self.events);
            }
        }

        self.update_view(widgets, sender);
    }
}

/// Kick an off-thread load, delivering the result back as `Loaded`.
fn spawn_load(sender: &ComponentSender<CalendarGridModel>) {
    let config = calendar_data::shell_calendar_config();
    let window = calendar_data::load_window();
    sender.oneshot_command(async move {
        CalendarGridCommandOutput::Loaded(calendar_data::fetch(config, window).await)
    });
}

/// Start the 60 s day-rollover tick.
fn start_tick(sender: &ComponentSender<CalendarGridModel>) -> SourceId {
    let sender = sender.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(60), move || {
        sender.input(CalendarGridInput::CheckDayRollover);
        glib::ControlFlow::Continue
    })
}

impl Drop for CalendarGridModel {
    fn drop(&mut self) {
        if let Some(id) = self.timer_id.take() {
            id.remove();
        }
    }
}
