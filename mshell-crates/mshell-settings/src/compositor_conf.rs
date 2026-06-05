//! Shared read / patch / reload for margo's compositor `config.conf`, used by
//! the Appearance / Effects / Behaviour Settings pages.
//!
//! `config.conf` is frequently a dotfiles **symlink** — `std::fs::write`
//! follows it, so we edit in place (content lands in the dotfiles repo, the
//! symlink is preserved). Reads are raw-line based (not via `margo_config`) so
//! enum-valued keys (`drag_corner`, `hotarea_corner`, `allow_tearing`, …) come
//! back as the integer that's actually in the file — no enum↔index juggling.

use std::path::PathBuf;

pub(crate) fn conf_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    base.join("margo").join("config.conf")
}

/// Raw value for `key` from `config.conf` (`key = value  # comment` → `value`),
/// or `None` if the key is absent/commented. Inline `#` comments are stripped.
pub(crate) fn read_raw(key: &str) -> Option<String> {
    let body = std::fs::read_to_string(conf_path()).ok()?;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(val) = rest.strip_prefix('=') {
                let val = val.split('#').next().unwrap_or("").trim();
                return Some(val.to_string());
            }
        }
    }
    None
}

pub(crate) fn read_int(key: &str, default: i64) -> i64 {
    read_raw(key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

pub(crate) fn read_f64(key: &str, default: f64) -> f64 {
    read_raw(key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

pub(crate) fn read_bool(key: &str, default: bool) -> bool {
    match read_raw(key) {
        Some(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        None => default,
    }
}

/// Replace each `key = value` line in place (first match), or append it.
/// Preserves the rest of the file (comments, blank lines, ordering).
pub(crate) fn patch_conf(updates: &[(&str, String)]) {
    let path = conf_path();
    let body = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = body.lines().map(|s| s.to_string()).collect();
    for (key, val) in updates {
        let mut found = false;
        for line in lines.iter_mut() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix(*key)
                && rest.trim_start().starts_with('=')
            {
                *line = format!("{key} = {val}");
                found = true;
                break;
            }
        }
        if !found {
            lines.push(format!("{key} = {val}"));
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    if let Err(e) = std::fs::write(&path, out) {
        tracing::warn!(error = %e, "settings: failed to write config.conf");
    }
}

/// Set a single key and re-parse the compositor config live.
pub(crate) fn set_and_reload(key: &str, val: String) {
    patch_conf(&[(key, val)]);
    reload();
}

pub(crate) fn reload() {
    if let Err(e) = std::process::Command::new("mctl").args(["reload"]).spawn() {
        tracing::warn!(error = %e, "settings: `mctl reload` failed to spawn");
    }
}
