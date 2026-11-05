//! World-clock maths, GTK-free: offsets, "3h ahead · Tomorrow morning",
//! working hours, the hour strip, zone search and the clock-pill text.
//! Everything takes an explicit UTC instant so the tests pin time.

use chrono::{DateTime, Datelike, Offset, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

/// Parse an IANA zone id (`Europe/Istanbul`); `None` for anything else.
pub(crate) fn parse_zone(id: &str) -> Option<Tz> {
    id.trim().parse::<Tz>().ok()
}

/// `America/Los_Angeles` → `Los Angeles`, `UTC` → `UTC`.
pub(crate) fn city_label(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).replace('_', " ")
}

/// UTC offset of `tz` at `now`, in minutes (DST-aware).
pub(crate) fn offset_minutes(tz: Tz, now: DateTime<Utc>) -> i32 {
    tz.offset_from_utc_datetime(&now.naive_utc())
        .fix()
        .local_minus_utc()
        / 60
}

/// "Same time" / "3h ahead" / "5h 30m behind" relative to the device.
pub(crate) fn relative_offset(zone_minutes: i32, device_minutes: i32) -> String {
    let diff = zone_minutes - device_minutes;
    if diff == 0 {
        return "Same time".to_string();
    }
    let abs = diff.unsigned_abs();
    let (h, m) = (abs / 60, abs % 60);
    let amount = match (h, m) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    };
    format!("{amount} {}", if diff > 0 { "ahead" } else { "behind" })
}

/// Part of the day for an hour, for the "Tomorrow morning" hint.
pub(crate) fn period(hour: u32) -> &'static str {
    match hour {
        5..=11 => "morning",
        12..=16 => "afternoon",
        17..=20 => "evening",
        _ => "night",
    }
}

/// "Tomorrow morning" / "Yesterday evening" / "+2 days night" when the
/// zone's calendar day differs from the device's; `None` on the same day.
pub(crate) fn day_hint(zone_now: (i32, u32, u32), device_day: (i32, u32)) -> Option<String> {
    let (zone_year, zone_ordinal, hour) = zone_now;
    let (dev_year, dev_ordinal) = device_day;
    // Ordinals only compare within a year; across New Year the other side
    // is always the adjacent day.
    let diff = match zone_year.cmp(&dev_year) {
        std::cmp::Ordering::Equal => zone_ordinal as i64 - dev_ordinal as i64,
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Less => -1,
    };
    let day = match diff {
        0 => return None,
        1 => "Tomorrow".to_string(),
        -1 => "Yesterday".to_string(),
        d => format!("{d:+} days"),
    };
    Some(format!("{day} {}", period(hour)))
}

/// Is `hour` inside the working day `start..end` (24 h, end exclusive)? An
/// end before the start wraps past midnight (a night shift, 22–6).
pub(crate) fn in_work_hours(hour: u32, start: u32, end: u32) -> bool {
    let (start, end) = (start.min(23), end.min(24));
    if start == end {
        false
    } else if start < end {
        (start..end).contains(&hour)
    } else {
        hour >= start || hour < end
    }
}

/// One cell of a city's hour strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HourCell {
    /// Hour of day (`14` / `2p`), or the weekday + date at midnight.
    pub(crate) label: String,
    pub(crate) now: bool,
    pub(crate) work: bool,
    pub(crate) midnight: bool,
}

/// The local hours of `tz` from `before` hours ago to `after` hours ahead
/// of `now`, aligned to whole UTC hours like the device's own strip.
pub(crate) fn hour_strip(
    tz: Tz,
    now: DateTime<Utc>,
    before: u32,
    after: u32,
    use_24h: bool,
    work: (u32, u32),
) -> Vec<HourCell> {
    let floored = now.timestamp() - now.timestamp().rem_euclid(3600);
    (-(before as i64)..=after as i64)
        .filter_map(|col| {
            let at = Utc.timestamp_opt(floored + col * 3600, 0).single()?;
            let local = at.with_timezone(&tz);
            let hour = local.hour();
            let midnight = hour == 0;
            let label = if midnight {
                local.format("%a %-d").to_string()
            } else if use_24h {
                hour.to_string()
            } else {
                let h12 = if hour % 12 == 0 { 12 } else { hour % 12 };
                format!("{h12}{}", if hour < 12 { "a" } else { "p" })
            };
            Some(HourCell {
                label,
                now: col == 0,
                work: in_work_hours(hour, work.0, work.1),
                midnight,
            })
        })
        .collect()
}

/// `HH:MM` or `h:MM am` for `tz` at `now`.
pub(crate) fn clock_text(tz: Tz, now: DateTime<Utc>, use_24h: bool) -> String {
    let local = now.with_timezone(&tz);
    if use_24h {
        local.format("%H:%M").to_string()
    } else {
        local.format("%-I:%M %P").to_string()
    }
}

/// The comparison line under a city: offset, day hint, working hours.
pub(crate) fn describe(
    tz: Tz,
    now: DateTime<Utc>,
    device_offset: i32,
    device_day: (i32, u32),
    work: (u32, u32),
) -> String {
    let local = now.with_timezone(&tz);
    let mut parts = vec![relative_offset(offset_minutes(tz, now), device_offset)];
    if let Some(day) = day_hint((local.year(), local.ordinal(), local.hour()), device_day) {
        parts.push(day);
    }
    parts.push(
        if in_work_hours(local.hour(), work.0, work.1) {
            "Working hours"
        } else {
            "Outside working hours"
        }
        .to_string(),
    );
    parts.join(" · ")
}

/// The IANA areas whose zones are canonical cities.
const REGIONS: &[&str] = &[
    "Africa",
    "America",
    "Antarctica",
    "Arctic",
    "Asia",
    "Atlantic",
    "Australia",
    "Europe",
    "Indian",
    "Pacific",
];

/// Up to `limit` IANA zones whose id or city contains every word of
/// `query` (case-insensitive), best matches (city starts with the query)
/// first.
pub(crate) fn search_zones(query: &str, limit: usize) -> Vec<&'static str> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_lowercase().replace(' ', "_"))
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let needle = query.trim().to_lowercase().replace(' ', "_");
    let mut hits: Vec<(bool, &'static str)> = chrono_tz::TZ_VARIANTS
        .iter()
        .map(|tz| tz.name())
        // Only the canonical region zones: legacy aliases (`US/Pacific`,
        // `Etc/GMT+3`, `Turkey`) only clutter a picker.
        .filter(|name| {
            name.split_once('/')
                .is_some_and(|(region, _)| REGIONS.contains(&region))
        })
        .filter(|name| {
            let hay = name.to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|name| {
            let city = name.rsplit('/').next().unwrap_or(name).to_lowercase();
            (!city.starts_with(&needle), name)
        })
        .collect();
    hits.sort();
    hits.into_iter().take(limit).map(|(_, n)| n).collect()
}

/// The device zone from the `/etc/localtime` symlink target
/// (`/usr/share/zoneinfo/Europe/Istanbul` → `Europe/Istanbul`).
pub(crate) fn zone_from_localtime_target(target: &str) -> Option<String> {
    let rest = target.split("zoneinfo/").nth(1)?;
    let rest = rest.strip_prefix("posix/").unwrap_or(rest);
    parse_zone(rest).map(|_| rest.to_string())
}

/// The clock pill's extra text: the first `count` valid zones as
/// `City HH:MM`, joined by `·`. Empty when there is nothing to show.
pub(crate) fn pill_text(
    zones: &[String],
    count: usize,
    now: DateTime<Utc>,
    use_24h: bool,
) -> String {
    zones
        .iter()
        .filter_map(|id| parse_zone(id).map(|tz| (id, tz)))
        .take(count)
        .map(|(id, tz)| format!("{} {}", city_label(id), clock_text(tz, now, use_24h)))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The clock pill's tooltip block: every valid zone with its time and
/// offset. Empty for an empty list.
pub(crate) fn tooltip_lines(
    zones: &[String],
    now: DateTime<Utc>,
    device_offset: i32,
    use_24h: bool,
) -> String {
    zones
        .iter()
        .filter_map(|id| parse_zone(id).map(|tz| (id, tz)))
        .map(|(id, tz)| {
            format!(
                "{}  {}  ({})",
                city_label(id),
                clock_text(tz, now, use_24h),
                relative_offset(offset_minutes(tz, now), device_offset)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-05 21:30 UTC — a Thursday; New York is on EST (UTC-5) by
    /// then, Istanbul is UTC+3 all year, Kolkata UTC+5:30.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 5, 21, 30, 0).unwrap()
    }

    fn tz(id: &str) -> Tz {
        parse_zone(id).unwrap()
    }

    #[test]
    fn zones_parse_and_label_by_city() {
        assert!(parse_zone("Europe/Istanbul").is_some());
        assert!(parse_zone("Mars/Olympus").is_none());
        assert_eq!(city_label("America/Los_Angeles"), "Los Angeles");
        assert_eq!(city_label("America/Argentina/Buenos_Aires"), "Buenos Aires");
        assert_eq!(city_label("UTC"), "UTC");
    }

    #[test]
    fn offsets_follow_dst() {
        assert_eq!(offset_minutes(tz("Europe/Istanbul"), now()), 180);
        assert_eq!(offset_minutes(tz("America/New_York"), now()), -300);
        let july = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
        assert_eq!(offset_minutes(tz("America/New_York"), july), -240);
        assert_eq!(offset_minutes(tz("Asia/Kolkata"), now()), 330);
    }

    #[test]
    fn relative_offsets_read_naturally() {
        assert_eq!(relative_offset(180, 180), "Same time");
        assert_eq!(relative_offset(-300, 180), "8h behind");
        assert_eq!(relative_offset(330, 180), "2h 30m ahead");
        assert_eq!(relative_offset(210, 180), "30m ahead");
    }

    #[test]
    fn day_hints_name_the_neighbouring_day_and_its_period() {
        assert_eq!(day_hint((2026, 310, 9), (2026, 310)), None);
        assert_eq!(
            day_hint((2026, 311, 6), (2026, 310)).as_deref(),
            Some("Tomorrow morning")
        );
        assert_eq!(
            day_hint((2026, 309, 19), (2026, 310)).as_deref(),
            Some("Yesterday evening")
        );
        // Across New Year.
        assert_eq!(
            day_hint((2027, 1, 2), (2026, 365)).as_deref(),
            Some("Tomorrow night")
        );
    }

    #[test]
    fn work_hours_include_start_exclude_end_and_wrap() {
        assert!(in_work_hours(9, 9, 17));
        assert!(!in_work_hours(17, 9, 17));
        assert!(in_work_hours(23, 22, 6));
        assert!(in_work_hours(3, 22, 6));
        assert!(!in_work_hours(12, 22, 6));
        assert!(!in_work_hours(9, 9, 9), "an empty range is never work");
        assert!(in_work_hours(23, 0, 24));
    }

    #[test]
    fn the_comparison_line_has_offset_day_and_work_hint() {
        // 21:30 UTC = 00:30 Fri in Istanbul (device), 16:30 Thu in New York.
        let ist = tz("Europe/Istanbul");
        let local = now().with_timezone(&ist);
        let device_day = (local.year(), local.ordinal());
        assert_eq!(
            describe(tz("America/New_York"), now(), 180, device_day, (9, 17)),
            "8h behind · Yesterday afternoon · Working hours"
        );
        assert_eq!(
            describe(tz("Asia/Tokyo"), now(), 180, device_day, (9, 17)),
            "6h ahead · Outside working hours"
        );
    }

    #[test]
    fn the_hour_strip_marks_now_work_and_midnight() {
        let strip = hour_strip(tz("Europe/Istanbul"), now(), 2, 3, true, (9, 17));
        let labels: Vec<&str> = strip.iter().map(|c| c.label.as_str()).collect();
        // 21:30 UTC floors to 21:00 = 00:00 Friday in Istanbul.
        assert_eq!(labels, ["22", "23", "Fri 6", "1", "2", "3"]);
        assert!(strip[2].now && strip[2].midnight);
        assert_eq!(strip.iter().filter(|c| c.now).count(), 1);
        assert!(strip.iter().all(|c| !c.work), "night hours are not work");
        let ny = hour_strip(tz("America/New_York"), now(), 0, 1, false, (9, 17));
        assert_eq!(ny[0].label, "4p");
        assert!(ny[0].work && !ny[1].work, "16:00 is work, 17:00 is not");
    }

    #[test]
    fn half_hour_zones_still_align_to_whole_utc_hours() {
        let strip = hour_strip(tz("Asia/Kolkata"), now(), 0, 0, true, (9, 17));
        // 21:00 UTC = 02:30 IST: the strip shows the local hour, 2.
        assert_eq!(strip[0].label, "2");
    }

    #[test]
    fn clock_text_honours_the_12_24_hour_choice() {
        assert_eq!(clock_text(tz("America/New_York"), now(), true), "16:30");
        assert_eq!(clock_text(tz("America/New_York"), now(), false), "4:30 pm");
    }

    #[test]
    fn search_finds_cities_by_words_and_ranks_prefixes_first() {
        let hits = search_zones("york", 5);
        assert_eq!(hits, ["America/New_York"]);
        let hits = search_zones("los ang", 5);
        assert_eq!(hits, ["America/Los_Angeles"]);
        assert!(search_zones("", 5).is_empty());
        assert!(
            search_zones("pacific", 50)
                .iter()
                .all(|z| !z.starts_with("US/"))
        );
        let hits = search_zones("london", 5);
        assert_eq!(hits.first(), Some(&"Europe/London"));
    }

    #[test]
    fn the_device_zone_comes_from_the_localtime_link() {
        assert_eq!(
            zone_from_localtime_target("/usr/share/zoneinfo/Europe/Istanbul").as_deref(),
            Some("Europe/Istanbul")
        );
        assert_eq!(
            zone_from_localtime_target("../usr/share/zoneinfo/posix/Asia/Tokyo").as_deref(),
            Some("Asia/Tokyo")
        );
        assert_eq!(zone_from_localtime_target("/etc/whatever"), None);
    }

    #[test]
    fn the_pill_shows_the_first_n_valid_zones() {
        let zones = vec![
            "Nowhere/Land".to_string(),
            "America/New_York".to_string(),
            "Asia/Tokyo".to_string(),
        ];
        assert_eq!(pill_text(&zones, 0, now(), true), "");
        assert_eq!(pill_text(&zones, 1, now(), true), "New York 16:30");
        assert_eq!(
            pill_text(&zones, 3, now(), true),
            "New York 16:30 · Tokyo 06:30"
        );
        assert_eq!(
            tooltip_lines(&zones[1..2], now(), 180, true),
            "New York  16:30  (8h behind)"
        );
    }
}
