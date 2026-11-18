//! Which exported menu belongs to the focused window.
//!
//! Applications register a menu with the AppMenu registrar under an X11
//! window id, which Wayland does not expose. Like the reference daemon we
//! match by identity instead: first the registering process's pid against
//! the focused window's pid (margo reports it for Wayland and XWayland
//! clients), then the process's executable / argv[0] basename against the
//! window's app id, both normalised (lowercase, alphanumerics only), so
//! `org.telegram.desktop` ↔ `telegram-desktop` and `Code` ↔ `code` meet.
//! When several registrations fit, the newest window id wins.

/// One `RegisterWindow` call (or an external registrar's entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Registration {
    pub(crate) window_id: u32,
    pub(crate) sender: String,
    pub(crate) path: String,
    pub(crate) pid: Option<u32>,
    /// Normalised executable / argv[0] basenames of `pid`.
    pub(crate) identities: Vec<String>,
}

pub(crate) fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// Identity tokens from a process's `/proc/<pid>/exe` target and argv[0].
pub(crate) fn identities(exe: Option<&str>, argv0: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = [exe, argv0]
        .into_iter()
        .flatten()
        .map(|p| normalize(basename(p)))
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Tokens a window's app id may appear as: the whole id and its last
/// dot-separated segment.
fn wanted(app_id: &str) -> Vec<String> {
    let mut w = vec![normalize(app_id)];
    if let Some(last) = app_id.rsplit('.').next() {
        w.push(normalize(last));
    }
    w.retain(|s| !s.is_empty());
    w
}

/// The registration for the focused window (`app_id`, `pid`), if any.
pub(crate) fn pick<'a>(
    regs: &'a [Registration],
    app_id: &str,
    pid: Option<u32>,
) -> Option<&'a Registration> {
    let newest = |it: Vec<&'a Registration>| it.into_iter().max_by_key(|r| r.window_id);
    if let Some(pid) = pid.filter(|p| *p > 0)
        && let Some(r) = newest(regs.iter().filter(|r| r.pid == Some(pid)).collect())
    {
        return Some(r);
    }
    let want = wanted(app_id);
    if want.is_empty() {
        return None;
    }
    newest(
        regs.iter()
            .filter(|r| r.identities.iter().any(|i| want.contains(i)))
            .collect(),
    )
}

/// A menu label without its mnemonic underscores (`_File` → `File`,
/// `Save __As` → `Save _As`).
pub(crate) fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(id: u32, pid: Option<u32>, ids: &[&str]) -> Registration {
        Registration {
            window_id: id,
            sender: format!(":1.{id}"),
            path: "/MenuBar/1".into(),
            pid,
            identities: ids.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn identities_are_normalised_basenames() {
        assert_eq!(
            identities(
                Some("/usr/lib/telegram-desktop/Telegram"),
                Some("telegram-desktop")
            ),
            ["telegram", "telegramdesktop"]
        );
        assert!(identities(None, Some("")).is_empty());
    }

    #[test]
    fn pid_wins_then_identity_then_newest() {
        let regs = [
            reg(10, Some(100), &["kate"]),
            reg(11, Some(200), &["telegramdesktop"]),
            reg(12, Some(201), &["telegramdesktop"]),
        ];
        assert_eq!(
            pick(&regs, "whatever", Some(100)).map(|r| r.window_id),
            Some(10)
        );
        assert_eq!(
            pick(&regs, "org.telegram.desktop", Some(999)).map(|r| r.window_id),
            None,
            "neither the full id nor `desktop` names the process"
        );
        assert_eq!(
            pick(&regs, "telegram-desktop", None).map(|r| r.window_id),
            Some(12)
        );
        assert_eq!(
            pick(&regs, "org.kde.kate", None).map(|r| r.window_id),
            Some(10)
        );
        assert_eq!(pick(&regs, "", None), None);
        assert_eq!(pick(&[], "kate", Some(1)), None);
    }

    #[test]
    fn mnemonics_are_stripped() {
        assert_eq!(strip_mnemonic("_File"), "File");
        assert_eq!(strip_mnemonic("Save __As"), "Save _As");
        assert_eq!(strip_mnemonic("Plain"), "Plain");
    }
}
