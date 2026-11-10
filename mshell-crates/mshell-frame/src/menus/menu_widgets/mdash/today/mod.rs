//! The **Today** tab: calendar + agenda, weather, media and timers, laid
//! out in two columns by [`super::layout::today`] so both end level.

pub(crate) mod calendar;
pub(crate) mod media;
pub(crate) mod timers;
pub(crate) mod weather;

use super::card::GAP;
use super::layout;
use calendar::CalendarCards;
use media::{MediaCard, MediaEvent};
use mshell_config::schema::dashboard::{DashboardCard, DashboardTabConfig};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use std::rc::Rc;
use timers::TimersCard;
use weather::WeatherCard;

pub(crate) struct TodayTab {
    pub(crate) root: gtk::Box,
    pub(crate) calendar: Rc<CalendarCards>,
    pub(crate) weather: WeatherCard,
    pub(crate) media: MediaCard,
    pub(crate) timers: TimersCard,
}

impl TodayTab {
    pub(crate) fn new(cfg: &DashboardTabConfig, on_media: impl Fn(MediaEvent) + 'static) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
        root.add_css_class("mdash-tab-columns");
        root.set_homogeneous(true);

        let calendar = CalendarCards::new();
        let weather = WeatherCard::new();
        let media = MediaCard::new(on_media);
        let timers = TimersCard::new();

        for cards in layout::today(|c| cfg.shows(c)) {
            let column = gtk::Box::new(gtk::Orientation::Vertical, GAP);
            for card in &cards {
                let widget = match card {
                    DashboardCard::Calendar => &calendar.calendar_card.root,
                    DashboardCard::Agenda => &calendar.agenda_card.root,
                    DashboardCard::Weather => &weather.card.root,
                    DashboardCard::Media => &media.card.root,
                    DashboardCard::Timers => &timers.card.root,
                    _ => continue,
                };
                // The agenda is the one flexible card: it takes the
                // height the left column has to spare, so both columns end
                // level. Every other card keeps its natural height.
                widget.set_vexpand(*card == DashboardCard::Agenda);
                column.append(widget);
            }
            root.append(&column);
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
