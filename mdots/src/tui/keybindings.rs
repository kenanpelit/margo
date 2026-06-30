//! Single source of truth for TUI keybindings.
//!
//! Both the per-screen statusbar footer (`components::statusbar`) and the
//! full help overlay (`components::help`, toggled by `?`) read their key
//! lists from here, so the two views can never drift apart from each other
//! or from what the key handlers in `app.rs` / `screens/*.rs` actually do.

use crate::tui::screens::Screen;

/// One key plus a short description of what it does, e.g. `[r] refresh`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyHint {
    pub key: &'static str,
    pub desc: &'static str,
}

impl KeyHint {
    const fn new(key: &'static str, desc: &'static str) -> Self {
        Self { key, desc }
    }
}

/// Keybindings that work on every screen, regardless of mode.
pub const GLOBAL_HINTS: &[KeyHint] = &[
    KeyHint::new("?", "toggle this help"),
    KeyHint::new("q", "quit"),
    KeyHint::new("m", "toggle sidebar"),
    KeyHint::new("Esc", "back / close"),
];

/// Keybindings that only apply while the sidebar is expanded and focused.
pub const SIDEBAR_HINTS: &[KeyHint] = &[
    KeyHint::new("Tab / Shift+Tab", "select section"),
    KeyHint::new("Enter", "open section"),
];

/// Keybindings active while a screen's `/` filter text field has focus.
pub const FILTER_HINTS: &[KeyHint] = &[
    KeyHint::new("type", "filter text"),
    KeyHint::new("Enter / Esc", "close filter"),
    KeyHint::new("Backspace", "delete character"),
];

const OVERVIEW_HINTS: &[KeyHint] = &[
    KeyHint::new("j/k, ↓/↑", "scroll config tree"),
    KeyHint::new("r", "refresh"),
];

const MODULES_HINTS: &[KeyHint] = &[
    KeyHint::new("j/k, ↓/↑", "navigate"),
    KeyHint::new("space/Enter", "toggle enable"),
    KeyHint::new("/", "filter"),
    KeyHint::new("r", "refresh"),
];

const PACKAGES_HINTS: &[KeyHint] = &[
    KeyHint::new("j/k, ↓/↑", "navigate"),
    KeyHint::new("/", "filter"),
    KeyHint::new("r", "refresh"),
];

const SYNC_HINTS: &[KeyHint] = &[
    KeyHint::new("j/k, ↓/↑", "scroll plan"),
    KeyHint::new("s/Enter", "run sync"),
    KeyHint::new("r", "refresh"),
];

const SERVICES_HINTS: &[KeyHint] = &[
    KeyHint::new("j/k, ↓/↑", "navigate"),
    KeyHint::new("space/Enter", "toggle enable"),
    KeyHint::new("r", "refresh"),
];

const SECRETS_HINTS: &[KeyHint] = &[
    KeyHint::new("j/k, ↓/↑", "navigate"),
    KeyHint::new("e/Enter", "edit (sops)"),
    KeyHint::new("s", "sync secrets"),
    KeyHint::new("r", "refresh"),
];

/// Context keybindings for a screen's normal mode (i.e. not while a filter
/// text field is capturing raw characters — see [`FILTER_HINTS`] for that).
pub fn screen_hints(screen: &Screen) -> &'static [KeyHint] {
    match screen {
        Screen::Overview(_) => OVERVIEW_HINTS,
        Screen::Modules(_) => MODULES_HINTS,
        Screen::Packages(_) => PACKAGES_HINTS,
        Screen::Sync(_) => SYNC_HINTS,
        Screen::Services(_) => SERVICES_HINTS,
        Screen::Secrets(_) => SECRETS_HINTS,
    }
}

/// Short, one-line hint set for the statusbar footer.
///
/// While a filter field is focused the footer shows only the
/// [`FILTER_HINTS`] (Enter/Esc/Backspace): `?` is a valid filter character
/// in that state — it types into the query rather than opening help — and
/// `q` likewise types `q`, so neither may be advertised as a shortcut there.
/// Otherwise it shows the screen's context keys plus the two globals that
/// are always reachable (`?` help, `q` quit).
pub fn footer_hints(screen: &Screen) -> Vec<KeyHint> {
    if screen.is_filtering() {
        return FILTER_HINTS.to_vec();
    }
    let mut hints: Vec<KeyHint> = screen_hints(screen).to_vec();
    hints.push(KeyHint::new("?", "help"));
    hints.push(KeyHint::new("q", "quit"));
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::screens::{ModulesScreenState, OverviewScreenState, Screen};

    #[test]
    fn footer_hints_normal_mode_ends_with_help_and_quit() {
        let screen = Screen::Overview(OverviewScreenState::default());
        let hints = footer_hints(&screen);
        assert_eq!(hints.last(), Some(&KeyHint::new("q", "quit")));
        assert_eq!(hints[hints.len() - 2], KeyHint::new("?", "help"));
        // And it carries the screen's own context keys too.
        assert!(hints.iter().any(|h| h.key == "r"));
    }

    #[test]
    fn footer_hints_differ_per_screen() {
        let overview = footer_hints(&Screen::Overview(OverviewScreenState::default()));
        let modules = footer_hints(&Screen::Modules(ModulesScreenState::default()));
        assert_ne!(overview, modules);
        assert!(modules.iter().any(|h| h.key == "/"));
        assert!(!overview.iter().any(|h| h.key == "/"));
    }

    #[test]
    fn footer_hints_switch_to_filter_hints_while_filtering() {
        let mut state = ModulesScreenState::default();
        // Drive through the public ScreenTrait surface rather than poking
        // private fields, so this test tracks real key-handling behaviour.
        use crate::tui::screens::ScreenTrait;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let _ = state.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));

        let screen = Screen::Modules(state);
        assert!(screen.is_filtering());
        let hints = footer_hints(&screen);
        assert!(hints.iter().any(|h| h.key == "Enter / Esc"));
        assert!(!hints.iter().any(|h| h.key == "/"));
        // `?` and `q` are valid filter characters while filtering, so the
        // footer must NOT advertise them as shortcuts in that state — only
        // the FILTER_HINTS are authoritative here.
        assert!(!hints.iter().any(|h| h.key == "?"));
        assert!(!hints.iter().any(|h| h.key == "q"));
        assert_eq!(hints, FILTER_HINTS.to_vec());
    }

    #[test]
    fn global_and_sidebar_hint_lists_are_non_empty() {
        assert!(!GLOBAL_HINTS.is_empty());
        assert!(!SIDEBAR_HINTS.is_empty());
    }
}
