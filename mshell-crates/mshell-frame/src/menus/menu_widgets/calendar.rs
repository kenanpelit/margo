//! Clock-menu calendar widget — noctalia-style card stack, now event-aware.
//!
//! Top card ("hero"): primary-coloured rectangle showing the big day number,
//! month + year, weekday, and live clock readout — the panel header.
//!
//! Middle card: month grid (built-in `gtk::Calendar` styled via CSS). Days that
//! carry at least one event are marked. Prev / next month nav and today
//! highlighting come from GTK.
//!
//! Bottom card: an **agenda** for the selected day (title · time · location),
//! fed by the `mcal` calendar core. Selecting a day on the grid refills it.
//!
//! Data: `mcal::load_all` reads the local calendar dir (`~/.config/margo/
//! calendars`) — and, once configured, remote ICS subscriptions — off the GTK
//! thread via a relm4 command; the result comes back as `EventsLoaded`. The
//! load is (re)kicked on menu reveal so newly-dropped `.ics` files show up.
//!
//! Updates: a 1 Hz glib timer refreshes the clock readout. On day rollover the
//! hero + grid selection move to the new today; otherwise only the time label
//! re-renders so the calendar's navigation state isn't disturbed.

use chrono::{Local, NaiveDate};
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use reactive_graph::traits::GetUntracked;
use relm4::{
    Component, ComponentParts, ComponentSender,
    gtk::{
        self,
        glib::{self, SourceId},
        pango,
        prelude::*,
    },
    once_cell,
};
use time::format_description::parse;
use time::{Date, OffsetDateTime};

/// How far either side of "now" we load events for, so month navigation
/// within roughly a year needs no reload.
const LOAD_WINDOW_DAYS: i64 = 400;

static TIME_FORMAT_24: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| {
        parse("[hour repr:24 padding:zero]:[minute padding:zero]").unwrap()
    });

static TIME_FORMAT_12: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| {
        parse("[hour repr:12 padding:zero]:[minute padding:zero] [period case:lower]").unwrap()
    });

static MONTH_FORMAT: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| parse("[month repr:long] [year]").unwrap());

static WEEKDAY_FORMAT: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| parse("[weekday repr:long]").unwrap());

#[derive(Debug)]
pub(crate) struct CalendarModel {
    timer_id: Option<SourceId>,
    current_date: Date,
    hero_day: String,
    hero_month: String,
    hero_weekday: String,
    hero_time: String,
    format_24_h: bool,
    /// All loaded events (recurrence already expanded) across the load window.
    events: Vec<mcal::Event>,
    /// The grid day the agenda is showing.
    selected: NaiveDate,
    /// Agenda card heading, e.g. "Friday, July 4".
    agenda_heading: String,
}

#[derive(Debug)]
pub(crate) enum CalendarInput {
    UpdateTime,
    /// Menu reveal state changed — the 1 Hz tick is stopped while the
    /// menu is closed and restarted (with an immediate refresh) on show,
    /// and events are (re)loaded on show.
    ParentRevealChanged(bool),
    /// A fresh event set arrived from the loader.
    EventsLoaded(Vec<mcal::Event>),
    /// The user picked a day on the grid — refill the agenda.
    DaySelected,
    /// The grid navigated to another month/year — re-apply day marks.
    VisibleMonthChanged,
}

#[derive(Debug)]
pub(crate) enum CalendarOutput {}

#[derive(Debug)]
pub(crate) enum CalendarCommandOutput {
    Loaded(Vec<mcal::Event>),
}

pub(crate) struct CalendarInit {}

#[relm4::component(pub)]
impl Component for CalendarModel {
    type Input = CalendarInput;
    type Output = CalendarOutput;
    type Init = CalendarInit;
    type CommandOutput = CalendarCommandOutput;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "calendar-menu-widget",
            set_hexpand: true,
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // ── Hero card ────────────────────────────────────
            gtk::Box {
                add_css_class: "calendar-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 16,
                set_hexpand: true,

                gtk::Label {
                    add_css_class: "calendar-hero-day",
                    #[watch]
                    set_label: model.hero_day.as_str(),
                    set_valign: gtk::Align::Center,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                    set_spacing: 0,

                    gtk::Label {
                        add_css_class: "calendar-hero-month",
                        #[watch]
                        set_label: model.hero_month.as_str(),
                        set_halign: gtk::Align::Start,
                    },

                    gtk::Label {
                        add_css_class: "calendar-hero-weekday",
                        #[watch]
                        set_label: model.hero_weekday.as_str(),
                        set_halign: gtk::Align::Start,
                    },
                },

                gtk::Label {
                    add_css_class: "calendar-hero-time",
                    #[watch]
                    set_label: model.hero_time.as_str(),
                    set_valign: gtk::Align::Center,
                    set_halign: gtk::Align::End,
                },
            },

            // ── Month grid card ──────────────────────────────
            gtk::Box {
                add_css_class: "calendar-grid-card",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_overflow: gtk::Overflow::Hidden,

                #[name = "calendar"]
                gtk::Calendar {
                    set_can_focus: false,
                    set_focus_on_click: false,
                    set_show_heading: true,
                    set_show_day_names: true,
                    set_hexpand: true,
                    connect_day_selected[sender] => move |_| {
                        sender.input(CalendarInput::DaySelected);
                    },
                    connect_prev_month[sender] => move |_| {
                        sender.input(CalendarInput::VisibleMonthChanged);
                    },
                    connect_next_month[sender] => move |_| {
                        sender.input(CalendarInput::VisibleMonthChanged);
                    },
                    connect_prev_year[sender] => move |_| {
                        sender.input(CalendarInput::VisibleMonthChanged);
                    },
                    connect_next_year[sender] => move |_| {
                        sender.input(CalendarInput::VisibleMonthChanged);
                    },
                },
            },

            // ── Agenda card ──────────────────────────────────
            gtk::Box {
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
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let id = start_tick(&sender);

        let format_24_h = mshell_config::config_manager::config_manager()
            .config()
            .general()
            .clock_format_24_h()
            .get_untracked();

        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        let today = today_local();

        let model = CalendarModel {
            timer_id: Some(id),
            current_date: now.date(),
            hero_day: now.day().to_string(),
            hero_month: now.format(&MONTH_FORMAT).unwrap_or_default(),
            hero_weekday: now.format(&WEEKDAY_FORMAT).unwrap_or_default(),
            hero_time: format_time(&now, format_24_h),
            format_24_h,
            events: Vec::new(),
            selected: today,
            agenda_heading: heading(today),
        };

        let widgets = view_output!();

        // Prime the grid to today. GTK's months are 0-indexed, `time`'s 1-indexed.
        widgets.calendar.set_year(now.year());
        widgets.calendar.set_month(now.month() as i32 - 1);
        widgets.calendar.set_day(now.day() as i32);

        // Kick the first load.
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
            CalendarCommandOutput::Loaded(events) => {
                sender.input(CalendarInput::EventsLoaded(events));
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
            CalendarInput::UpdateTime => {
                let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());

                self.hero_time = format_time(&now, self.format_24_h);

                if now.date() != self.current_date {
                    self.current_date = now.date();
                    self.hero_day = now.day().to_string();
                    self.hero_month = now.format(&MONTH_FORMAT).unwrap_or_default();
                    self.hero_weekday = now.format(&WEEKDAY_FORMAT).unwrap_or_default();
                    widgets.calendar.set_year(now.year());
                    widgets.calendar.set_month(now.month() as i32 - 1);
                    widgets.calendar.set_day(now.day() as i32);
                }
            }
            CalendarInput::ParentRevealChanged(visible) => {
                if visible {
                    if self.timer_id.is_none() {
                        self.timer_id = Some(start_tick(&sender));
                    }
                    sender.input(CalendarInput::UpdateTime);
                    // Reload so `.ics` files dropped since last open appear.
                    spawn_load(&sender);
                } else if let Some(id) = self.timer_id.take() {
                    id.remove();
                }
            }
            CalendarInput::EventsLoaded(events) => {
                self.events = events;
                refresh_marks(&widgets.calendar, &self.events);
                rebuild_agenda(
                    &widgets.agenda_list,
                    &widgets.agenda_empty,
                    &self.events,
                    self.selected,
                );
            }
            CalendarInput::DaySelected => {
                let date = widgets.calendar.date();
                if let Some(day) = NaiveDate::from_ymd_opt(
                    date.year(),
                    date.month() as u32,
                    date.day_of_month() as u32,
                ) {
                    self.selected = day;
                    self.agenda_heading = heading(day);
                    rebuild_agenda(
                        &widgets.agenda_list,
                        &widgets.agenda_empty,
                        &self.events,
                        self.selected,
                    );
                }
                // Selecting a day may cross into a new visible month.
                refresh_marks(&widgets.calendar, &self.events);
            }
            CalendarInput::VisibleMonthChanged => {
                refresh_marks(&widgets.calendar, &self.events);
            }
        }

        self.update_view(widgets, sender);
    }
}

/// Kick an off-thread load of every configured calendar, delivering the result
/// back as `Loaded`. Blocking IO (files + `ureq`) runs on a blocking task.
fn spawn_load(sender: &ComponentSender<CalendarModel>) {
    let now = chrono::Utc::now();
    let window = (
        now - chrono::Duration::days(LOAD_WINDOW_DAYS),
        now + chrono::Duration::days(LOAD_WINDOW_DAYS),
    );
    sender.oneshot_command(async move {
        let events = tokio::task::spawn_blocking(move || {
            mcal::load_all(&mcal::CalendarConfig::default(), window)
        })
        .await
        .unwrap_or_default();
        CalendarCommandOutput::Loaded(events)
    });
}

/// Re-mark the grid: a dot on every day of the *visible* month that has ≥1 event.
fn refresh_marks(calendar: &gtk::Calendar, events: &[mcal::Event]) {
    calendar.clear_marks();
    let shown = calendar.date();
    for day in mcal::days_with_events(events, shown.year(), shown.month() as u32) {
        calendar.mark_day(day);
    }
}

/// Rebuild the agenda list for `selected`, toggling the empty-state label.
fn rebuild_agenda(
    list: &gtk::Box,
    empty: &gtk::Label,
    events: &[mcal::Event],
    selected: NaiveDate,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let day_events = mcal::events_on_day(events, selected);
    empty.set_visible(day_events.is_empty());
    for event in &day_events {
        list.append(&agenda_row(event));
    }
}

/// One agenda row: a fixed-width time column + title (and location, if any).
fn agenda_row(event: &mcal::Event) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .css_classes(["calendar-agenda-row"])
        .build();

    let time = gtk::Label::builder()
        .css_classes(["calendar-agenda-time", "label-small"])
        .label(time_label(event))
        .xalign(0.0)
        .width_request(64)
        .valign(gtk::Align::Start)
        .build();
    row.append(&time);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();

    let title = gtk::Label::builder()
        .css_classes(["calendar-agenda-title", "label-medium"])
        .label(event.summary.as_str())
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    body.append(&title);

    if let Some(location) = event.location.as_deref().filter(|l| !l.is_empty()) {
        let loc = gtk::Label::builder()
            .css_classes(["calendar-agenda-location", "label-small"])
            .label(location)
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        body.append(&loc);
    }

    row.append(&body);
    row
}

/// A row's time column: "All day" or local "HH:MM".
fn time_label(event: &mcal::Event) -> String {
    if event.all_day {
        "All day".to_string()
    } else {
        event.start.with_timezone(&Local).format("%H:%M").to_string()
    }
}

/// The agenda card heading for a day, e.g. "Friday, July 4".
fn heading(date: NaiveDate) -> String {
    date.format("%A, %B %-d").to_string()
}

/// Today's date in the machine's local time.
fn today_local() -> NaiveDate {
    Local::now().date_naive()
}

/// Start the 1 Hz tick that drives `UpdateTime`.
fn start_tick(sender: &ComponentSender<CalendarModel>) -> SourceId {
    let sender = sender.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        sender.input(CalendarInput::UpdateTime);
        glib::ControlFlow::Continue
    })
}

fn format_time(now: &OffsetDateTime, format_24_h: bool) -> String {
    let fmt = if format_24_h {
        &*TIME_FORMAT_24
    } else {
        &*TIME_FORMAT_12
    };
    now.format(fmt).unwrap_or_default()
}

impl Drop for CalendarModel {
    fn drop(&mut self) {
        if let Some(id) = self.timer_id.take() {
            id.remove();
        }
    }
}
