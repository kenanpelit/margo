//! "My country": connect to the country the user is really in, or the
//! nearest one with relays.
//!
//! Where the user really is only shows while the tunnel is down (the
//! daemon's visible location is then the ISP's, `mullvad_exit_ip: false`).
//! mvpn records it each time it sees one (`real-location` in the Mullvad
//! data dir: country + coordinates, nothing else) so the pick also works
//! while connected.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::relays::Location;
use super::{sys, tunnel};

/// The last real (non-VPN) location seen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RealLocation {
    pub country: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

impl RealLocation {
    /// The real location a tunnel state shows, if it shows one: a
    /// disconnected tunnel whose visible address is not a Mullvad exit.
    pub fn from_tunnel(t: &tunnel::Tunnel) -> Option<Self> {
        let p = t.place.as_ref()?;
        (t.state == "disconnected" && !p.mullvad_exit && !p.country.is_empty()).then(|| Self {
            country: p.country.clone(),
            latitude: p.latitude,
            longitude: p.longitude,
        })
    }
}

/// `$OSC_MULLVAD_DIR`/`~/.mullvad` + `real-location`.
pub fn path() -> PathBuf {
    sys::mullvad_dir().join("real-location")
}

pub fn load() -> Option<RealLocation> {
    serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()
}

/// Remember `real` unless it is already what is stored.
pub fn save(real: &RealLocation) {
    if load().as_ref() == Some(real) {
        return;
    }
    if let Ok(body) = serde_json::to_string(real) {
        let _ = sys::write_atomic(&path(), &body);
    }
}

/// Read the tunnel state and, when it shows the real location, store it.
/// Returns what is known afterwards (the fresh one or the stored one).
pub fn refresh() -> Option<RealLocation> {
    if let Some(real) = tunnel::query().as_ref().and_then(RealLocation::from_tunnel) {
        save(&real);
        return Some(real);
    }
    load()
}

/// Great-circle distance in km.
fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = std::f64::consts::PI / 180.0;
    let (dlat, dlon) = ((lat2 - lat1) * r, (lon2 - lon1) * r);
    let a = (dlat / 2.0).sin().powi(2)
        + (lat1 * r).cos() * (lat2 * r).cos() * (dlon / 2.0).sin().powi(2);
    2.0 * 6371.0 * a.min(1.0).sqrt().asin()
}

/// The country to connect to for `real`: the one with its name, else the
/// one with the city nearest to its coordinates. `None` without a match
/// and without coordinates.
pub fn pick(locations: &[Location], real: &RealLocation) -> Option<String> {
    if let Some(c) = locations
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(&real.country))
    {
        return Some(c.code.clone());
    }
    let (lat, lon) = (real.latitude?, real.longitude?);
    locations
        .iter()
        .flat_map(|c| {
            c.cities
                .iter()
                .map(move |ci| (haversine(lat, lon, ci.latitude, ci.longitude), &c.code))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, code)| code.clone())
}

#[cfg(test)]
mod tests {
    use super::super::relays::parse_locations;
    use super::*;

    const LIST: &str = include_str!("../../tests/fixtures/relay-list.txt");

    fn real(country: &str, lat: f64, lon: f64) -> RealLocation {
        RealLocation {
            country: country.into(),
            latitude: Some(lat),
            longitude: Some(lon),
        }
    }

    #[test]
    fn picks_the_own_country_by_name() {
        let t = parse_locations(LIST);
        assert_eq!(pick(&t, &real("Turkey", 39.9, 32.8)), Some("tr".into()));
        assert_eq!(pick(&t, &real("sweden", 0.0, 0.0)), Some("se".into()));
    }

    #[test]
    fn falls_back_to_the_nearest_country_with_relays() {
        let t = parse_locations(LIST);
        // Skopje (North Macedonia, no relays): Tirana (~150 km) beats Sofia.
        assert_eq!(
            pick(&t, &real("North Macedonia", 41.99, 21.43)),
            Some("al".into())
        );
        // Thessaloniki (Greece is not in this list): Sofia (~230 km).
        assert_eq!(pick(&t, &real("Greece", 40.64, 22.94)), Some("bg".into()));
        // Copenhagen (not in this list): Malmö, Sweden.
        assert_eq!(pick(&t, &real("Denmark", 55.68, 12.57)), Some("se".into()));
        // New Zealand: Australia (Sydney / Brisbane), across the date line.
        assert_eq!(
            pick(&t, &real("New Zealand", -36.85, 174.76)),
            Some("au".into())
        );
        // Washington DC: the US.
        assert_eq!(pick(&t, &real("Nowhere", 38.9, -77.0)), Some("us".into()));
        // No name match and no coordinates: no pick.
        let none = RealLocation {
            country: "Atlantis".into(),
            latitude: None,
            longitude: None,
        };
        assert_eq!(pick(&t, &none), None);
        assert_eq!(pick(&[], &real("Turkey", 1.0, 1.0)), None);
    }

    #[test]
    fn only_a_disconnected_non_mullvad_location_is_real() {
        let t = |json: &str| tunnel::parse(json).expect("tunnel");
        let home = t(
            r#"{"state":"disconnected","details":{"location":{"latitude":41.0,"longitude":29.0,"mullvad_exit_ip":false,"country":"Turkey","city":"Istanbul"}}}"#,
        );
        assert_eq!(
            RealLocation::from_tunnel(&home),
            Some(real("Turkey", 41.0, 29.0))
        );
        let exit = t(include_str!("../../tests/fixtures/status-connected.json"));
        assert_eq!(RealLocation::from_tunnel(&exit), None);
        // Disconnected but still showing a Mullvad exit (lookup not redone).
        let stale = t(
            r#"{"state":"disconnected","details":{"location":{"mullvad_exit_ip":true,"country":"Germany"}}}"#,
        );
        assert_eq!(RealLocation::from_tunnel(&stale), None);
        let unknown = t(r#"{"state":"disconnected","details":{}}"#);
        assert_eq!(RealLocation::from_tunnel(&unknown), None);
    }

    #[test]
    fn haversine_is_sane() {
        // Istanbul → Ankara ≈ 350 km.
        let d = haversine(41.0082, 28.9784, 39.9334, 32.8597);
        assert!((340.0..360.0).contains(&d), "{d}");
        assert!(haversine(1.0, 1.0, 1.0, 1.0).abs() < 1e-9);
    }
}
