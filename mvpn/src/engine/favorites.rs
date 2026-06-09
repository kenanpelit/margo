//! Favorite relays: `~/.mullvad/favorites.txt`, one `relay|ping_avg` per line,
//! sorted fastest-first. Format-compatible with osc-mullvad so an existing file
//! carries over untouched.

use std::path::PathBuf;

use super::{actions, latency, relays, status, sys};

const SENTINEL: f64 = 999_999.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Fav {
    pub relay: String,
    /// Average ping in ms, or `None` when unmeasured ("N/A").
    pub ping: Option<f64>,
}

/// Favorites path. Precedence (mirrors osc-mullvad): `$OSC_MULLVAD_FAVORITES_FILE`
/// → `$OSC_MULLVAD_CONFIG_DIR`/favorites.txt → `$OSC_MULLVAD_DIR`/favorites.txt →
/// `~/.mullvad/favorites.txt`.
pub fn path() -> PathBuf {
    if let Ok(p) = std::env::var("OSC_MULLVAD_FAVORITES_FILE") {
        return PathBuf::from(p);
    }
    let dir = std::env::var("OSC_MULLVAD_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| sys::mullvad_dir());
    dir.join("favorites.txt")
}

fn sort_key(p: Option<f64>) -> f64 {
    p.unwrap_or(SENTINEL)
}

/// Parse the file body into entries, sorted fastest-first.
pub fn parse(body: &str) -> Vec<Fav> {
    let mut v: Vec<Fav> = body
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (relay, ping) = match line.split_once('|') {
                Some((r, p)) => (r.trim().to_string(), p.trim().parse::<f64>().ok()),
                None => (line.to_string(), None),
            };
            if relay.is_empty() {
                None
            } else {
                Some(Fav { relay, ping })
            }
        })
        .collect();
    v.sort_by(|a, b| {
        sort_key(a.ping)
            .partial_cmp(&sort_key(b.ping))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    v
}

pub fn load() -> Vec<Fav> {
    parse(&std::fs::read_to_string(path()).unwrap_or_default())
}

pub fn serialize(favs: &[Fav]) -> String {
    favs.iter()
        .map(|f| match f.ping {
            Some(p) => format!("{}|{}", f.relay, p),
            None => format!("{}|N/A", f.relay),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn save(favs: &[Fav]) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&p, serialize(favs));
}

/// Insert or update a relay's ping, then re-sort + persist.
pub fn upsert(relay: &str, ping: Option<f64>) {
    let mut favs = load();
    if let Some(f) = favs.iter_mut().find(|f| f.relay == relay) {
        f.ping = ping;
    } else {
        favs.push(Fav {
            relay: relay.to_string(),
            ping,
        });
    }
    let sorted = parse(&serialize(&favs));
    save(&sorted);
}

pub fn remove(relay: &str) {
    let favs: Vec<Fav> = load().into_iter().filter(|f| f.relay != relay).collect();
    save(&favs);
}

/// Add the currently-connected relay to favorites (measuring its ping).
pub fn add_current() -> bool {
    let st = status::query();
    if st.relay.is_empty() {
        return false;
    }
    let list = sys::mullvad(&["relay", "list"]);
    let ping = relays::relay_ipv4(&list, &st.relay).and_then(|ip| latency::ping_avg(&ip, 3, 2));
    upsert(&st.relay, ping);
    true
}

/// Connect to the fastest favorite (first after sort). Returns the relay id.
pub fn connect_fastest() -> Option<String> {
    let favs = load();
    let first = favs.first()?.relay.clone();
    if actions::set_relay(&first) {
        Some(first)
    } else {
        None
    }
}

/// Re-ping every favorite (optionally filtered by country prefix), drop dead
/// ones, re-sort. Returns the surviving list.
pub fn refresh(country: &str, count: u32, timeout: u32) -> Vec<Fav> {
    let list = sys::mullvad(&["relay", "list"]);
    let favs = load();
    let targets: Vec<(String, String)> = favs
        .iter()
        .filter(|f| country.is_empty() || f.relay.starts_with(&format!("{country}-")))
        .filter_map(|f| relays::relay_ipv4(&list, &f.relay).map(|ip| (f.relay.clone(), ip)))
        .collect();
    let measured = latency::ping_many(&targets, count, timeout);
    // Keep favorites outside the country filter untouched; replace measured.
    let mut out: Vec<Fav> = favs
        .into_iter()
        .filter(|f| !country.is_empty() && !f.relay.starts_with(&format!("{country}-")))
        .collect();
    for (relay, avg) in measured {
        out.push(Fav {
            relay,
            ping: Some(avg),
        });
    }
    let sorted = parse(&serialize(&out));
    save(&sorted);
    sorted
}

/// Find the fastest relay among a sampled set in `country` (empty = all),
/// connect to it, and record it in favorites. Returns (relay, ping_ms).
pub fn fastest(country: &str, sample: usize, count: u32, timeout: u32) -> Option<(String, f64)> {
    let list = sys::mullvad(&["relay", "list"]);
    let mut ids = relays::pick_relays(&list, country, "", relays::Ownership::Any);
    if ids.is_empty() {
        return None;
    }
    // Cheap shuffle-and-take to bound the ping count.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    let len = ids.len();
    ids.rotate_left(nanos % len);
    ids.truncate(sample.max(1));
    let targets: Vec<(String, String)> = ids
        .iter()
        .filter_map(|id| relays::relay_ipv4(&list, id).map(|ip| (id.clone(), ip)))
        .collect();
    let measured = latency::ping_many(&targets, count, timeout);
    let (relay, avg) = measured.into_iter().next()?; // sorted fastest-first
    if actions::set_relay(&relay) {
        upsert(&relay, Some(avg));
        Some((relay, avg))
    } else {
        None
    }
}

/// Seed favorites across many countries: for each code, ping a sample and add
/// the fastest relay (does not connect). Returns how many were seeded.
pub fn sweep(codes: &[&str], per: usize, count: u32, timeout: u32) -> usize {
    let list = sys::mullvad(&["relay", "list"]);
    let mut seeded = 0;
    for cc in codes {
        let ids = relays::pick_relays(&list, cc, "", relays::Ownership::Any);
        let targets: Vec<(String, String)> = ids
            .iter()
            .take(per.max(1))
            .filter_map(|id| relays::relay_ipv4(&list, id).map(|ip| (id.clone(), ip)))
            .collect();
        if let Some((relay, avg)) = latency::ping_many(&targets, count, timeout)
            .into_iter()
            .next()
        {
            upsert(&relay, Some(avg));
            seeded += 1;
        }
    }
    seeded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sorts_numeric_na_last() {
        let body = "de-ber-wg-006|12.5\nfr-par-wg-001|N/A\nus-nyc-wg-101|3.2\n";
        let favs = parse(body);
        assert_eq!(favs[0].relay, "us-nyc-wg-101");
        assert_eq!(favs[1].relay, "de-ber-wg-006");
        assert_eq!(favs[2].relay, "fr-par-wg-001"); // N/A sorts last
        assert_eq!(favs[2].ping, None);
    }

    #[test]
    fn serialize_roundtrip() {
        let favs = vec![
            Fav {
                relay: "a-b-wg-1".into(),
                ping: Some(3.2),
            },
            Fav {
                relay: "c-d-wg-2".into(),
                ping: None,
            },
        ];
        let s = serialize(&favs);
        let back = parse(&s);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].relay, "a-b-wg-1");
        assert_eq!(back[0].ping, Some(3.2));
        assert_eq!(back[1].ping, None);
    }

    #[test]
    fn parse_tolerates_bare_lines() {
        let favs = parse("de-ber-wg-006\n\n");
        assert_eq!(favs.len(), 1);
        assert_eq!(favs[0].ping, None);
    }
}
