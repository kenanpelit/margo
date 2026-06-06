//! One daily-forecast cell. A plain `gtk::Box` builder — **not** a relm4
//! component (same reasoning as `hourly_item.rs`: a component-per-cell with a
//! reactive config watcher each was hundreds of launches on the GTK main
//! thread). The parent (`daily.rs`) owns the temperature-unit watch.

use mshell_utils::weather::{get_percent_string, get_temperature_string, get_weather_icon_name};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use wayle_weather::{DailyForecast, TemperatureUnit};

/// Build a single daily cell (day · icon · high · low · rain%) as a plain
/// widget tree. No component, no reactive effects.
pub(crate) fn build_daily_item(
    daily: &DailyForecast,
    temperature_unit: &TemperatureUnit,
) -> gtk::Box {
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();

    let day = gtk::Label::new(Some(&daily.date.format("%a").to_string()));
    day.add_css_class("label-small-bold");
    root.append(&day);

    let icon = gtk::Image::new();
    icon.add_css_class("hourly-weather-icon");
    icon.set_icon_name(Some(get_weather_icon_name(&daily.condition, true)));
    root.append(&icon);

    let high = gtk::Label::new(Some(
        get_temperature_string(&daily.temp_high, temperature_unit).as_str(),
    ));
    high.add_css_class("label-small-bold");
    root.append(&high);

    let low = gtk::Label::new(Some(
        get_temperature_string(&daily.temp_low, temperature_unit).as_str(),
    ));
    low.add_css_class("label-small-bold");
    root.append(&low);

    let rain = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .halign(gtk::Align::Center)
        .spacing(2)
        .build();
    rain.add_css_class("daily-rain");
    let rain_icon = gtk::Image::new();
    rain_icon.add_css_class("daily-rain-icon");
    rain_icon.set_icon_name(Some("weather-showers-scattered-symbolic"));
    rain.append(&rain_icon);
    let rain_label = gtk::Label::new(Some(get_percent_string(&daily.rain_chance).as_str()));
    rain_label.add_css_class("label-small");
    rain.append(&rain_label);
    root.append(&rain);

    root
}
