//! power-profiles-daemon control via `powerprofilesctl`.
//!
//! We shell out to the CLI rather than speaking D-Bus directly: it is the
//! same path the retired script used, has zero extra dependencies, and the
//! daemon's tick rate makes a fork-per-tick negligible. Profile names are
//! ppd's own: `performance` / `balanced` / `power-saver`.

use std::process::Command;

/// The currently active profile, or `None` if ppd / the CLI is unavailable.
pub fn get() -> Option<String> {
    let out = Command::new("powerprofilesctl").arg("get").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Set the active profile. Returns `true` on success.
pub fn set(profile: &str) -> bool {
    Command::new("powerprofilesctl")
        .arg("set")
        .arg(profile)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Available profiles in `powerprofilesctl list` order (top→bottom, which
/// is typically performance → balanced → power-saver). Empty if the CLI
/// is unavailable — callers fall back to the canonical order.
pub fn list() -> Vec<String> {
    match Command::new("powerprofilesctl").arg("list").output() {
        Ok(o) if o.status.success() => parse_list(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// Parse `powerprofilesctl list`: profile headers are `name:` (optionally
/// `* `-prefixed for the active one); detail lines are `Key: value` and are
/// skipped because they don't *end* with the colon.
pub fn parse_list(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|line| {
            let t = line.trim().trim_start_matches('*').trim();
            let name = t.strip_suffix(':')?;
            if name.is_empty() || name.chars().any(char::is_whitespace) {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_list;

    #[test]
    fn parses_profile_headers_only() {
        let out = "  performance:\n    CpuDriver:  intel_pstate\n    Degraded:   no\n\
                   * balanced:\n    CpuDriver:  intel_pstate\n  power-saver:\n    CpuDriver:  intel_pstate\n";
        assert_eq!(
            parse_list(out),
            vec!["performance", "balanced", "power-saver"]
        );
    }
}
