use mshell_common::watch;
use mshell_services::weather_service;
use relm4::{Component, ComponentSender};
use wayle_weather::{
    Distance, Percentage, Precipitation, Pressure, Speed, Temperature, TemperatureUnit,
    WeatherCondition, WindDirection,
};

pub fn get_weather_icon_name(weather_condition: &WeatherCondition, is_day: bool) -> &'static str {
    match weather_condition {
        WeatherCondition::Clear => {
            if is_day {
                "weather-clear-day-symbolic"
            } else {
                "weather-clear-night-symbolic"
            }
        }
        WeatherCondition::PartlyCloudy => {
            if is_day {
                "weather-partly-cloudy-day-symbolic"
            } else {
                "weather-partly-cloudy-night-symbolic"
            }
        }
        WeatherCondition::Cloudy => "weather-cloudy-symbolic",
        WeatherCondition::Overcast => "weather-overcast-symbolic",
        WeatherCondition::Mist => "weather-mist-symbolic",
        WeatherCondition::Fog => "weather-fog-symbolic",
        WeatherCondition::LightRain => "weather-rain-light-symbolic",
        WeatherCondition::Rain => "weather-rain-symbolic",
        WeatherCondition::HeavyRain => "weather-rain-heavy-symbolic",
        WeatherCondition::Drizzle => "weather-drizzle-symbolic",
        WeatherCondition::LightSnow => "weather-snow-light-symbolic",
        WeatherCondition::Snow => "weather-snow-symbolic",
        WeatherCondition::HeavySnow => "weather-snow-heavy-symbolic",
        WeatherCondition::Sleet => "weather-sleet-symbolic",
        WeatherCondition::Thunderstorm => "weather-thunderstorm-symbolic",
        WeatherCondition::Windy => "weather-windy-symbolic",
        WeatherCondition::Hail => "weather-hail-symbolic",
        WeatherCondition::Unknown => "weather-unknown-symbolic",
    }
}

pub fn get_temperature_string(
    temperature: &Temperature,
    temperature_unit: &TemperatureUnit,
) -> String {
    match temperature_unit {
        TemperatureUnit::Metric => {
            format!("{}°C", temperature.celsius())
        }
        TemperatureUnit::Imperial => {
            format!("{}°F", temperature.fahrenheit())
        }
    }
}

pub fn get_wind_speed(wind_speed: &Speed, temperature_unit: &TemperatureUnit) -> String {
    match temperature_unit {
        TemperatureUnit::Metric => wind_speed.kmh().round().to_string(),
        TemperatureUnit::Imperial => wind_speed.mph().round().to_string(),
    }
}

pub fn get_wind_speed_units_string(temperature_unit: &TemperatureUnit) -> &'static str {
    match temperature_unit {
        TemperatureUnit::Metric => " kmh winds",
        TemperatureUnit::Imperial => " mph winds",
    }
}

/// Bare wind-speed unit suffix (no "winds" suffix) for the compact
/// detail grid — e.g. "km/h" / "mph".
pub fn wind_speed_unit_short(temperature_unit: &TemperatureUnit) -> &'static str {
    match temperature_unit {
        TemperatureUnit::Metric => "km/h",
        TemperatureUnit::Imperial => "mph",
    }
}

/// Human-readable condition summary — the worded description GNOME's
/// OpenWeather shows next to the icon (margo previously rendered the
/// icon only). `is_day` only affects the clear-sky wording.
pub fn condition_label(condition: &WeatherCondition, is_day: bool) -> &'static str {
    match condition {
        WeatherCondition::Clear => {
            if is_day {
                "Clear"
            } else {
                "Clear night"
            }
        }
        WeatherCondition::PartlyCloudy => "Partly cloudy",
        WeatherCondition::Cloudy => "Cloudy",
        WeatherCondition::Overcast => "Overcast",
        WeatherCondition::Mist => "Mist",
        WeatherCondition::Fog => "Fog",
        WeatherCondition::LightRain => "Light rain",
        WeatherCondition::Rain => "Rain",
        WeatherCondition::HeavyRain => "Heavy rain",
        WeatherCondition::Drizzle => "Drizzle",
        WeatherCondition::LightSnow => "Light snow",
        WeatherCondition::Snow => "Snow",
        WeatherCondition::HeavySnow => "Heavy snow",
        WeatherCondition::Sleet => "Sleet",
        WeatherCondition::Thunderstorm => "Thunderstorm",
        WeatherCondition::Windy => "Windy",
        WeatherCondition::Hail => "Hail",
        WeatherCondition::Unknown => "—",
    }
}

/// Pressure formatted with the unit appropriate to the temperature
/// unit (metric → hPa, imperial → inHg). hPa is integer-rounded; inHg
/// keeps two decimals (its useful range is ~28–31).
pub fn get_pressure_string(pressure: &Pressure, temperature_unit: &TemperatureUnit) -> String {
    match temperature_unit {
        TemperatureUnit::Metric => format!("{} hPa", pressure.hpa().round() as i32),
        TemperatureUnit::Imperial => format!("{:.2} inHg", pressure.inhg()),
    }
}

/// Visibility formatted in km (metric) or miles (imperial), one
/// decimal so "9.7 km" reads naturally.
pub fn get_visibility_string(visibility: &Distance, temperature_unit: &TemperatureUnit) -> String {
    match temperature_unit {
        TemperatureUnit::Metric => format!("{:.1} km", visibility.km()),
        TemperatureUnit::Imperial => format!("{:.1} mi", visibility.miles()),
    }
}

/// Precipitation amount in mm (metric) or inches (imperial).
pub fn get_precipitation_string(
    precipitation: &Precipitation,
    temperature_unit: &TemperatureUnit,
) -> String {
    match temperature_unit {
        TemperatureUnit::Metric => format!("{:.1} mm", precipitation.mm()),
        TemperatureUnit::Imperial => format!("{:.2} in", precipitation.inches()),
    }
}

/// Bare percentage string (e.g. "40%") — used for rain chance and
/// cloud cover in the compact rows.
pub fn get_percent_string(percent: &Percentage) -> String {
    format!("{}%", percent.get())
}

/// 8-point compass label for a wind bearing ("N", "NE", … "NW").
pub fn wind_direction_label(direction: &WindDirection) -> &'static str {
    direction.cardinal()
}

pub fn spawn_weather_watcher<C>(
    sender: &ComponentSender<C>,
    status_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let service = weather_service();

    let status = service.status.clone();
    let weather = service.weather.clone();

    watch!(sender, [status.watch(), weather.watch()], |out| {
        let _ = out.send(status_state());
    });
}

/// On-disk cache of the last successfully-fetched weather, so the bar pill
/// and menu can show the most recent reading instead of "unavailable" when
/// the provider is unreachable or rate-limited — including across restarts,
/// where the in-memory `weather_service().weather` starts empty.
fn weather_cache_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        });
    base.join("margo").join("weather.json")
}

/// Persist the last good weather snapshot (called on every successful fetch).
pub fn save_weather_cache(weather: &wayle_weather::Weather) {
    let path = weather_cache_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string(weather) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!(error = %e, "weather: failed to write cache");
            }
        }
        Err(e) => tracing::warn!(error = %e, "weather: failed to serialize cache"),
    }
}

/// Load the last good weather snapshot from disk, if any.
pub fn load_weather_cache() -> Option<wayle_weather::Weather> {
    let data = std::fs::read_to_string(weather_cache_path()).ok()?;
    serde_json::from_str(&data).ok()
}

/// Poll interval to apply for a given weather status, or `None` to leave it
/// unchanged (used for the transient `Loading` state). Drives the retry loop:
/// a rate-limit (HTTP 429) backs off hard (1h) because the quota won't reset
/// until its window rolls over; other errors use the configurable fast retry;
/// a successful load returns to the normal cadence.
pub fn weather_poll_interval(
    status: &wayle_weather::WeatherStatus,
    retry_mins: u64,
    normal_mins: u64,
) -> Option<std::time::Duration> {
    use std::time::Duration;
    use wayle_weather::{WeatherErrorKind, WeatherStatus};
    match status {
        WeatherStatus::Error(WeatherErrorKind::RateLimited) => Some(Duration::from_secs(60 * 60)),
        WeatherStatus::Error(_) => Some(Duration::from_secs(retry_mins.max(1) * 60)),
        WeatherStatus::Loaded => Some(Duration::from_secs(normal_mins.max(1) * 60)),
        WeatherStatus::Loading => None,
    }
}

#[cfg(test)]
mod tests {
    use super::weather_poll_interval;
    use std::time::Duration;
    use wayle_weather::{WeatherErrorKind, WeatherStatus};

    #[test]
    fn rate_limit_backs_off_one_hour_ignoring_retry() {
        let i = weather_poll_interval(&WeatherStatus::Error(WeatherErrorKind::RateLimited), 2, 15);
        assert_eq!(i, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn transient_error_uses_fast_retry() {
        let i = weather_poll_interval(&WeatherStatus::Error(WeatherErrorKind::Network), 2, 15);
        assert_eq!(i, Some(Duration::from_secs(120)));
    }

    #[test]
    fn loaded_returns_to_normal_cadence() {
        let i = weather_poll_interval(&WeatherStatus::Loaded, 2, 15);
        assert_eq!(i, Some(Duration::from_secs(900)));
    }

    #[test]
    fn loading_leaves_interval_unchanged() {
        assert_eq!(weather_poll_interval(&WeatherStatus::Loading, 2, 15), None);
    }

    #[test]
    fn zero_minutes_are_clamped_to_one() {
        // Guard against a 0-minute config busy-looping the poller.
        let i = weather_poll_interval(&WeatherStatus::Loaded, 0, 0);
        assert_eq!(i, Some(Duration::from_secs(60)));
    }
}
