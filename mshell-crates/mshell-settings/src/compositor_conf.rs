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
    parse_raw(&std::fs::read_to_string(conf_path()).ok()?, key)
}

/// Pure value-extraction over a config body (no IO) — see [`read_raw`].
fn parse_raw(body: &str, key: &str) -> Option<String> {
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
    apply_patch(&mut lines, updates);
    let mut out = lines.join("\n");
    out.push('\n');
    if let Err(e) = std::fs::write(&path, out) {
        tracing::warn!(error = %e, "settings: failed to write config.conf");
    }
}

/// Pure in-place line patch (no IO) — see [`patch_conf`]. Each `key` is replaced
/// at its first non-comment occurrence, or appended if absent.
fn apply_patch(lines: &mut Vec<String>, updates: &[(&str, String)]) {
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

/// Does this line declare `<prefix> = …` (ignoring leading space + comments)?
fn line_is(prefix: &str, line: &str) -> bool {
    let t = line.trim_start();
    !t.starts_with('#')
        && t.strip_prefix(prefix)
            .is_some_and(|r| r.trim_start().starts_with('='))
}

/// Every `<prefix> = <payload>` payload in `config.conf`, in document order.
/// Used by the repeating-entry editors (`windowrule`, `monitorrule`).
pub(crate) fn read_block(prefix: &str) -> Vec<String> {
    block_payloads(
        &std::fs::read_to_string(conf_path()).unwrap_or_default(),
        prefix,
    )
}

/// Pure block payload extraction (no IO) — see [`read_block`].
fn block_payloads(body: &str, prefix: &str) -> Vec<String> {
    body.lines()
        .filter(|l| line_is(prefix, l))
        .filter_map(|l| {
            let rest = l.trim_start().strip_prefix(prefix)?.trim_start();
            Some(rest.strip_prefix('=')?.trim().to_string())
        })
        .collect()
}

/// Replace the whole `<prefix> = …` block with `payloads`, in place: removes
/// every existing `<prefix>` line and re-inserts the new set at the position of
/// the first one (or at end of file if there were none). Then `mctl reload`.
pub(crate) fn write_block(prefix: &str, payloads: &[String]) {
    let path = conf_path();
    let body = std::fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<String> = body.lines().map(|s| s.to_string()).collect();
    let mut text = rebuild_block(&lines, prefix, payloads).join("\n");
    text.push('\n');
    if let Err(e) = std::fs::write(&path, text) {
        tracing::warn!(error = %e, "settings: failed to write config.conf block");
        return;
    }
    reload();
}

/// Pure block rewrite (no IO) — see [`write_block`]. Removes every `<prefix>`
/// line and re-inserts `payloads` at the position of the first one (or appends).
fn rebuild_block(lines: &[String], prefix: &str, payloads: &[String]) -> Vec<String> {
    let insert_at = lines
        .iter()
        .position(|l| line_is(prefix, l))
        .unwrap_or(lines.len());

    let mut out: Vec<String> = Vec::with_capacity(lines.len() + payloads.len());
    for (i, line) in lines.iter().enumerate() {
        if i == insert_at {
            for p in payloads {
                out.push(format!("{prefix} = {p}"));
            }
        }
        if !line_is(prefix, line) {
            out.push(line.clone());
        }
    }
    if insert_at >= lines.len() {
        for p in payloads {
            out.push(format!("{prefix} = {p}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    #[test]
    fn parse_raw_handles_padding_and_inline_comments() {
        let body = "\
# a comment
borderpx = 4
border_radius   =   12   # rounded
#commented = 99
focused_opacity = 1.0";
        assert_eq!(parse_raw(body, "borderpx").as_deref(), Some("4"));
        // Padded `key   =   val   # comment` → trimmed value, comment stripped.
        assert_eq!(parse_raw(body, "border_radius").as_deref(), Some("12"));
        assert_eq!(parse_raw(body, "focused_opacity").as_deref(), Some("1.0"));
        // Commented-out keys are invisible.
        assert_eq!(parse_raw(body, "commented"), None);
        assert_eq!(parse_raw(body, "missing"), None);
    }

    #[test]
    fn apply_patch_replaces_first_match_in_place() {
        let mut l = lines("# header\nborderpx = 1\ngappih = 5");
        apply_patch(&mut l, &[("borderpx", "9".into())]);
        assert_eq!(l, lines("# header\nborderpx = 9\ngappih = 5"));
    }

    #[test]
    fn apply_patch_appends_when_absent() {
        let mut l = lines("gappih = 5");
        apply_patch(&mut l, &[("borderpx", "3".into())]);
        assert_eq!(l, lines("gappih = 5\nborderpx = 3"));
    }

    #[test]
    fn apply_patch_is_idempotent() {
        let mut l = lines("borderpx = 1");
        apply_patch(&mut l, &[("borderpx", "7".into())]);
        let once = l.clone();
        apply_patch(&mut l, &[("borderpx", "7".into())]);
        assert_eq!(
            l, once,
            "re-applying the same value must not change the file"
        );
    }

    #[test]
    fn block_payloads_collects_in_order() {
        let body = "\
windowrule = appid:foo, isfloating:1
# windowrule = appid:commented
gappih = 5
windowrule = title:bar, tags:2";
        assert_eq!(
            block_payloads(body, "windowrule"),
            vec![
                "appid:foo, isfloating:1".to_string(),
                "title:bar, tags:2".to_string()
            ]
        );
    }

    #[test]
    fn rebuild_block_replaces_set_at_first_position() {
        let l = lines("a = 1\nwindowrule = old1\nb = 2\nwindowrule = old2\nc = 3");
        let out = rebuild_block(&l, "windowrule", &["new1".into(), "new2".into()]);
        assert_eq!(
            out,
            lines("a = 1\nwindowrule = new1\nwindowrule = new2\nb = 2\nc = 3")
        );
    }

    #[test]
    fn rebuild_block_appends_when_none_present() {
        let l = lines("a = 1\nb = 2");
        let out = rebuild_block(&l, "windowrule", &["only".into()]);
        assert_eq!(out, lines("a = 1\nb = 2\nwindowrule = only"));
    }

    #[test]
    fn rebuild_block_clears_when_empty() {
        let l = lines("a = 1\nwindowrule = gone\nb = 2");
        let out = rebuild_block(&l, "windowrule", &[]);
        assert_eq!(out, lines("a = 1\nb = 2"));
    }
}
