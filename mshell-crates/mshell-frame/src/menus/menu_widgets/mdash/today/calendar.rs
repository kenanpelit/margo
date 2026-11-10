//! Today: the month calendar card and, beside it, the selected day's
//! agenda card. Events come from the shared `calendar_data` loader (local
//! `.ics` + subscriptions) the clock menu and the old mdash grid use.

use super::super::card::{Card, empty_label};
use crate::menus::menu_widgets::calendar_data;
use chrono::{Datelike, Local, NaiveDate};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ObjectExt, WidgetExt};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The agenda list's floor (about two event rows): the agenda never asks
/// for more, so it can't make the right column outgrow the left.
const AGENDA_MIN_HEIGHT: i32 = 56;

pub(crate) struct CalendarCards {
    pub(crate) calendar_card: Card,
    pub(crate) agenda_card: Card,
    calendar: gtk::Calendar,
    agenda_list: gtk::Box,
    agenda_scroller: gtk::ScrolledWindow,
    agenda_empty: gtk::Label,
    events: RefCell<Vec<mcal::Event>>,
    selected: Cell<NaiveDate>,
}

/// "Tuesday 10 November" for the agenda title, "Today" for today.
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

/// "2 events" / "1 event" / "" for the agenda card's meta.
pub(crate) fn event_count(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => "1 event".to_string(),
        n => format!("{n} events"),
    }
}

impl CalendarCards {
    pub(crate) fn new() -> Rc<Self> {
        let calendar_card = Card::new("x-office-calendar-symbolic", "Calendar");
        calendar_card.root.add_css_class("mdash-calendar-card");
        let calendar = gtk::Calendar::new();
        calendar.set_show_heading(true);
        calendar.set_show_day_names(true);
        calendar.set_hexpand(true);
        calendar.set_focus_on_click(false);
        calendar.add_css_class("mdash-calendar");
        calendar_card.body.append(&calendar);

        let agenda_card = Card::new("view-list-symbolic", "Today");
        agenda_card.root.add_css_class("mdash-agenda-card");
        let agenda_list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        agenda_list.add_css_class("calendar-agenda-list");
        // The agenda is Today's flexible card: its list asks for no height
        // of its own (beyond a two-row floor) and takes what the left
        // column leaves, scrolling when a busy day overflows it.
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_propagate_natural_height(false);
        scroller.set_min_content_height(AGENDA_MIN_HEIGHT);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&agenda_list));
        agenda_card.body.set_vexpand(true);
        agenda_card.body.append(&scroller);
        let agenda_empty = empty_label("No events");
        agenda_empty.set_vexpand(true);
        // The same floor as the list: an empty day is as tall as a quiet one.
        agenda_empty.set_size_request(-1, AGENDA_MIN_HEIGHT);
        agenda_card.body.append(&agenda_empty);

        let today = Local::now().date_naive();
        let cards = Rc::new(Self {
            calendar_card,
            agenda_card,
            calendar,
            agenda_list,
            agenda_scroller: scroller,
            agenda_empty,
            events: RefCell::new(Vec::new()),
            selected: Cell::new(today),
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
        if self.selected.get() != today && self.selected.get() < today {
            self.select(today);
        }
    }

    pub(crate) fn set_events(&self, events: Vec<mcal::Event>) {
        *self.events.borrow_mut() = events;
        self.refresh();
    }

    fn refresh(&self) {
        let events = self.events.borrow();
        calendar_data::refresh_marks(&self.calendar, &events);
        let day = self.selected.get();
        calendar_data::rebuild_agenda(&self.agenda_list, &self.agenda_empty, &events, day);
        let n = mcal::events_on_day(&events, day).len();
        self.agenda_scroller.set_visible(n > 0);
        self.agenda_card
            .set_title(&agenda_title(day, Local::now().date_naive()));
        self.agenda_card.set_meta(&event_count(n));
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
