//! Battery-level history for the power menu's graph — ports the noctalia
//! `battery-graph` plugin natively. UPower already records the charge
//! curve (`/var/lib/upower/history-charge-*.dat`); we read it over D-Bus
//! with `org.freedesktop.UPower.Device.GetHistory("charge", span, n)`
//! instead of the plugin's `gdbus` shell-out.
//!
//! The decision logic (which device, which samples, where they land on
//! the time axis) is pure and unit-tested against real `GetHistory`
//! output captured on the owner's ThinkPad; the cairo drawing lives with
//! the power menu.

use zbus::{proxy, zvariant::OwnedObjectPath};

/// UPower `GetHistory` sample: `(unix time, value %, state)`.
pub(crate) type RawSample = (u32, f64, u32);

/// UPower device state `0` = unknown. UPower closes every history file
/// with a `(t, 0, 0)` marker (seen on the ThinkPad), which must not be
/// drawn as a drop to 0 %.
const STATE_UNKNOWN: u32 = 0;
/// Resolution requested from UPower (max points it returns).
pub(crate) const RESOLUTION: u32 = 200;

/// Timeframe presets (hours) shown under the graph.
pub(crate) const PRESET_HOURS: [u32; 3] = [6, 24, 168];

/// One graph point: `x` in 0..=1 across the timeframe (0 = oldest edge,
/// 1 = now) and the charge in percent (0..=100).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: f64,
    pub(crate) pct: f64,
}

/// Clamp the configured `power.history_hours` into a sane window
/// (1 h .. 7 d; UPower keeps about a week of history).
pub(crate) fn clamp_hours(hours: u32) -> u32 {
    hours.clamp(1, 168)
}

/// `6h` / `24h` / `7d` label for a timeframe.
pub(crate) fn hours_label(hours: u32) -> String {
    if hours >= 24 && hours.is_multiple_of(24) && hours > 24 {
        format!("{}d", hours / 24)
    } else {
        format!("{hours}h")
    }
}

/// Turn raw `GetHistory` samples into graph points, oldest first.
/// Drops unknown-state markers and anything outside `[now - span, now]`,
/// collapses duplicate timestamps (keeping the last reported value) and
/// clamps the charge to 0..=100.
pub(crate) fn to_points(raw: &[RawSample], now: u32, span_secs: u32) -> Vec<Point> {
    if span_secs == 0 {
        return Vec::new();
    }
    let start = now.saturating_sub(span_secs);
    let mut samples: Vec<(u32, f64)> = raw
        .iter()
        .filter(|(t, _, state)| *state != STATE_UNKNOWN && *t >= start && *t <= now)
        .map(|(t, v, _)| (*t, v.clamp(0.0, 100.0)))
        .collect();
    // Stable sort keeps UPower's order inside equal timestamps; UPower
    // lists newest first, so the *first* of a run is the latest value.
    samples.sort_by_key(|(t, _)| *t);
    let mut out: Vec<Point> = Vec::with_capacity(samples.len());
    let mut last_t: Option<u32> = None;
    for (t, pct) in samples {
        if last_t == Some(t) {
            continue;
        }
        last_t = Some(t);
        out.push(Point {
            x: f64::from(t - start) / f64::from(span_secs),
            pct,
        });
    }
    out
}

/// Pick the laptop battery among UPower's devices: the first
/// `…/battery_*` path (sorted for determinism), never the aggregate
/// `DisplayDevice` or a peripheral (`headset_*`, `mouse_*`).
pub(crate) fn pick_battery(paths: &[String]) -> Option<String> {
    let mut batteries: Vec<&String> = paths
        .iter()
        .filter(|p| {
            p.rsplit('/')
                .next()
                .is_some_and(|leaf| leaf.starts_with("battery_"))
        })
        .collect();
    batteries.sort();
    batteries.first().map(|p| (*p).clone())
}

/// Map points to pixel coordinates in a `w × h` area, fixed 0–100 %
/// scale (so a flat 80 % reads as 80 %, unlike an auto-scaled sparkline),
/// with 2 px headroom so the stroke isn't clipped at 0 / 100 %.
pub(crate) fn to_pixels(points: &[Point], w: f64, h: f64) -> Vec<(f64, f64)> {
    points
        .iter()
        .map(|p| (p.x * w, h - 2.0 - (p.pct / 100.0) * (h - 4.0)))
        .collect()
}

/// `min · max` summary under the graph (empty when there's no data).
pub(crate) fn summary(points: &[Point]) -> String {
    let Some(first) = points.first() else {
        return String::new();
    };
    let (lo, hi) = points.iter().fold((first.pct, first.pct), |(lo, hi), p| {
        (lo.min(p.pct), hi.max(p.pct))
    });
    format!("Low {lo:.0}%  ·  High {hi:.0}%")
}

#[proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    fn enumerate_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower"
)]
trait UPowerDevice {
    fn get_history(
        &self,
        type_: &str,
        timespan: u32,
        resolution: u32,
    ) -> zbus::Result<Vec<(u32, f64, u32)>>;
}

/// Fetch the battery charge history for the last `hours` as graph points.
/// `Ok(empty)` when there's no battery or no recorded history.
pub(crate) async fn fetch(hours: u32) -> zbus::Result<Vec<Point>> {
    let span = clamp_hours(hours) * 3600;
    // The shell's one shared system-bus connection, not a new one (Hello,
    // auth, AddMatch) per fetch.
    let conn = mshell_services::systemd::system_bus().await?;
    let upower = UPowerProxy::new(&conn).await?;
    let paths: Vec<String> = upower
        .enumerate_devices()
        .await?
        .into_iter()
        .map(|p| p.to_string())
        .collect();
    let Some(path) = pick_battery(&paths) else {
        return Ok(Vec::new());
    };
    let device = UPowerDeviceProxy::builder(&conn)
        .path(path)?
        .build()
        .await?;
    let raw = device.get_history("charge", span, RESOLUTION).await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u32::try_from(d.as_secs()).unwrap_or(u32::MAX))
        .unwrap_or(0);
    Ok(to_points(&raw, now, span))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `GetHistory charge 3600 20` output from the owner's ThinkPad
    /// (busctl, charging 65 → 78 %), newest first, closed by the
    /// `(t, 0, 0)` unknown-state marker.
    const THINKPAD: &[RawSample] = &[
        (1793809911, 65.0, 2),
        (1793809791, 66.0, 2),
        (1793809640, 67.0, 2),
        (1793809520, 68.0, 2),
        (1793809370, 69.0, 2),
        (1793809250, 70.0, 2),
        (1793809099, 71.0, 2),
        (1793808949, 72.0, 2),
        (1793808798, 73.0, 2),
        (1793808678, 74.0, 2),
        (1793808558, 75.0, 2),
        (1793808468, 76.0, 2),
        (1793808377, 77.0, 2),
        (1793808287, 78.0, 2),
        (1793808287, 0.0, 0),
    ];

    #[test]
    fn real_history_drops_unknown_marker_and_sorts_oldest_first() {
        let now = 1793809911 + 60;
        let pts = to_points(THINKPAD, now, 3600);
        assert_eq!(pts.len(), 14, "the (t, 0, 0) marker must be dropped");
        assert!(pts.iter().all(|p| p.pct > 0.0));
        assert_eq!(pts.first().unwrap().pct, 78.0);
        assert_eq!(pts.last().unwrap().pct, 65.0);
        assert!(pts.windows(2).all(|w| w[0].x < w[1].x));
        assert!(pts.iter().all(|p| (0.0..=1.0).contains(&p.x)));
        // Newest sample lands 60 s before the right edge.
        let last = pts.last().unwrap().x;
        assert!((last - (3540.0 / 3600.0)).abs() < 1e-9, "{last}");
    }

    #[test]
    fn a_lone_unknown_state_marker_is_not_a_drop_to_zero() {
        let raw = [(100, 50.0, 2), (90, 0.0, 0), (80, 51.0, 2)];
        let pts = to_points(&raw, 100, 100);
        assert_eq!(pts.len(), 2);
        assert!(pts.iter().all(|p| p.pct > 0.0));
    }

    #[test]
    fn samples_outside_the_window_are_dropped() {
        let raw = [(100, 50.0, 1), (5000, 60.0, 1), (9000, 70.0, 1)];
        let pts = to_points(&raw, 9000, 3600);
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0], Point { x: 1.0, pct: 70.0 });
        assert!(to_points(&raw, 9000, 0).is_empty());
    }

    #[test]
    fn duplicate_timestamps_keep_the_newest_reported_value() {
        // UPower lists newest first: (t=10, 40) was reported after (t=10, 41).
        let raw = [(10, 40.0, 2), (10, 41.0, 2), (5, 42.0, 2)];
        let pts = to_points(&raw, 20, 20);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[1].pct, 40.0);
    }

    #[test]
    fn values_are_clamped() {
        let pts = to_points(&[(10, 101.5, 4), (11, -3.0, 2)], 20, 20);
        assert_eq!(pts[0].pct, 100.0);
        assert_eq!(pts[1].pct, 0.0);
    }

    #[test]
    fn picks_the_laptop_battery_not_peripherals_or_display_device() {
        let paths: Vec<String> = [
            "/org/freedesktop/UPower/devices/line_power_AC",
            "/org/freedesktop/UPower/devices/headset_dev_00_7F_1D_22_4C_18",
            "/org/freedesktop/UPower/devices/DisplayDevice",
            "/org/freedesktop/UPower/devices/battery_BAT1",
            "/org/freedesktop/UPower/devices/battery_BAT0",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            pick_battery(&paths).as_deref(),
            Some("/org/freedesktop/UPower/devices/battery_BAT0")
        );
        assert_eq!(pick_battery(&paths[..3]), None);
    }

    #[test]
    fn hours_clamp_and_labels() {
        assert_eq!(clamp_hours(0), 1);
        assert_eq!(clamp_hours(6), 6);
        assert_eq!(clamp_hours(1000), 168);
        assert_eq!(hours_label(6), "6h");
        assert_eq!(hours_label(24), "24h");
        assert_eq!(hours_label(168), "7d");
        assert_eq!(hours_label(36), "36h");
    }

    #[test]
    fn pixels_use_a_fixed_percent_scale() {
        let pts = [
            Point { x: 0.0, pct: 100.0 },
            Point { x: 1.0, pct: 0.0 },
            Point { x: 0.5, pct: 50.0 },
        ];
        let px = to_pixels(&pts, 200.0, 64.0);
        assert_eq!(px[0], (0.0, 2.0));
        assert_eq!(px[1], (200.0, 62.0));
        assert_eq!(px[2], (100.0, 32.0));
    }

    #[test]
    fn summary_reports_low_and_high() {
        assert_eq!(summary(&[]), "");
        let pts = to_points(THINKPAD, 1793809911, 3600);
        assert_eq!(summary(&pts), "Low 65%  ·  High 78%");
    }
}
