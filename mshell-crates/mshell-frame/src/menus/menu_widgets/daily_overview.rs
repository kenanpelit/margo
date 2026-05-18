//! Dashboard "Daily Overview" tile — single big card that hosts
//! the calendar + weather + intel signals on one shared surface.
//!
//! Replaces the previous Container([OverviewIntel, CalendarGrid,
//! Weather]) stack in the left column with one widget that owns
//! all three controllers and a wrapping surface. SCSS strips the
//! individual chromes of the inner widgets so they read as
//! merged sections of a single tile, not three stacked cards.
//!
//! The intel bullets live at the BOTTOM (matching the user's
//! mockup) — calendar acts as the visual anchor, weather sits
//! underneath, intel finishes off the card.

use crate::menus::menu_widgets::calendar_grid::{CalendarGridInit, CalendarGridModel};
use crate::menus::menu_widgets::overview_intel::{OverviewIntelInit, OverviewIntelModel};
use crate::menus::menu_widgets::weather::weather::{WeatherInit, WeatherModel};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

pub(crate) struct DailyOverviewModel {
    _calendar: Controller<CalendarGridModel>,
    _weather: Controller<WeatherModel>,
    _intel: Controller<OverviewIntelModel>,
}

#[derive(Debug)]
pub(crate) enum DailyOverviewInput {}

#[derive(Debug)]
pub(crate) enum DailyOverviewOutput {}

pub(crate) struct DailyOverviewInit {}

#[relm4::component(pub)]
impl Component for DailyOverviewModel {
    type CommandOutput = ();
    type Input = DailyOverviewInput;
    type Output = DailyOverviewOutput;
    type Init = DailyOverviewInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "daily-overview-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            set_spacing: 10,

            // Calendar grid sits on top — the visual anchor.
            #[name = "calendar_slot"]
            gtk::Box {
                add_css_class: "daily-overview-section",
                set_orientation: gtk::Orientation::Vertical,
            },

            // Faint divider between sections so they read as
            // segments of one card, not three glued tiles.
            gtk::Separator {
                add_css_class: "daily-overview-divider",
                set_orientation: gtk::Orientation::Horizontal,
            },

            // Weather underneath the calendar.
            #[name = "weather_slot"]
            gtk::Box {
                add_css_class: "daily-overview-section",
                set_orientation: gtk::Orientation::Vertical,
            },

            gtk::Separator {
                add_css_class: "daily-overview-divider",
                set_orientation: gtk::Orientation::Horizontal,
            },

            // Intel bullets at the bottom — matches the user's
            // mockup where the day summary closes the card.
            #[name = "intel_slot"]
            gtk::Box {
                add_css_class: "daily-overview-section",
                set_orientation: gtk::Orientation::Vertical,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let calendar = CalendarGridModel::builder()
            .launch(CalendarGridInit {})
            .detach();
        let weather = WeatherModel::builder().launch(WeatherInit {}).detach();
        let intel = OverviewIntelModel::builder()
            .launch(OverviewIntelInit {})
            .detach();

        let model = DailyOverviewModel {
            _calendar: calendar,
            _weather: weather,
            _intel: intel,
        };

        let widgets = view_output!();

        widgets
            .calendar_slot
            .append(model._calendar.widget());
        widgets.weather_slot.append(model._weather.widget());
        widgets.intel_slot.append(model._intel.widget());

        let _ = root;
        ComponentParts { model, widgets }
    }
}
