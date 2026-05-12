//! Weather via Open-Meteo (free, no API key).
//!
//! Geocoder lookup is cached for the session — Noctalia's config
//! ships the city as `"Atasehir"`; we resolve it to a (lat, lon)
//! once on first call and reuse forever. Forecast pull is one
//! curl per refresh.

use std::cell::RefCell;
use std::process::Command;

use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub temp_c: i32,
    pub feels_like_c: i32,
    pub weather_code: u32,
    /// Resolved geocoder name (e.g. "Atasehir").
    pub city: String,
    pub humidity_pct: u32,
    pub wind_kmh: u32,
}

/// City name — Noctalia setting `name: "Atasehir"`. Will become
/// user-tunable once mshell grows a config file again.
const CITY: &str = "Atasehir";

thread_local! {
    /// Geocoder cache. `None` = not looked up yet, `Some((None, _))`
    /// = lookup attempted and failed (don't retry every refresh).
    static GEO: RefCell<Option<Option<(f64, f64)>>> = const { RefCell::new(None) };
}

pub fn current() -> Option<Snapshot> {
    let (lat, lon) = geocode()?;
    let forecast = fetch_forecast(lat, lon)?;
    let cur = forecast.get("current")?;
    Some(Snapshot {
        temp_c: cur
            .get("temperature_2m")
            .and_then(Value::as_f64)
            .map(|v| v.round() as i32)
            .unwrap_or(0),
        feels_like_c: cur
            .get("apparent_temperature")
            .and_then(Value::as_f64)
            .map(|v| v.round() as i32)
            .unwrap_or(0),
        weather_code: cur
            .get("weather_code")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        city: CITY.to_string(),
        humidity_pct: cur
            .get("relative_humidity_2m")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        wind_kmh: cur
            .get("wind_speed_10m")
            .and_then(Value::as_f64)
            .map(|v| v.round() as u32)
            .unwrap_or(0),
    })
}

/// Resolve the city to (lat, lon) via the geocoding API. Cached
/// per-thread for the lifetime of the bar process.
fn geocode() -> Option<(f64, f64)> {
    let cached = GEO.with(|g| g.borrow().clone());
    if let Some(opt) = cached {
        return opt;
    }
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&format=json",
        CITY
    );
    let result = curl_json(&url).and_then(|json| {
        let first = json.get("results")?.as_array()?.first()?;
        let lat = first.get("latitude").and_then(Value::as_f64)?;
        let lon = first.get("longitude").and_then(Value::as_f64)?;
        Some((lat, lon))
    });
    GEO.with(|g| *g.borrow_mut() = Some(result));
    result
}

fn fetch_forecast(lat: f64, lon: f64) -> Option<Value> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}\
         &current=temperature_2m,apparent_temperature,weather_code,relative_humidity_2m,wind_speed_10m\
         &timezone=auto"
    );
    curl_json(&url)
}

fn curl_json(url: &str) -> Option<Value> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "5", url])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Open-Meteo WMO weather code → Nerd Font glyph.
pub fn glyph(code: u32) -> &'static str {
    match code {
        0 => "\u{f185}",                   // sun (clear)
        1 | 2 => "\u{f6c4}",               // partly cloudy
        3 => "\u{f0c2}",                   // cloud
        45 | 48 => "\u{f74e}",             // fog
        51..=57 => "\u{f743}",             // drizzle
        61..=67 => "\u{f73d}",             // rain
        71..=77 | 85 | 86 => "\u{f76b}",   // snow
        80..=82 => "\u{f73c}",             // showers
        95 | 96 | 99 => "\u{f76c}",        // thunderstorm
        _ => "\u{f077}",                   // dash
    }
}

/// Short human label — what hover tooltips use.
pub fn label(code: u32) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 => "Light drizzle",
        53 => "Drizzle",
        55 => "Dense drizzle",
        56 | 57 => "Freezing drizzle",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        66 | 67 => "Freezing rain",
        71 => "Light snow",
        73 => "Snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Light showers",
        81 => "Showers",
        82 => "Violent showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm w/ hail",
        _ => "Unknown",
    }
}
