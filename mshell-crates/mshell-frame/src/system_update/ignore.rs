//! Ignored / hidden packages.
//!
//! Two lists, shown together in the panel's "Hidden" section:
//! - `IgnorePkg` from `/etc/pacman.conf` — pacman never upgrades these
//!   (checkupdates drops them already); read-only here, the panel
//!   never edits pacman.conf.
//! - `bars.widgets.system_update.hidden_packages` — names the user
//!   hid from the count in the panel. They still upgrade with the rest
//!   (holding one back would be a partial upgrade).

use super::{UpdateEntry, UpdateReport};

pub(crate) const PACMAN_CONF: &str = "/etc/pacman.conf";

/// Every `IgnorePkg` name in the `[options]` section (the directive
/// may repeat; values are whitespace-separated; `#` starts a comment).
pub(crate) fn parse_ignore_pkg(conf: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_options = false;
    for raw in conf.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_options = section == "options";
            continue;
        }
        if !in_options {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "IgnorePkg" {
            for name in value.split_whitespace() {
                if !out.iter().any(|n| n == name) {
                    out.push(name.to_string());
                }
            }
        }
    }
    out
}

pub(crate) fn pacman_ignored() -> Vec<String> {
    std::fs::read_to_string(PACMAN_CONF)
        .map(|c| parse_ignore_pkg(&c))
        .unwrap_or_default()
}

/// Arch package names are `[a-z0-9@._+-]`, not starting with `-` or
/// `.`. Anything else is refused before it reaches config or a shell.
pub(crate) fn valid_pkg_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c))
}

/// Move entries whose name is in `hidden` out of the count.
pub(crate) fn apply_hidden(report: &mut UpdateReport, hidden: &[String]) {
    let (keep, hide): (Vec<UpdateEntry>, Vec<UpdateEntry>) = std::mem::take(&mut report.entries)
        .into_iter()
        .partition(|e| !hidden.contains(&e.name));
    report.entries = keep;
    report.hidden = hide;
}

/// `hidden` with `name` toggled (added when absent, removed when
/// present), keeping the order stable.
pub(crate) fn toggled(hidden: &[String], name: &str) -> Vec<String> {
    if hidden.iter().any(|h| h == name) {
        hidden.iter().filter(|h| *h != name).cloned().collect()
    } else {
        let mut v = hidden.to_vec();
        v.push(name.to_string());
        v
    }
}

#[cfg(test)]
mod tests {
    use super::super::Source;
    use super::*;

    const CONF: &str = include_str!("testdata/pacman.conf");

    #[test]
    fn reads_ignore_pkg_from_this_machines_pacman_conf() {
        assert_eq!(
            parse_ignore_pkg(CONF),
            ["gnome-monitor-config-git", "margo-git"]
        );
    }

    #[test]
    fn ignore_pkg_edge_cases() {
        let conf = "#IgnorePkg = commented\n[options]\nIgnorePkg = a b # trailing\n\
                    IgnorePkg=c\nIgnorePkg = a\n#IgnorePkg = d\n[core]\nIgnorePkg = repo-scoped\n";
        assert_eq!(parse_ignore_pkg(conf), ["a", "b", "c"]);
        assert!(parse_ignore_pkg("").is_empty());
    }

    fn e(name: &str) -> UpdateEntry {
        UpdateEntry {
            source: Source::Repo,
            name: name.into(),
            old_version: None,
            new_version: None,
        }
    }

    #[test]
    fn hidden_entries_leave_the_count() {
        let mut r = UpdateReport {
            entries: vec![e("firefox"), e("electron43")],
            ..Default::default()
        };
        apply_hidden(&mut r, &["electron43".into()]);
        assert_eq!(r.total(), 1);
        assert_eq!(r.entries[0].name, "firefox");
        assert_eq!(r.hidden[0].name, "electron43");
    }

    #[test]
    fn package_names() {
        assert!(valid_pkg_name("lib32-gcc-libs"));
        assert!(valid_pkg_name("gtk+3"));
        assert!(!valid_pkg_name("a b"));
        assert!(!valid_pkg_name("-Rns"));
        assert!(!valid_pkg_name(""));
    }

    #[test]
    fn toggling() {
        let h = toggled(&[], "a");
        assert_eq!(h, ["a"]);
        let h = toggled(&h, "b");
        assert_eq!(h, ["a", "b"]);
        assert_eq!(toggled(&h, "a"), ["b"]);
    }
}
