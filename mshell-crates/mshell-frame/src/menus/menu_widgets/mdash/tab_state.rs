//! Which tab mdash shows: the remembered tab, keyboard switching and the
//! `mshellctl menu mdash <tab>` request. Pure decisions + the tiny state
//! file (`$XDG_STATE_HOME/margo/mdash-tab`, one token — state, not config).

use mshell_config::schema::dashboard::DashboardTab;
use relm4::gtk::gdk;
use std::path::{Path, PathBuf};

/// `$XDG_STATE_HOME/margo/mdash-tab`.
pub(crate) fn state_path() -> PathBuf {
    margo_fs::xdg::state_home().join("margo/mdash-tab")
}

/// The tab remembered in `path` (`None`: never written, unreadable or an
/// unknown token).
pub(crate) fn read_remembered(path: &Path) -> Option<DashboardTab> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| DashboardTab::from_token(&text))
}

/// Write `tab` to `path` (atomic; creates the directory).
pub(crate) fn write_remembered(path: &Path, tab: DashboardTab) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    margo_fs::atomic_write::atomic_write(path, format!("{}\n", tab.token()).as_bytes())
}

/// Remember `tab` for the next start, off the GTK thread. Tests never
/// write the real state dir.
pub(crate) fn remember(tab: DashboardTab) {
    if cfg!(test) {
        return;
    }
    std::thread::spawn(move || {
        let path = state_path();
        if let Err(e) = write_remembered(&path, tab) {
            tracing::debug!(error = %e, path = %path.display(), "mdash: couldn't remember the tab");
        }
    });
}

/// The tab to start on: the remembered one when it is shown, else the
/// first shown tab.
pub(crate) fn initial_index(tabs: &[DashboardTab], remembered: Option<DashboardTab>) -> usize {
    remembered
        .and_then(|r| tabs.iter().position(|t| *t == r))
        .unwrap_or(0)
}

/// The neighbouring tab, wrapping around.
pub(crate) fn cycle(len: usize, current: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

/// A tab-switching key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabKey {
    /// `1`…`9` (plain, or on the keypad): that tab, 0-based.
    Index(usize),
    /// Ctrl+Tab / Ctrl+Page Down.
    Next,
    /// Ctrl+Shift+Tab / Ctrl+Page Up.
    Prev,
}

/// What `key` does to the tabs, if anything. Digits only count without
/// Ctrl/Alt (Ctrl+1 stays free); the dashboard only sees a digit when no
/// text field took it.
pub(crate) fn tab_key(key: gdk::Key, state: gdk::ModifierType) -> Option<TabKey> {
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    let alt = state.contains(gdk::ModifierType::ALT_MASK);
    let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
    if ctrl {
        return match key {
            gdk::Key::Tab | gdk::Key::KP_Tab => {
                Some(if shift { TabKey::Prev } else { TabKey::Next })
            }
            // Shift+Tab arrives as ISO_Left_Tab.
            gdk::Key::ISO_Left_Tab => Some(TabKey::Prev),
            gdk::Key::Page_Down => Some(TabKey::Next),
            gdk::Key::Page_Up => Some(TabKey::Prev),
            _ => None,
        };
    }
    if alt {
        return None;
    }
    let digit = key
        .to_unicode()
        .and_then(|c| c.to_digit(10))
        .or(match key {
            gdk::Key::KP_1 => Some(1),
            gdk::Key::KP_2 => Some(2),
            gdk::Key::KP_3 => Some(3),
            gdk::Key::KP_4 => Some(4),
            gdk::Key::KP_5 => Some(5),
            gdk::Key::KP_6 => Some(6),
            gdk::Key::KP_7 => Some(7),
            gdk::Key::KP_8 => Some(8),
            gdk::Key::KP_9 => Some(9),
            _ => None,
        })?;
    (1..=9)
        .contains(&digit)
        .then(|| TabKey::Index(digit as usize - 1))
}

/// The tab index a key lands on among `len` tabs (`None`: not a tab key,
/// or a digit past the last tab).
pub(crate) fn apply_key(key: TabKey, len: usize, current: usize) -> Option<usize> {
    match key {
        TabKey::Index(i) => (i < len).then_some(i),
        TabKey::Next => (len > 1).then(|| cycle(len, current, true)),
        TabKey::Prev => (len > 1).then(|| cycle(len, current, false)),
    }
}

/// What `mshellctl menu mdash <tab>` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabRequest {
    /// Show this tab (opening mdash if it was closed).
    Show(usize),
    /// mdash was already open on that tab: the command toggles it shut.
    Close,
    /// The tab is hidden in Settings (or unknown here): just open.
    Ignore,
}

/// Decide a `mshellctl menu mdash <tab>` request: open on / switch to the
/// tab; asked again for the tab already on screen, close (a toggle like
/// every other `mshellctl menu` verb).
pub(crate) fn request(
    tabs: &[DashboardTab],
    current: usize,
    requested: DashboardTab,
    was_open: bool,
) -> TabRequest {
    match tabs.iter().position(|t| *t == requested) {
        Some(i) if was_open && i == current => TabRequest::Close,
        Some(i) => TabRequest::Show(i),
        None => TabRequest::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DashboardTab::*;

    const ALL: [DashboardTab; 4] = [Today, Controls, System, Tools];

    #[test]
    fn the_remembered_tab_survives_a_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("margo/mdash-tab");
        assert_eq!(read_remembered(&path), None, "never written");
        write_remembered(&path, System).expect("write");
        assert_eq!(read_remembered(&path), Some(System));
        write_remembered(&path, Tools).expect("overwrite");
        assert_eq!(read_remembered(&path), Some(Tools));
        std::fs::write(&path, "garbage").expect("corrupt");
        assert_eq!(read_remembered(&path), None);
    }

    #[test]
    fn start_on_the_remembered_tab_when_it_is_shown() {
        assert_eq!(initial_index(&ALL, Some(System)), 2);
        assert_eq!(initial_index(&ALL, None), 0);
        // Remembered tab hidden since: the first shown tab.
        assert_eq!(initial_index(&[Today, Tools], Some(System)), 0);
    }

    #[test]
    fn ctrl_tab_cycles_and_wraps() {
        assert_eq!(cycle(4, 3, true), 0);
        assert_eq!(cycle(4, 0, false), 3);
        assert_eq!(cycle(4, 1, true), 2);
        assert_eq!(cycle(0, 0, true), 0);
    }

    #[test]
    fn keys_map_to_tabs() {
        let none = gdk::ModifierType::empty();
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let ctrl_shift = ctrl | gdk::ModifierType::SHIFT_MASK;
        assert_eq!(tab_key(gdk::Key::_1, none), Some(TabKey::Index(0)));
        assert_eq!(tab_key(gdk::Key::_4, none), Some(TabKey::Index(3)));
        assert_eq!(tab_key(gdk::Key::KP_2, none), Some(TabKey::Index(1)));
        assert_eq!(tab_key(gdk::Key::_0, none), None);
        assert_eq!(tab_key(gdk::Key::_1, ctrl), None, "Ctrl+1 stays free");
        assert_eq!(tab_key(gdk::Key::a, none), None);
        assert_eq!(tab_key(gdk::Key::Tab, ctrl), Some(TabKey::Next));
        assert_eq!(tab_key(gdk::Key::Tab, ctrl_shift), Some(TabKey::Prev));
        assert_eq!(
            tab_key(gdk::Key::ISO_Left_Tab, ctrl_shift),
            Some(TabKey::Prev)
        );
        assert_eq!(tab_key(gdk::Key::Tab, none), None, "plain Tab moves focus");

        assert_eq!(apply_key(TabKey::Index(3), 4, 0), Some(3));
        assert_eq!(apply_key(TabKey::Index(3), 3, 0), None, "no fourth tab");
        assert_eq!(apply_key(TabKey::Prev, 4, 0), Some(3));
        assert_eq!(apply_key(TabKey::Next, 1, 0), None);
    }

    #[test]
    fn mshellctl_tab_requests() {
        // Closed → open on the tab.
        assert_eq!(request(&ALL, 0, System, false), TabRequest::Show(2));
        // Open on another tab → switch.
        assert_eq!(request(&ALL, 0, System, true), TabRequest::Show(2));
        // Open on that very tab → toggle shut.
        assert_eq!(request(&ALL, 2, System, true), TabRequest::Close);
        // A hidden tab → just open.
        assert_eq!(
            request(&[Today, Tools], 0, System, false),
            TabRequest::Ignore
        );
    }
}
