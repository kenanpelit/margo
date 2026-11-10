//! The **Today** tab: calendar + weather on the left; the Today card
//! (events + unread notifications), media and timers on the right — laid
//! out in two columns by [`super::layout::today`] so both end level.

pub(crate) mod calendar;
pub(crate) mod fit;
pub(crate) mod media;
pub(crate) mod timers;
pub(crate) mod today_card;
pub(crate) mod weather;

use super::card::GAP;
use super::layout;
use calendar::CalendarCards;
use media::{MediaCard, MediaEvent, MediaView};
use mshell_config::schema::dashboard::{DashboardCard, DashboardTabConfig};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use std::rc::Rc;
use timers::TimersCard;
use today_card::{Spill, TodayCard, TodayEvent};
use weather::WeatherCard;

pub(crate) struct TodayTab {
    pub(crate) root: gtk::Box,
    pub(crate) calendar: Rc<CalendarCards>,
    pub(crate) weather: WeatherCard,
    pub(crate) media: Rc<MediaCard>,
    pub(crate) timers: TimersCard,
}

impl TodayTab {
    pub(crate) fn new(
        cfg: &DashboardTabConfig,
        on_media: impl Fn(MediaEvent) + 'static,
        on_today: impl Fn(TodayEvent) + 'static,
    ) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
        root.add_css_class("mdash-tab-columns");
        root.set_homogeneous(true);

        let today_card = TodayCard::new(on_today);
        let calendar = CalendarCards::new(today_card.clone());
        let weather = WeatherCard::new();
        let media = Rc::new(MediaCard::new(on_media));
        let timers = TimersCard::new();
        // The idle media card takes the Today card's whole-row leftover
        // (both sit in the right column when both are shown).
        if cfg.shows(DashboardCard::Agenda) && cfg.shows(DashboardCard::Media) {
            let spill: Rc<dyn Spill> = media.clone();
            today_card.set_spill(Rc::downgrade(&spill));
        }

        for cards in layout::today(|c| cfg.shows(c)) {
            let column = gtk::Box::new(gtk::Orientation::Vertical, GAP);
            for card in &cards {
                let widget = match card {
                    DashboardCard::Calendar => &calendar.calendar_card.root,
                    DashboardCard::Agenda => &today_card.card.root,
                    DashboardCard::Weather => &weather.card.root,
                    DashboardCard::Media => &media.card.root,
                    DashboardCard::Timers => &timers.card.root,
                    _ => continue,
                };
                // The Today card is the one flexible card: it takes the
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

    /// Show `view` on the media card; the Today card refits (an idle card
    /// can take leftover rows, a playing one can't).
    pub(crate) fn apply_media(&self, view: &MediaView) {
        self.media.apply(view);
        self.calendar.today_card.queue_fit();
    }
}
