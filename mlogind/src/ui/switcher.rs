use crossterm::event::KeyCode;
use log::warn;

use crate::config::{SwitcherConfig, SwitcherVisibility};

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct SwitcherItem<T> {
    pub title: String,
    pub content: T,
}

#[derive(Debug, Clone)]
struct Switcher<T> {
    selected: Option<usize>,
    items: Vec<SwitcherItem<T>>,
}

/// A widget used to select a specific window manager / session.
///
/// The selection logic (a small carousel) lives here; the *rendering* of the
/// chosen session is done by the greeter itself (`ui/mod.rs`), drawn inline
/// in the credential card so it stays readable and never clips at any
/// terminal width — see `current_title` / `has_prev` / `has_next`.
#[derive(Clone)]
pub struct SwitcherWidget<T> {
    selector: Switcher<T>,
    config: SwitcherConfig,
    /// Indicates whether the widget has been hidden by the config or keybind
    hidden: bool,
}

impl<T> SwitcherItem<T> {
    pub fn new(title: impl ToString, content: T) -> Self {
        let title = title.to_string();
        Self { title, content }
    }
}

impl<T> Switcher<T> {
    fn new(items: Vec<SwitcherItem<T>>) -> Self {
        let selected = if items.is_empty() { None } else { Some(0) };
        Self { selected, items }
    }

    #[inline]
    fn len(&self) -> usize {
        self.items.len()
    }

    pub fn try_select(&mut self, title: &str) {
        // Only set the selected if we find a matching title
        if let Some(selected) = self
            .items
            .iter()
            .enumerate()
            .find(|(_, item)| item.title == title)
            .map(|(index, _)| index)
        {
            self.selected = Some(selected);
        } else {
            warn!("Failed to find selection with title: '{}'", title);
        }
    }

    fn next_index(&self, index: usize) -> Option<usize> {
        let next_index = index + 1;

        if next_index == self.len() {
            None
        } else {
            Some(next_index)
        }
    }

    fn prev_index(&self, index: usize) -> Option<usize> {
        if index == 0 {
            return None;
        }

        Some(index - 1)
    }

    fn go_next(&mut self) {
        match self.selected.map(|index| self.next_index(index)) {
            None | Some(None) => {}
            Some(val) => self.selected = val,
        }
    }

    fn go_prev(&mut self) {
        match self.selected.map(|index| self.prev_index(index)) {
            None | Some(None) => {}
            Some(val) => self.selected = val,
        }
    }

    fn has_next(&self) -> bool {
        self.selected
            .and_then(|index| self.next_index(index))
            .is_some()
    }

    fn has_prev(&self) -> bool {
        self.selected
            .and_then(|index| self.prev_index(index))
            .is_some()
    }

    pub fn current(&self) -> Option<&SwitcherItem<T>> {
        self.selected.and_then(|index| {
            debug_assert!(self.len() > 0);
            self.items.get(index)
        })
    }
}

impl<T> SwitcherWidget<T> {
    pub fn new(items: Vec<SwitcherItem<T>>, config: SwitcherConfig) -> Self {
        // Always hidden by default unless explicitly stated to be visible
        let hidden = config.switcher_visibility != SwitcherVisibility::Visible;
        Self {
            selector: Switcher::new(items),
            config,
            hidden,
        }
    }

    pub fn try_select(&mut self, title: &str) {
        self.selector.try_select(title)
    }

    fn left(&mut self) {
        self.selector.go_prev();
    }

    fn right(&mut self) {
        self.selector.go_next();
    }

    pub fn hidden(&self) -> bool {
        self.hidden
    }

    /// Title of the currently selected session, if any.
    pub fn current_title(&self) -> Option<&str> {
        self.selector.current().map(|item| item.title.as_str())
    }

    /// Whether there is a session before / after the current one — drives the
    /// `‹ ›` arrows the greeter draws around the session name.
    pub fn has_prev(&self) -> bool {
        self.selector.has_prev()
    }
    pub fn has_next(&self) -> bool {
        self.selector.has_next()
    }

    /// The configured "no sessions found" placeholder text.
    pub fn no_envs_text(&self) -> &str {
        &self.config.no_envs_text
    }

    pub(crate) fn key_press(&mut self, key_code: KeyCode) -> Option<super::ErrorStatusMessage> {
        match key_code {
            KeyCode::Left | KeyCode::Char('h') => {
                self.left();
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.right();
            }
            kc if self.config.switcher_visibility == SwitcherVisibility::Keybind(kc) => {
                self.hidden ^= true;
            }
            _ => {}
        }

        None
    }

    pub fn selected(&self) -> Option<&SwitcherItem<T>> {
        let Self { selector, .. } = &self;
        selector.current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod window_manager_selector {
        use super::*;
        #[test]
        fn empty_creation() {
            // On an empty selector the go_next and go_prev should do nothing.

            let mut selector: Switcher<()> = Switcher::new(vec![]);
            assert_eq!(selector.current(), None);
            selector.go_next();
            assert_eq!(selector.current(), None);
            selector.go_prev();
            assert_eq!(selector.current(), None);

            let mut selector: Switcher<()> = Switcher::new(vec![]);
            assert_eq!(selector.current(), None);
            selector.go_prev();
            assert_eq!(selector.current(), None);
            selector.go_next();
            assert_eq!(selector.current(), None);
        }

        #[test]
        fn single_creation() {
            let wm: SwitcherItem<String> = SwitcherItem::new("abc", "/abc".into());

            let mut selector = Switcher::new(vec![wm.clone()]);
            assert_eq!(selector.current(), Some(&wm));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm));
            selector.go_prev();
            assert_eq!(selector.current(), Some(&wm));

            let mut selector = Switcher::new(vec![wm.clone()]);
            assert_eq!(selector.current(), Some(&wm));
            selector.go_prev();
            assert_eq!(selector.current(), Some(&wm));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm));

            let mut selector = Switcher::new(vec![wm.clone()]);
            assert_eq!(selector.current(), Some(&wm));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm));

            let mut selector = Switcher::new(vec![wm.clone()]);
            assert_eq!(selector.current(), Some(&wm));
            selector.go_prev();
            assert_eq!(selector.current(), Some(&wm));
            selector.go_prev();
            assert_eq!(selector.current(), Some(&wm));
        }

        #[test]
        fn multiple_creation() {
            let wm1: SwitcherItem<String> = SwitcherItem::new("abc", "/abc".into());
            let wm2 = SwitcherItem::new("def", "/def".into());

            let mut selector = Switcher::new(vec![wm1.clone(), wm2.clone()]);
            assert_eq!(selector.current(), Some(&wm1));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm2));
            selector.go_prev();
            assert_eq!(selector.current(), Some(&wm1));

            let mut selector = Switcher::new(vec![wm1.clone(), wm2.clone()]);
            assert_eq!(selector.current(), Some(&wm1));
            selector.go_prev();
            assert_eq!(selector.current(), Some(&wm1));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm2));

            let mut selector = Switcher::new(vec![wm1.clone(), wm2.clone()]);
            assert_eq!(selector.current(), Some(&wm1));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm2));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm2));

            let mut selector = Switcher::new(vec![wm1.clone(), wm2.clone()]);
            assert_eq!(selector.current(), Some(&wm1));
            selector.go_prev();
            assert_eq!(selector.current(), Some(&wm1));

            let wm3 = SwitcherItem::new("ghi", "/ghi".into());
            let wm4 = SwitcherItem::new("jkl", "/jkl".into());

            let mut selector =
                Switcher::new(vec![wm1.clone(), wm2.clone(), wm3.clone(), wm4.clone()]);
            assert_eq!(selector.current(), Some(&wm1));
            selector.go_prev();
            assert_eq!(selector.current(), Some(&wm1));

            let mut selector =
                Switcher::new(vec![wm1.clone(), wm2.clone(), wm3.clone(), wm4.clone()]);
            assert_eq!(selector.current(), Some(&wm1));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm2));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm3));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm4));
            selector.go_next();
            assert_eq!(selector.current(), Some(&wm4));
        }
    }
}
