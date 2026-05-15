//! Clock-menu calendar widget — noctalia-style two-card stack.
//!
//! Top card ("hero"): primary-coloured rectangle showing the big
//! day number, month + year, weekday, and live clock readout.
//! Functions as the panel header — instantly tells the user the
//! date at a glance.
//!
//! Bottom card: month grid (built-in `gtk::Calendar` styled via
//! CSS to match the surface palette). Prev / next month nav and
//! today highlighting come from GTK; we just feed it the current
//! date once and let it manage its own state.
//!
//! Updates: a 1 Hz glib timer refreshes the clock readout. On
//! the rare day rollover (midnight) the hero day number + the
//! calendar selection both update; otherwise only the time
//! label re-renders so the calendar's internal navigation state
//! isn't disturbed.

use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use reactive_graph::traits::GetUntracked;
use relm4::{
    Component, ComponentParts, ComponentSender,
    gtk::{
        self,
        glib::{self, SourceId},
        prelude::*,
    },
    once_cell,
};
use time::format_description::parse;
use time::{Date, OffsetDateTime};

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
    once_cell::sync::Lazy::new(|| {
        parse("[weekday repr:long]").unwrap()
    });

#[derive(Debug)]
pub(crate) struct CalendarModel {
    timer_id: Option<SourceId>,
    current_date: Date,
    hero_day: String,
    hero_month: String,
    hero_weekday: String,
    hero_time: String,
    format_24_h: bool,
}

#[derive(Debug)]
pub(crate) enum CalendarInput {
    UpdateTime,
}

#[derive(Debug)]
pub(crate) enum CalendarOutput {}

pub(crate) struct CalendarInit {}

#[relm4::component(pub)]
impl Component for CalendarModel {
    type Input = CalendarInput;
    type Output = CalendarOutput;
    type Init = CalendarInit;
    type CommandOutput = ();

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

                #[name = "calendar"]
                gtk::Calendar {
                    set_can_focus: false,
                    set_focus_on_click: false,
                    set_show_heading: true,
                    set_show_day_names: true,
                    set_hexpand: true,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // 1 Hz tick — fine-grained enough that the time label
        // never lags more than a second. The hero day + grid
        // selection only update on day rollover.
        let sender_clone = sender.clone();
        let id = glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            sender_clone.input(CalendarInput::UpdateTime);
            glib::ControlFlow::Continue
        });

        let format_24_h = mshell_config::config_manager::config_manager()
            .config()
            .general()
            .clock_format_24_h()
            .get_untracked();

        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());

        let model = CalendarModel {
            timer_id: Some(id),
            current_date: now.date(),
            hero_day: now.day().to_string(),
            hero_month: now.format(&MONTH_FORMAT).unwrap_or_default(),
            hero_weekday: now.format(&WEEKDAY_FORMAT).unwrap_or_default(),
            hero_time: format_time(&now, format_24_h),
            format_24_h,
        };

        let widgets = view_output!();

        // Prime the grid to today. GTK's months are 0-indexed,
        // `time`'s are 1-indexed.
        widgets.calendar.set_year(now.year());
        widgets.calendar.set_month(now.month() as i32 - 1);
        widgets.calendar.set_day(now.day() as i32);

        ComponentParts { model, widgets }
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
                    // Move the grid's selected day to the new today.
                    // The user may have navigated to another month —
                    // jumping back is the noctalia behaviour and the
                    // less surprising default.
                    widgets.calendar.set_year(now.year());
                    widgets.calendar.set_month(now.month() as i32 - 1);
                    widgets.calendar.set_day(now.day() as i32);
                }
            }
        }

        self.update_view(widgets, sender);
    }
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
