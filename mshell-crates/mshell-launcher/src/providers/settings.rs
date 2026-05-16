//! Settings sidebar navigator.
//!
//! Indexes the nine top-level sections of the Settings panel
//! (general / bar / display / fonts / idle / menus / theme /
//! wallpaper / widgets) by name + a small keyword set. Activating
//! a result fires the caller-supplied open callback with the
//! section id; the UI wires that to the Settings frame menu so the
//! launcher hops straight to the right pane.

use crate::{item::LauncherItem, provider::Provider};
use std::rc::Rc;

/// Stable id matching the stack-child names baked into
/// `mshell-settings`. Adding a new section means appending here +
/// in [`SettingsSection::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    General,
    Bar,
    Display,
    Fonts,
    Idle,
    Launcher,
    Menus,
    Theme,
    Wallpaper,
    Widgets,
}

impl SettingsSection {
    /// Stack child name in the Settings panel — caller forwards
    /// this string to the Settings frame menu.
    pub fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Bar => "bar",
            Self::Display => "display",
            Self::Fonts => "fonts",
            Self::Idle => "idle",
            Self::Launcher => "launcher",
            Self::Menus => "menus",
            Self::Theme => "theme",
            Self::Wallpaper => "wallpaper",
            Self::Widgets => "widgets",
        }
    }

    /// Human-readable label shown in launcher results.
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Bar => "Bar",
            Self::Display => "Display",
            Self::Fonts => "Fonts",
            Self::Idle => "Idle",
            Self::Launcher => "Launcher",
            Self::Menus => "Menus",
            Self::Theme => "Theme",
            Self::Wallpaper => "Wallpaper",
            Self::Widgets => "Widgets",
        }
    }

    /// Symbolic icon — matches the sidebar icons already used by
    /// the Settings panel so the launcher result is visually
    /// consistent with the destination.
    pub fn icon(self) -> &'static str {
        match self {
            Self::General => "preferences-system-symbolic",
            Self::Bar => "view-dual-symbolic",
            Self::Display => "video-display-symbolic",
            Self::Fonts => "preferences-desktop-font-symbolic",
            Self::Idle => "preferences-desktop-screensaver-symbolic",
            Self::Launcher => "system-search-symbolic",
            Self::Menus => "view-more-symbolic",
            Self::Theme => "preferences-desktop-theme-symbolic",
            Self::Wallpaper => "preferences-desktop-wallpaper-symbolic",
            Self::Widgets => "view-grid-symbolic",
        }
    }

    /// Extra search keywords (synonyms) so common queries land in
    /// the right section even when the user doesn't know the
    /// canonical label.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::General => &["general", "options", "preferences"],
            Self::Bar => &["bar", "topbar", "panel", "pill", "pills"],
            Self::Display => &["display", "monitor", "screen", "layout", "twilight"],
            Self::Fonts => &["fonts", "font", "typography"],
            Self::Idle => &["idle", "screensaver", "lock", "timeout", "afk"],
            Self::Launcher => &[
                "launcher", "spotlight", "search", "frecency", "cache",
                "history", "scripts", "start",
            ],
            Self::Menus => &["menus", "menu", "popups"],
            Self::Theme => &["theme", "colors", "matugen", "scheme", "dark", "light"],
            Self::Wallpaper => &["wallpaper", "background", "rotation"],
            Self::Widgets => &["widgets", "widget", "pill", "extension"],
        }
    }

    /// Every section, in sidebar order.
    pub const ALL: &'static [SettingsSection] = &[
        Self::General,
        Self::Bar,
        Self::Display,
        Self::Fonts,
        Self::Idle,
        Self::Launcher,
        Self::Menus,
        Self::Theme,
        Self::Wallpaper,
        Self::Widgets,
    ];
}

/// Callback the UI provides — given a section id, open the
/// Settings panel and switch to that section's stack child.
pub type OpenSection = Rc<dyn Fn(&str) + 'static>;

pub struct SettingsProvider {
    open: OpenSection,
}

impl SettingsProvider {
    /// Create a provider that delegates section navigation to
    /// `open`. The UI typically wraps `FrameInput::OpenSettings` /
    /// `SettingsWindowInput::ActivateSection`.
    pub fn new(open: OpenSection) -> Self {
        Self { open }
    }
}

fn score_section(section: SettingsSection, query: &str) -> f64 {
    let q = query.to_ascii_lowercase();
    if q.is_empty() {
        return 0.0;
    }
    let label = section.label().to_ascii_lowercase();
    if label.starts_with(&q) {
        return 180.0;
    }
    let mut best: f64 = -1.0;
    if label.contains(&q) {
        best = best.max(130.0);
    }
    for kw in section.keywords() {
        if kw.starts_with(&q) {
            best = best.max(150.0);
        } else if kw.contains(&q) {
            best = best.max(90.0);
        }
    }
    best
}

impl Provider for SettingsProvider {
    fn name(&self) -> &str {
        "Settings"
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "settings:palette".into(),
            name: "Settings sections".into(),
            description: "Type display / theme / fonts / wallpaper / …".into(),
            icon: "preferences-system-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Settings".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim();
        if q.is_empty() {
            return Vec::new();
        }
        SettingsSection::ALL
            .iter()
            .filter_map(|section| {
                let score = score_section(*section, q);
                if score < 0.0 {
                    return None;
                }
                let open = self.open.clone();
                let id = section.id();
                Some(LauncherItem {
                    id: format!("settings:{id}"),
                    name: format!("Settings → {}", section.label()),
                    description: "Open settings section".into(),
                    icon: section.icon().into(),
                    icon_is_path: false,
                    score,
                    provider_name: "Settings".into(),
                    usage_key: Some(format!("settings:{id}")),
                    on_activate: Rc::new(move || open(id)),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn capturing_provider() -> (SettingsProvider, Rc<RefCell<Vec<String>>>) {
        let calls: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let calls_clone = calls.clone();
        let provider = SettingsProvider::new(Rc::new(move |id: &str| {
            calls_clone.borrow_mut().push(id.to_string());
        }));
        (provider, calls)
    }

    #[test]
    fn label_prefix_scores_top() {
        let (p, _) = capturing_provider();
        let items = p.search("dis");
        assert!(items.iter().any(|i| i.name == "Settings → Display"));
        // Label-prefix on the nucleo-comparable scale.
        assert!(items[0].score >= 150.0);
    }

    #[test]
    fn keyword_match_pulls_in_synonym() {
        let (p, _) = capturing_provider();
        let items = p.search("matugen");
        assert!(items.iter().any(|i| i.name == "Settings → Theme"));
    }

    #[test]
    fn empty_query_returns_empty() {
        let (p, _) = capturing_provider();
        assert!(p.search("").is_empty());
    }

    #[test]
    fn activation_invokes_callback_with_section_id() {
        let (p, calls) = capturing_provider();
        let items = p.search("display");
        let display = items.iter().find(|i| i.name == "Settings → Display").unwrap();
        (display.on_activate)();
        assert_eq!(calls.borrow().as_slice(), &["display".to_string()]);
    }
}
