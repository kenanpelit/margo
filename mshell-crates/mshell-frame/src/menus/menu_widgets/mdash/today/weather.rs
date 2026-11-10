//! Today: the weather card — now (big glyph + temperature + condition),
//! the next hours, and a three-day outlook, all from the weather service's
//! last good reading (in memory, else the on-disk cache).

use super::super::card::{Card, empty_label};
use chrono::{NaiveDate, NaiveDateTime, Timelike};
use mshell_utils::weather::{condition_label, get_weather_icon_name};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, GridExt, WidgetExt};
use wayle_weather::{DailyForecast, HourlyForecast, Temperature, TemperatureUnit, Weather};

/// Hours in the strip.
pub(crate) const HOURS: usize = 6;
/// Days in the outlook.
pub(crate) const DAYS: usize = 3;

/// "11°" (whole degrees; the unit is in the header line).
pub(crate) fn degrees(t: &Temperature, unit: &TemperatureUnit) -> String {
    let v = match unit {
        TemperatureUnit::Metric => t.celsius(),
        TemperatureUnit::Imperial => t.fahrenheit(),
    };
    format!("{}°", v.round() as i64)
}

/// The next `n` hourly forecasts from the hour `now` is in.
pub(crate) fn next_hours(
    hourly: &[HourlyForecast],
    now: NaiveDateTime,
    n: usize,
) -> Vec<&HourlyForecast> {
    let hour_start = now
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .unwrap_or(now);
    hourly
        .iter()
        .filter(|h| h.time >= hour_start)
        .take(n)
        .collect()
}

/// The `n` days after `today`.
pub(crate) fn outlook_days(
    daily: &[DailyForecast],
    today: NaiveDate,
    n: usize,
) -> Vec<&DailyForecast> {
    daily.iter().filter(|d| d.date > today).take(n).collect()
}

pub(crate) struct WeatherCard {
    pub(crate) card: Card,
    content: gtk::Box,
    empty: gtk::Label,
    now_icon: gtk::Image,
    now_temp: gtk::Label,
    now_condition: gtk::Label,
    now_detail: gtk::Label,
    hours: gtk::Box,
    days: gtk::Grid,
}

impl WeatherCard {
    pub(crate) fn new() -> Self {
        let card = Card::new("weather-few-clouds-symbolic", "Weather");
        card.root.add_css_class("mdash-weather-card");

        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        content.set_vexpand(true);

        let now = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        now.add_css_class("mdash-weather-now");
        let now_icon = gtk::Image::new();
        now_icon.add_css_class("mdash-weather-icon");
        now.append(&now_icon);
        let now_temp = gtk::Label::new(None);
        now_temp.add_css_class("mdash-weather-temp");
        now.append(&now_temp);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let now_condition = gtk::Label::new(None);
        now_condition.add_css_class("mdash-weather-condition");
        now_condition.set_xalign(0.0);
        now_condition.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&now_condition);
        let now_detail = gtk::Label::new(None);
        now_detail.add_css_class("mdash-dim");
        now_detail.set_xalign(0.0);
        now_detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&now_detail);
        now.append(&text);
        content.append(&now);

        let hours = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hours.set_homogeneous(true);
        hours.add_css_class("mdash-weather-hours");
        content.append(&hours);

        let days = gtk::Grid::new();
        days.add_css_class("mdash-weather-days");
        days.set_row_spacing(4);
        days.set_column_spacing(10);
        days.set_vexpand(true);
        days.set_valign(gtk::Align::End);
        content.append(&days);

        card.body.append(&content);
        let empty = empty_label("Weather unavailable — set a location in Settings → Weather");
        empty.set_visible(false);
        card.body.append(&empty);
        Self {
            card,
            content,
            empty,
            now_icon,
            now_temp,
            now_condition,
            now_detail,
            hours,
            days,
        }
    }

    /// Paint `weather` (`None`: the calm "unavailable" state).
    pub(crate) fn apply(
        &self,
        weather: Option<&Weather>,
        unit: &TemperatureUnit,
        h24: bool,
        now: NaiveDateTime,
    ) {
        let Some(w) = weather else {
            self.content.set_visible(false);
            self.empty.set_visible(true);
            self.card.set_meta("");
            return;
        };
        self.content.set_visible(true);
        self.empty.set_visible(false);
        self.card.set_meta(&w.location.city);

        let c = &w.current;
        self.now_icon
            .set_icon_name(Some(get_weather_icon_name(&c.condition, c.is_day)));
        self.now_temp.set_label(&degrees(&c.temperature, unit));
        self.now_condition
            .set_label(condition_label(&c.condition, c.is_day));
        let today = w.daily.iter().find(|d| d.date == now.date());
        let mut detail = format!("Feels {}", degrees(&c.feels_like, unit));
        if let Some(d) = today {
            detail.push_str(&format!(
                " · H {} L {}",
                degrees(&d.temp_high, unit),
                degrees(&d.temp_low, unit)
            ));
        }
        self.now_detail.set_label(&detail);

        while let Some(child) = self.hours.first_child() {
            self.hours.remove(&child);
        }
        for h in next_hours(&w.hourly, now, HOURS) {
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
            cell.add_css_class("mdash-weather-hour");
            let time = gtk::Label::new(Some(&if h24 {
                h.time.format("%H:%M").to_string()
            } else {
                h.time.format("%-I %p").to_string()
            }));
            time.add_css_class("mdash-dim");
            cell.append(&time);
            let icon = gtk::Image::from_icon_name(get_weather_icon_name(&h.condition, h.is_day));
            cell.append(&icon);
            let temp = gtk::Label::new(Some(&degrees(&h.temperature, unit)));
            temp.add_css_class("mdash-weather-hour-temp");
            cell.append(&temp);
            self.hours.append(&cell);
        }

        while let Some(child) = self.days.first_child() {
            self.days.remove(&child);
        }
        for (row, d) in outlook_days(&w.daily, now.date(), DAYS)
            .into_iter()
            .enumerate()
        {
            let row = row as i32;
            let name = gtk::Label::new(Some(&d.date.format("%a").to_string()));
            name.add_css_class("mdash-weather-day");
            name.set_xalign(0.0);
            self.days.attach(&name, 0, row, 1, 1);
            let icon = gtk::Image::from_icon_name(get_weather_icon_name(&d.condition, true));
            self.days.attach(&icon, 1, row, 1, 1);
            let cond = gtk::Label::new(Some(condition_label(&d.condition, true)));
            cond.add_css_class("mdash-dim");
            cond.set_xalign(0.0);
            cond.set_hexpand(true);
            cond.set_ellipsize(gtk::pango::EllipsizeMode::End);
            self.days.attach(&cond, 2, row, 1, 1);
            let range = gtk::Label::new(Some(&format!(
                "{} / {}",
                degrees(&d.temp_high, unit),
                degrees(&d.temp_low, unit)
            )));
            range.add_css_class("mdash-weather-range");
            range.set_xalign(1.0);
            self.days.attach(&range, 3, row, 1, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Weather {
        serde_json::from_str(include_str!("../testdata/weather.json")).expect("fixture parses")
    }

    #[test]
    fn the_strip_starts_at_the_current_hour() {
        let w = fixture();
        let now = NaiveDate::from_ymd_opt(2026, 11, 10)
            .and_then(|d| d.and_hms_opt(14, 37, 0))
            .expect("time");
        let hours = next_hours(&w.hourly, now, HOURS);
        assert_eq!(hours.len(), HOURS);
        assert_eq!(hours[0].time.hour(), 14);
        assert!(hours.windows(2).all(|p| p[0].time < p[1].time));
    }

    #[test]
    fn the_outlook_is_the_next_three_days() {
        let w = fixture();
        let today = NaiveDate::from_ymd_opt(2026, 11, 10).expect("date");
        let days = outlook_days(&w.daily, today, DAYS);
        assert_eq!(days.len(), DAYS);
        assert_eq!(days[0].date, today.succ_opt().expect("next"));
    }

    #[test]
    fn degrees_round_and_follow_the_unit() {
        let t = Temperature::new(10.6).expect("valid");
        assert_eq!(degrees(&t, &TemperatureUnit::Metric), "11°");
        assert_eq!(degrees(&t, &TemperatureUnit::Imperial), "51°");
    }
}
