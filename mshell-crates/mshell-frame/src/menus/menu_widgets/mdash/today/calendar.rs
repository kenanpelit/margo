//! Today: the month calendar card. Its selected day drives the Today card
//! ([`super::today_card`]: that day's events, the week after it, and the
//! unread notifications). Events come from the shared `calendar_data`
//! loader (local `.ics` + subscriptions) the clock menu uses.

use super::super::card::Card;
use super::today_card::{NoteLine, TodayCard, event_lines};
use crate::menus::menu_widgets::calendar_data;
use chrono::{Datelike, Local, NaiveDate};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ObjectExt, WidgetExt};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(crate) struct CalendarCards {
    pub(crate) calendar_card: Card,
    pub(crate) today_card: Rc<TodayCard>,
    calendar: gtk::Calendar,
    events: RefCell<Vec<mcal::Event>>,
    notes: RefCell<(Vec<NoteLine>, Vec<NoteLine>)>,
    selected: Cell<NaiveDate>,
    /// The dashboard's today (frozen in render fixtures).
    today: Cell<NaiveDate>,
}

/// "Tuesday 10 November" for the Today card's title, "Today" for today.
pub(crate) fn agenda_title(day: NaiveDate, today: NaiveDate) -> String {
    if day == today {
        "Today".to_string()
    } else if day.succ_opt() == Some(today) {
        "Yesterday".to_string()
    } else if today.succ_opt() == Some(day) {
        "Tomorrow".to_string()
    } else {
        day.format("%A %-d %B").to_string()
    }
}

/// "2 events" / "1 event" / "" for the Today card's meta.
pub(crate) fn event_count(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => "1 event".to_string(),
        n => format!("{n} events"),
    }
}

impl CalendarCards {
    pub(crate) fn new(today_card: Rc<TodayCard>) -> Rc<Self> {
        let calendar_card = Card::new("x-office-calendar-symbolic", "Calendar");
        calendar_card.root.add_css_class("mdash-calendar-card");
        let calendar = gtk::Calendar::new();
        calendar.set_show_heading(true);
        calendar.set_show_day_names(true);
        calendar.set_hexpand(true);
        calendar.set_focus_on_click(false);
        calendar.add_css_class("mdash-calendar");
        calendar_card.body.append(&calendar);

        let today = Local::now().date_naive();
        let cards = Rc::new(Self {
            calendar_card,
            today_card,
            calendar,
            events: RefCell::new(Vec::new()),
            notes: RefCell::new((Vec::new(), Vec::new())),
            selected: Cell::new(today),
            today: Cell::new(today),
        });

        let weak = Rc::downgrade(&cards);
        cards.calendar.connect_day_selected(move |cal| {
            if let Some(cards) = weak.upgrade() {
                let d = cal.date();
                if let Some(day) =
                    NaiveDate::from_ymd_opt(d.year(), d.month() as u32, d.day_of_month() as u32)
                {
                    cards.selected.set(day);
                    cards.refresh();
                }
            }
        });
        for signal in ["prev-month", "next-month", "prev-year", "next-year"] {
            let weak = Rc::downgrade(&cards);
            cards.calendar.connect_local(signal, false, move |_| {
                if let Some(cards) = weak.upgrade() {
                    calendar_data::refresh_marks(&cards.calendar, &cards.events.borrow());
                }
                None
            });
        }
        cards.refresh();
        cards
    }

    /// Show `day` (and its month).
    pub(crate) fn select(&self, day: NaiveDate) {
        self.calendar.set_year(day.year());
        self.calendar.set_month(day.month0() as i32);
        self.calendar.set_day(day.day() as i32);
        self.selected.set(day);
        self.refresh();
    }

    /// Jump back to today if the day rolled over since the last look.
    pub(crate) fn follow_today(&self, today: NaiveDate) {
        self.today.set(today);
        if self.selected.get() != today && self.selected.get() < today {
            self.select(today);
        }
    }

    pub(crate) fn set_events(&self, events: Vec<mcal::Event>) {
        *self.events.borrow_mut() = events;
        self.refresh();
    }

    /// The unread notifications and the read ones before them, newest
    /// first (no-op when unchanged).
    pub(crate) fn set_notes(&self, unread: Vec<NoteLine>, earlier: Vec<NoteLine>) {
        let notes = (unread, earlier);
        if *self.notes.borrow() != notes {
            *self.notes.borrow_mut() = notes;
            self.refresh();
        }
    }

    fn refresh(&self) {
        let events = self.events.borrow();
        calendar_data::refresh_marks(&self.calendar, &events);
        let day = self.selected.get();
        let today = self.today.get();
        let n = mcal::events_on_day(&events, day).len();
        self.today_card.set_content(
            &agenda_title(day, today),
            &event_count(n),
            &event_lines(&events, day, today),
            &self.notes.borrow().0,
            &self.notes.borrow().1,
        );
        self.calendar_card
            .set_meta(&format!("Week {}", day.iso_week().week()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agenda_title_names_nearby_days() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 10).expect("date");
        assert_eq!(agenda_title(today, today), "Today");
        assert_eq!(
            agenda_title(today.succ_opt().expect("next"), today),
            "Tomorrow"
        );
        assert_eq!(
            agenda_title(today.pred_opt().expect("prev"), today),
            "Yesterday"
        );
        assert_eq!(
            agenda_title(NaiveDate::from_ymd_opt(2026, 11, 20).expect("date"), today),
            "Friday 20 November"
        );
    }

    #[test]
    fn event_count_is_quiet_when_empty() {
        assert_eq!(event_count(0), "");
        assert_eq!(event_count(1), "1 event");
        assert_eq!(event_count(3), "3 events");
    }
}
