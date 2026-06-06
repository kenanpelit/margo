//! One hourly-forecast cell. A plain `gtk::Box` builder — **not** a relm4
//! component. Each cell used to be its own `Component` that registered two
//! reactive `EffectScope` config watchers (temperature unit + clock format);
//! with a full forecast (dozens of hours) × every output that was hundreds of
//! component launches + reactive subscriptions on the GTK main thread at
//! startup, burning ~15 s at 100 % CPU before the bar could paint. The parent
//! (`hourly.rs`) now owns the unit/format watch centrally and rebuilds these
//! cheap widgets on change.

use mshell_utils::weather::{get_percent_string, get_temperature_string, get_weather_icon_name};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use wayle_weather::{HourlyForecast, TemperatureUnit};

/// Build a single hourly cell (time · icon · temp · UV · rain%) as a plain
/// widget tree. No component, no reactive effects.
pub(crate) fn build_hourly_item(
    hourly: &HourlyForecast,
    temperature_unit: &TemperatureUnit,
    format_24_h: bool,
) -> gtk::Box {
    let time_label = if format_24_h {
        hourly.time.format("%H").to_string()
    } else {
        hourly.time.format("%I %p").to_string()
    };

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();

    let time = gtk::Label::new(Some(&time_label));
    time.add_css_class("label-small-bold");
    root.append(&time);

    let icon = gtk::Image::new();
    icon.add_css_class("hourly-weather-icon");
    icon.set_icon_name(Some(get_weather_icon_name(
        &hourly.condition,
        hourly.is_day,
    )));
    root.append(&icon);

    let temp = gtk::Label::new(Some(
        get_temperature_string(&hourly.temperature, temperature_unit).as_str(),
    ));
    temp.add_css_class("label-small-bold");
    root.append(&temp);

    let uv = gtk::Label::new(Some(&format!("{} UV", hourly.uv_index)));
    uv.add_css_class("label-small-bold");
    root.append(&uv);

    // Rain chance.
    let rain = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .halign(gtk::Align::Center)
        .spacing(2)
        .build();
    rain.add_css_class("hourly-rain");
    let rain_icon = gtk::Image::new();
    rain_icon.add_css_class("hourly-rain-icon");
    rain_icon.set_icon_name(Some("weather-showers-scattered-symbolic"));
    rain.append(&rain_icon);
    let rain_label = gtk::Label::new(Some(get_percent_string(&hourly.rain_chance).as_str()));
    rain_label.add_css_class("label-small");
    rain.append(&rain_label);
    root.append(&rain);

    root
}
