//! `MenuWidget::Dashboard` — mdash's tabbed dashboard.
//!
//! The default `mdash_menu` is a single `Dashboard` widget: a header
//! (greeting, time, Settings + one power button), a status strip of small
//! chips, and four tabs — **Today**, **Controls**, **System**, **Tools** —
//! that share one panel size. The tab list (order + visibility) and the
//! cards shown on each tab are configurable here; the arrangement of the
//! cards inside a tab is fixed by the shell so every tab stays balanced
//! (a hidden card hands its room to its neighbour, never leaves a hole).
//!
//! `hidden_cards` (not a "shown" list) keeps the shape forward compatible:
//! a card added in a later release shows up without touching the profile.
//! Old custom `mdash_menu` layouts (Containers, QuickActions, …) are
//! untouched — they simply don't contain a `Dashboard`.

use reactive_stores::Store;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One dashboard tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, Store, JsonSchema)]
pub enum DashboardTab {
    /// Calendar + agenda, weather, media and timers.
    Today,
    /// Volume / mic / brightness sliders and every quick-settings tile.
    Controls,
    /// CPU, memory, temperature, disk, battery + power profile, network,
    /// pending updates and AI usage limits.
    System,
    /// A labelled launcher grid of the shell's menus.
    Tools,
}

impl DashboardTab {
    /// Every tab, in the default order.
    pub const ALL: [DashboardTab; 4] = [
        DashboardTab::Today,
        DashboardTab::Controls,
        DashboardTab::System,
        DashboardTab::Tools,
    ];

    /// The tab's label (segmented control, Settings, `mshellctl`).
    pub fn label(self) -> &'static str {
        match self {
            DashboardTab::Today => "Today",
            DashboardTab::Controls => "Controls",
            DashboardTab::System => "System",
            DashboardTab::Tools => "Tools",
        }
    }

    /// The lowercase token used by `mshellctl menu mdash <tab>` and the
    /// remembered-tab state file.
    pub fn token(self) -> &'static str {
        match self {
            DashboardTab::Today => "today",
            DashboardTab::Controls => "controls",
            DashboardTab::System => "system",
            DashboardTab::Tools => "tools",
        }
    }

    /// Parse a tab token (case-insensitive, surrounding blanks ignored).
    pub fn from_token(token: &str) -> Option<Self> {
        let t = token.trim();
        Self::ALL
            .into_iter()
            .find(|tab| tab.token().eq_ignore_ascii_case(t))
    }

    /// The cards that live on this tab, in display order.
    pub fn cards(self) -> &'static [DashboardCard] {
        use DashboardCard as C;
        match self {
            DashboardTab::Today => &[C::Calendar, C::Agenda, C::Weather, C::Media, C::Timers],
            DashboardTab::Controls => &[C::Sliders, C::Tiles],
            DashboardTab::System => &[
                C::Cpu,
                C::Memory,
                C::Temperature,
                C::Disk,
                C::Battery,
                C::Network,
                C::Updates,
                C::AiUsage,
            ],
            DashboardTab::Tools => &[C::Launchers],
        }
    }
}

/// One card on a dashboard tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, Store, JsonSchema)]
pub enum DashboardCard {
    // ── Today ──
    Calendar,
    Agenda,
    Weather,
    Media,
    Timers,
    // ── Controls ──
    Sliders,
    Tiles,
    // ── System ──
    Cpu,
    Memory,
    Temperature,
    Disk,
    /// Battery (when present) + the power-profile selector.
    Battery,
    Network,
    Updates,
    AiUsage,
    // ── Tools ──
    Launchers,
}

impl DashboardCard {
    pub fn label(self) -> &'static str {
        match self {
            DashboardCard::Calendar => "Calendar",
            DashboardCard::Agenda => "Agenda",
            DashboardCard::Weather => "Weather",
            DashboardCard::Media => "Media",
            DashboardCard::Timers => "Timers & alarms",
            DashboardCard::Sliders => "Sliders",
            DashboardCard::Tiles => "Quick settings",
            DashboardCard::Cpu => "CPU",
            DashboardCard::Memory => "Memory",
            DashboardCard::Temperature => "Temperature",
            DashboardCard::Disk => "Disk",
            DashboardCard::Battery => "Battery & power",
            DashboardCard::Network => "Network",
            DashboardCard::Updates => "Updates",
            DashboardCard::AiUsage => "AI usage",
            DashboardCard::Launchers => "Launchers",
        }
    }

    /// The tab this card belongs to.
    pub fn tab(self) -> DashboardTab {
        DashboardTab::ALL
            .into_iter()
            .find(|t| t.cards().contains(&self))
            .unwrap_or(DashboardTab::Today)
    }
}

/// One entry of the dashboard's tab list.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
pub struct DashboardTabConfig {
    pub tab: DashboardTab,
    /// Show this tab. A hidden tab keeps its place in the order.
    #[serde(default = "default_true")]
    pub visible: bool,
    /// Cards of this tab to leave out. Unknown / other-tab cards are
    /// ignored.
    #[serde(default)]
    pub hidden_cards: Vec<DashboardCard>,
}

fn default_true() -> bool {
    true
}

impl DashboardTabConfig {
    pub fn new(tab: DashboardTab) -> Self {
        Self {
            tab,
            visible: true,
            hidden_cards: Vec::new(),
        }
    }

    /// Whether `card` shows on this tab.
    pub fn shows(&self, card: DashboardCard) -> bool {
        card.tab() == self.tab && !self.hidden_cards.contains(&card)
    }
}

/// `MenuWidget::Dashboard` configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, JsonSchema)]
#[serde(default)]
pub struct DashboardConfig {
    /// Tab order + visibility + per-tab hidden cards.
    pub tabs: Vec<DashboardTabConfig>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            tabs: DashboardTab::ALL
                .into_iter()
                .map(DashboardTabConfig::new)
                .collect(),
        }
    }
}

impl DashboardConfig {
    /// The visible tabs in their configured order: each tab once (first
    /// entry wins), tabs missing from the list appended in default order
    /// (so a tab added in a later release appears), and never empty — with
    /// every tab hidden, Today still shows.
    pub fn visible_tabs(&self) -> Vec<DashboardTabConfig> {
        let mut seen: Vec<DashboardTab> = Vec::new();
        let mut out = Vec::new();
        for entry in &self.tabs {
            if seen.contains(&entry.tab) {
                continue;
            }
            seen.push(entry.tab);
            if entry.visible {
                out.push(entry.clone());
            }
        }
        for tab in DashboardTab::ALL {
            if !seen.contains(&tab) {
                out.push(DashboardTabConfig::new(tab));
            }
        }
        if out.is_empty() {
            out.push(DashboardTabConfig::new(DashboardTab::Today));
        }
        out
    }

    /// The configured entry for `tab` (a default one when absent).
    pub fn tab(&self, tab: DashboardTab) -> DashboardTabConfig {
        self.tabs
            .iter()
            .find(|e| e.tab == tab)
            .cloned()
            .unwrap_or_else(|| DashboardTabConfig::new(tab))
    }
}

/// Settings edits. Each first normalises the list (every tab exactly
/// once, configured order kept, missing tabs appended) so the editor
/// always works on all four tabs.
impl DashboardConfig {
    /// Every tab once, in the configured order.
    pub fn normalised(&self) -> Vec<DashboardTabConfig> {
        let mut out: Vec<DashboardTabConfig> = Vec::new();
        for entry in &self.tabs {
            if !out.iter().any(|e| e.tab == entry.tab) {
                out.push(entry.clone());
            }
        }
        for tab in DashboardTab::ALL {
            if !out.iter().any(|e| e.tab == tab) {
                out.push(DashboardTabConfig::new(tab));
            }
        }
        out
    }

    /// Move `tab` one place up (`-1`) or down (`+1`); no-op at an end.
    pub fn move_tab(&mut self, tab: DashboardTab, delta: i32) {
        let mut tabs = self.normalised();
        if let Some(i) = tabs.iter().position(|e| e.tab == tab) {
            let j = i as i64 + i64::from(delta);
            if (0..tabs.len() as i64).contains(&j) {
                tabs.swap(i, j as usize);
            }
        }
        self.tabs = tabs;
    }

    /// Show / hide a tab. Hiding the last shown tab is refused (the
    /// dashboard would fall back to Today anyway; the switch says so).
    pub fn set_tab_visible(&mut self, tab: DashboardTab, visible: bool) -> bool {
        let mut tabs = self.normalised();
        if !visible && tabs.iter().filter(|e| e.visible && e.tab != tab).count() == 0 {
            return false;
        }
        if let Some(e) = tabs.iter_mut().find(|e| e.tab == tab) {
            e.visible = visible;
        }
        self.tabs = tabs;
        true
    }

    /// Show / hide one card on its own tab.
    pub fn set_card_shown(&mut self, card: DashboardCard, shown: bool) {
        let mut tabs = self.normalised();
        if let Some(e) = tabs.iter_mut().find(|e| e.tab == card.tab()) {
            e.hidden_cards.retain(|c| *c != card);
            if !shown {
                e.hidden_cards.push(card);
            }
        }
        self.tabs = tabs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_tokens_round_trip_case_insensitively() {
        for tab in DashboardTab::ALL {
            assert_eq!(DashboardTab::from_token(tab.token()), Some(tab));
            assert_eq!(
                DashboardTab::from_token(&tab.label().to_uppercase()),
                Some(tab)
            );
        }
        assert_eq!(
            DashboardTab::from_token("  system "),
            Some(DashboardTab::System)
        );
        assert_eq!(DashboardTab::from_token("media"), None);
        assert_eq!(DashboardTab::from_token(""), None);
    }

    #[test]
    fn every_card_belongs_to_exactly_one_tab() {
        let mut all: Vec<DashboardCard> = Vec::new();
        for tab in DashboardTab::ALL {
            for card in tab.cards() {
                assert_eq!(card.tab(), tab, "{card:?}");
                assert!(!all.contains(card), "{card:?} listed twice");
                all.push(*card);
            }
        }
        assert_eq!(all.len(), 16);
    }

    #[test]
    fn visible_tabs_keep_order_drop_hidden_and_append_missing() {
        let cfg = DashboardConfig {
            tabs: vec![
                DashboardTabConfig::new(DashboardTab::Tools),
                DashboardTabConfig {
                    visible: false,
                    ..DashboardTabConfig::new(DashboardTab::Controls)
                },
                DashboardTabConfig::new(DashboardTab::Tools),
                DashboardTabConfig::new(DashboardTab::Today),
            ],
        };
        let tabs: Vec<DashboardTab> = cfg.visible_tabs().into_iter().map(|t| t.tab).collect();
        assert_eq!(
            tabs,
            [
                DashboardTab::Tools,
                DashboardTab::Today,
                DashboardTab::System
            ]
        );
    }

    #[test]
    fn all_hidden_still_shows_today() {
        let cfg = DashboardConfig {
            tabs: DashboardTab::ALL
                .into_iter()
                .map(|t| DashboardTabConfig {
                    visible: false,
                    ..DashboardTabConfig::new(t)
                })
                .collect(),
        };
        let tabs: Vec<DashboardTab> = cfg.visible_tabs().into_iter().map(|t| t.tab).collect();
        assert_eq!(tabs, [DashboardTab::Today]);
    }

    #[test]
    fn hidden_cards_only_affect_their_own_tab() {
        let today = DashboardTabConfig {
            hidden_cards: vec![DashboardCard::Media, DashboardCard::Cpu],
            ..DashboardTabConfig::new(DashboardTab::Today)
        };
        assert!(!today.shows(DashboardCard::Media));
        assert!(today.shows(DashboardCard::Weather));
        // A card of another tab never shows on Today, hidden or not.
        assert!(!today.shows(DashboardCard::Cpu));
        assert!(!today.shows(DashboardCard::Tiles));
    }

    #[test]
    fn partial_yaml_parses_with_defaults() {
        let cfg: DashboardConfig =
            yaml_serde::from_str("tabs:\n  - tab: System\n  - tab: Today\n    visible: false\n")
                .expect("partial dashboard config parses");
        assert_eq!(cfg.tabs[0], DashboardTabConfig::new(DashboardTab::System));
        assert!(!cfg.tabs[1].visible);
        assert!(cfg.tabs[1].hidden_cards.is_empty());
        let empty: DashboardConfig = yaml_serde::from_str("{}").expect("empty parses");
        assert_eq!(empty, DashboardConfig::default());
    }

    #[test]
    fn moving_a_tab_swaps_neighbours_and_stops_at_the_ends() {
        let mut cfg = DashboardConfig::default();
        cfg.move_tab(DashboardTab::System, -1);
        let order: Vec<DashboardTab> = cfg.tabs.iter().map(|e| e.tab).collect();
        assert_eq!(
            order,
            [
                DashboardTab::Today,
                DashboardTab::System,
                DashboardTab::Controls,
                DashboardTab::Tools
            ]
        );
        let before = cfg.clone();
        cfg.move_tab(DashboardTab::Today, -1);
        cfg.move_tab(DashboardTab::Tools, 1);
        assert_eq!(cfg, before);
    }

    #[test]
    fn edits_normalise_a_sparse_list() {
        let mut cfg = DashboardConfig {
            tabs: vec![DashboardTabConfig::new(DashboardTab::Tools)],
        };
        cfg.set_card_shown(DashboardCard::Media, false);
        let order: Vec<DashboardTab> = cfg.tabs.iter().map(|e| e.tab).collect();
        assert_eq!(
            order,
            [
                DashboardTab::Tools,
                DashboardTab::Today,
                DashboardTab::Controls,
                DashboardTab::System
            ]
        );
        assert!(!cfg.tab(DashboardTab::Today).shows(DashboardCard::Media));
        cfg.set_card_shown(DashboardCard::Media, true);
        cfg.set_card_shown(DashboardCard::Media, true);
        assert!(cfg.tab(DashboardTab::Today).hidden_cards.is_empty());
    }

    #[test]
    fn the_last_shown_tab_cannot_be_hidden() {
        let mut cfg = DashboardConfig::default();
        for tab in [
            DashboardTab::Today,
            DashboardTab::Controls,
            DashboardTab::System,
        ] {
            assert!(cfg.set_tab_visible(tab, false));
        }
        assert!(!cfg.set_tab_visible(DashboardTab::Tools, false));
        assert!(cfg.tab(DashboardTab::Tools).visible);
        assert!(cfg.set_tab_visible(DashboardTab::Today, true));
        assert!(cfg.set_tab_visible(DashboardTab::Tools, false));
    }
}
