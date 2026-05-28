//! Composite keys that namespace plugins by their source.
//!
//! Plugins from the official source keep their plain `id`. Plugins from a
//! custom source are stored as `<hash>:<id>`, where `hash` is a short stable
//! digest of the source URL — so two sources can ship a plugin with the same
//! id without colliding on disk or in the enabled list.

/// Length of the source-hash prefix (hex chars).
const HASH_LEN: usize = 6;

/// Short, stable digest of a source URL (FNV-1a, lower-cased hex). Not
/// cryptographic — just a deterministic namespace tag, so no crate dep.
pub fn source_hash(url: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
    for b in url.trim().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
    }
    // Keep the low 24 bits → 6 hex chars.
    format!("{:0width$x}", h & 0x00ff_ffff, width = HASH_LEN)
}

/// Build the on-disk / enabled-list key for a plugin from a given source.
/// Official source → plain `id`; any other → `<hash>:<id>`.
pub fn composite_key(id: &str, source_url: &str, official_url: &str) -> String {
    if source_url.trim() == official_url.trim() {
        id.to_string()
    } else {
        format!("{}:{}", source_hash(source_url), id)
    }
}

/// Split a composite key back into `(source_hash, id)`. A plain id (official
/// source) yields `(None, id)`. Only a `:` at exactly the hash boundary is
/// treated as a prefix, so plain ids are never misread.
pub fn parse_composite_key(key: &str) -> (Option<String>, String) {
    if let Some(idx) = key.find(':')
        && idx == HASH_LEN
    {
        return (Some(key[..idx].to_string()), key[idx + 1..].to_string());
    }
    (None, key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFICIAL: &str = "https://github.com/kenanpelit/margo-plugins";
    const CUSTOM: &str = "https://github.com/someone/their-plugins";

    #[test]
    fn hash_is_stable_and_sized() {
        let a = source_hash(CUSTOM);
        let b = source_hash(CUSTOM);
        assert_eq!(a, b);
        assert_eq!(a.len(), HASH_LEN);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn official_keeps_plain_id() {
        assert_eq!(composite_key("weather", OFFICIAL, OFFICIAL), "weather");
    }

    #[test]
    fn custom_source_gets_hash_prefix() {
        let key = composite_key("weather", CUSTOM, OFFICIAL);
        let expected = format!("{}:weather", source_hash(CUSTOM));
        assert_eq!(key, expected);
    }

    #[test]
    fn round_trips() {
        let (h, id) = parse_composite_key("weather");
        assert_eq!(h, None);
        assert_eq!(id, "weather");

        let key = composite_key("weather", CUSTOM, OFFICIAL);
        let (h, id) = parse_composite_key(&key);
        assert_eq!(h, Some(source_hash(CUSTOM)));
        assert_eq!(id, "weather");
    }
}
