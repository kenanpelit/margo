//! The **Today** tab: calendar + agenda, weather, media and timers, laid
//! out by [`super::layout::today`] so both columns end level.

pub(crate) mod calendar;
pub(crate) mod media;
pub(crate) mod timers;
pub(crate) mod weather;

use super::layout::{self, Slot};
use calendar::CalendarCards;
use media::{MediaCard, MediaEvent};
use mshell_config::schema::dashboard::{DashboardCard, DashboardTabConfig};
use relm4::gtk;
use relm4::gtk::prelude::{GridExt, WidgetExt};
use std::rc::Rc;
use timers::TimersCard;
use weather::WeatherCard;

pub(crate) struct TodayTab {
    pub(crate) root: gtk::Grid,
    pub(crate) calendar: Rc<CalendarCards>,
    pub(crate) weather: WeatherCard,
    pub(crate) media: MediaCard,
    pub(crate) timers: TimersCard,
}

impl TodayTab {
    pub(crate) fn new(cfg: &DashboardTabConfig, on_media: impl Fn(MediaEvent) + 'static) -> Self {
        let root = gtk::Grid::new();
        root.add_css_class("mdash-tab-grid");
        root.set_column_homogeneous(true);
        root.set_row_spacing(10);
        root.set_column_spacing(10);
        root.set_vexpand(true);

        let calendar = CalendarCards::new();
        let weather = WeatherCard::new();
        let media = MediaCard::new(on_media);
        let timers = TimersCard::new();

        for Slot {
            card,
            col,
            row,
            width,
            height,
        } in layout::today(|c| cfg.shows(c))
        {
            let widget = match card {
                DashboardCard::Calendar => &calendar.calendar_card.root,
                DashboardCard::Agenda => &calendar.agenda_card.root,
                DashboardCard::Weather => &weather.card.root,
                DashboardCard::Media => &media.card.root,
                DashboardCard::Timers => &timers.card.root,
                _ => continue,
            };
            root.attach(widget, col, row, width, height);
        }
        // The flexible cards: the agenda list and the timers list take any
        // height the tab has to spare, never the calendar or weather.
        calendar.agenda_card.root.set_vexpand(true);
        timers.card.root.set_vexpand(true);
        for fixed in [
            &calendar.calendar_card.root,
            &weather.card.root,
            &media.card.root,
        ] {
            fixed.set_vexpand(false);
        }

        Self {
            root,
            calendar,
            weather,
            media,
            timers,
        }
    }
}
