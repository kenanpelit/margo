//! Margo-native quick actions exposed through the launcher.
//!
//! Every entry here is a one-shot `mctl` / `mshellctl` /
//! `mscreenshot` invocation that the user would otherwise have to
//! type into a terminal or rig up as a keybind. Surfacing them in
//! the launcher means a single search box drives the whole
//! compositor without remembering CLI flags.
//!
//! The action list is hardcoded — these are the verbs users
//! actually run dozens of times a day (toggle night mode, cycle
//! wallpaper, swap layout). Discovering them dynamically from
//! `mctl actions` would surface every internal dispatch even ones
//! that take arguments you can't type into a search box.
//!
//! ## Coverage (Phase 2.1)
//!
//! | Group | Actions |
//! |---|---|
//! | Twilight | toggle · reset · day (6500K) · evening (4500K) · night (3500K) · midnight (2700K) |
//! | Wallpaper | next · previous · random |
//! | Layout | switch to each of the 14 named layouts (tile / scroller / grid / monocle / deck / center_tile / right_tile / vertical_scroller / vertical_tile / vertical_grid / vertical_deck / tgmix / canvas / dwindle) |
//! | Screenshot | region · full · window |
//! | Compositor | reload config |

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::process::Command;
use std::rc::Rc;

/// One quick-action: how to label / search / launch it.
#[derive(Debug, Clone)]
struct McAction {
    /// Stable, namespaced id used for the LauncherItem id +
    /// frecency key.
    id: &'static str,
    label: &'static str,
    description: &'static str,
    icon: &'static str,
    /// Extra search keywords. Always lowercase — match_score
    /// lowercases the query and compares directly.
    keywords: &'static [&'static str],
    /// Command vector to spawn. First element is the binary name.
    command: &'static [&'static str],
}

/// The hardcoded action list. Order is roughly "most likely to
/// be reached for" — though the launcher's frecency boost will
/// reshuffle once usage builds up.
const ACTIONS: &[McAction] = &[
    // ── Twilight ──────────────────────────────────────────
    McAction {
        id: "mctl:twilight:toggle",
        label: "Twilight: toggle",
        description: "Turn the schedule on or off",
        icon: "weather-clear-night-symbolic",
        keywords: &["twilight", "toggle", "night", "blue", "filter"],
        command: &["mctl", "twilight", "toggle"],
    },
    McAction {
        id: "mctl:twilight:reset",
        label: "Twilight: reset",
        description: "Clear preview/test override, resume schedule",
        icon: "view-refresh-symbolic",
        keywords: &["twilight", "reset", "clear", "schedule"],
        command: &["mctl", "twilight", "reset"],
    },
    McAction {
        id: "mctl:twilight:day",
        label: "Twilight: day (6500K)",
        description: "Preview neutral daylight temperature",
        icon: "weather-clear-symbolic",
        keywords: &["twilight", "day", "daylight", "cool", "6500"],
        command: &["mctl", "twilight", "preview", "6500"],
    },
    McAction {
        id: "mctl:twilight:evening",
        label: "Twilight: evening (4500K)",
        description: "Preview soft-evening temperature",
        icon: "weather-few-clouds-symbolic",
        keywords: &["twilight", "evening", "warm", "4500", "sunset"],
        command: &["mctl", "twilight", "preview", "4500"],
    },
    McAction {
        id: "mctl:twilight:night",
        label: "Twilight: night (3500K)",
        description: "Preview warm night temperature",
        icon: "weather-clear-night-symbolic",
        keywords: &["twilight", "night", "warm", "3500"],
        command: &["mctl", "twilight", "preview", "3500"],
    },
    McAction {
        id: "mctl:twilight:midnight",
        label: "Twilight: midnight (2700K)",
        description: "Preview deep-night temperature",
        icon: "weather-clear-night-symbolic",
        keywords: &["twilight", "midnight", "deep", "warm", "2700"],
        command: &["mctl", "twilight", "preview", "2700"],
    },
    // ── Wallpaper ─────────────────────────────────────────
    McAction {
        id: "mctl:wallpaper:next",
        label: "Wallpaper: next",
        description: "Switch to the next wallpaper in the directory",
        icon: "go-next-symbolic",
        keywords: &["wallpaper", "wp", "next", "cycle"],
        command: &["mshellctl", "wallpaper", "next"],
    },
    McAction {
        id: "mctl:wallpaper:prev",
        label: "Wallpaper: previous",
        description: "Switch to the previous wallpaper",
        icon: "go-previous-symbolic",
        keywords: &["wallpaper", "wp", "prev", "previous", "back"],
        command: &["mshellctl", "wallpaper", "prev"],
    },
    McAction {
        id: "mctl:wallpaper:random",
        label: "Wallpaper: random",
        description: "Pick a random wallpaper from the directory",
        icon: "media-playlist-shuffle-symbolic",
        keywords: &["wallpaper", "wp", "random", "shuffle"],
        command: &["mshellctl", "wallpaper", "random"],
    },
    // ── Screenshot ────────────────────────────────────────
    McAction {
        id: "mctl:screenshot:region",
        label: "Screenshot: region",
        description: "Drag-select a rectangle and copy / save",
        icon: "screenshot-symbolic",
        keywords: &["screenshot", "shot", "region", "select", "area", "snip"],
        command: &["mscreenshot", "region"],
    },
    McAction {
        id: "mctl:screenshot:full",
        label: "Screenshot: full screen",
        description: "Capture every output and copy / save",
        icon: "video-display-symbolic",
        keywords: &["screenshot", "shot", "full", "screen", "all"],
        command: &["mscreenshot", "full"],
    },
    McAction {
        id: "mctl:screenshot:window",
        label: "Screenshot: focused window",
        description: "Capture just the focused window",
        icon: "window-symbolic",
        keywords: &["screenshot", "shot", "window", "active"],
        command: &["mscreenshot", "window"],
    },
    // ── Compositor ────────────────────────────────────────
    McAction {
        id: "mctl:reload",
        label: "Compositor: reload config",
        description: "Re-read config.conf without restarting",
        icon: "view-refresh-symbolic",
        keywords: &["reload", "compositor", "config", "refresh"],
        command: &["mctl", "dispatch", "reload_config"],
    },
];

/// The 14 mango layouts, in registry-bind order so the index
/// matches `mctl layout <N>`. Pulled from `mctl status` —
/// hardcoded here so the launcher works even when state.json
/// isn't readable (very early in the session).
const LAYOUTS: &[(usize, &str, &str)] = &[
    (0, "Tile", "Master + stack tile"),
    (1, "Scroller", "Niri-style scrollable workspaces"),
    (2, "Grid", "Even N×M grid"),
    (3, "Monocle", "Single full-tag window"),
    (4, "Deck", "Stacked deck of windows"),
    (5, "Center tile", "Master in centre, stacks left+right"),
    (6, "Right tile", "Master on right, stack left"),
    (7, "Vertical scroller", "Scroller rotated 90°"),
    (8, "Vertical tile", "Master on top, stack below"),
    (9, "Vertical grid", "Grid with vertical bias"),
    (10, "Vertical deck", "Deck rotated 90°"),
    (11, "Tgmix", "Tile-grid hybrid"),
    (12, "Canvas", "Free-form spatial canvas"),
    (13, "Dwindle", "BSP-style recursive split"),
];

pub struct MctlProvider;

impl MctlProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MctlProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Score an action against `query`. Returns scores in the same
/// 0..~200 range nucleo produces for Apps fuzzy matches so the
/// runtime's global sort interleaves them sensibly:
///
/// | Match kind | Score |
/// |---|---|
/// | Label prefix | 180 |
/// | Keyword prefix | 150 |
/// | Label contains | 130 |
/// | Keyword contains | 90 |
/// | No match | -1 |
///
/// These constants are intentionally above a typical app fuzzy
/// match (~80-150) so users typing a known mctl verb (`wallpaper`,
/// `night`, `screenshot`) land on the action even when an app
/// happens to fuzzy-subseq-match the same query.
fn match_score(label: &str, keywords: &[&str], query: &str) -> f64 {
    let q = query.to_ascii_lowercase();
    if q.is_empty() {
        return -1.0;
    }
    let label_lower = label.to_ascii_lowercase();
    if label_lower.starts_with(&q) {
        return 180.0;
    }
    let mut best: f64 = -1.0;
    if label_lower.contains(&q) {
        best = best.max(130.0);
    }
    for kw in keywords {
        if kw.starts_with(&q) {
            best = best.max(150.0);
        } else if kw.contains(&q) {
            best = best.max(90.0);
        }
    }
    best
}

impl Provider for MctlProvider {
    fn name(&self) -> &str {
        "Margo"
    }

    fn category(&self) -> &str {
        "Compositor"
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "mctl:palette".into(),
            name: "Margo actions".into(),
            description: "Type wallpaper / night / layout / screenshot / …".into(),
            icon: "preferences-desktop-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Margo".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim();
        if q.is_empty() {
            // Skip empty-query browse — eight different
            // twilight/wallpaper/layout entries would drown out
            // the app list.
            return Vec::new();
        }

        let mut results: Vec<LauncherItem> = Vec::new();

        // Hardcoded named actions.
        for action in ACTIONS {
            let score = match_score(action.label, action.keywords, q);
            if score < 0.0 {
                continue;
            }
            let command: Vec<String> = action.command.iter().map(|s| s.to_string()).collect();
            let label = action.label.to_string();
            results.push(LauncherItem {
                id: action.id.into(),
                name: action.label.into(),
                description: action.description.into(),
                icon: action.icon.into(),
                icon_is_path: false,
                score,
                provider_name: "Margo".into(),
                usage_key: Some(action.id.into()),
                on_activate: Rc::new(move || {
                    run(&command);
                    toast(&label, format!("Ran: {}", command.join(" ")));
                }),
            });
        }

        // Layout entries — match on either the user-friendly
        // label ("Center tile") or the canonical config name
        // ("center_tile") so users who know the config keys
        // don't have to translate.
        let layout_keywords: [&str; 1] = ["layout"];
        for (idx, label, description) in LAYOUTS {
            let canonical = label.to_lowercase().replace(' ', "_");
            let kws = [canonical.as_str(), "layout"];
            // Score against either the human label or the
            // canonical config name; take the max.
            let label_score = match_score(label, &layout_keywords, q);
            let canonical_score = match_score(&canonical, &kws, q);
            let score = label_score.max(canonical_score);
            if score < 0.0 {
                continue;
            }
            let id = format!("mctl:layout:{idx}");
            let command: Vec<String> = vec!["mctl".into(), "layout".into(), idx.to_string()];
            let usage_key = id.clone();
            results.push(LauncherItem {
                id: id.clone(),
                name: format!("Layout: {label}"),
                description: (*description).into(),
                icon: icon_for_layout(&canonical).into(),
                icon_is_path: false,
                score,
                provider_name: "Margo".into(),
                usage_key: Some(usage_key),
                on_activate: Rc::new(move || run(&command)),
            });
        }

        results
    }
}

/// Map a canonical layout name to a MargoMaterial icon. Falls
/// back to `view-list-symbolic` for the few layouts that don't
/// have a dedicated icon yet.
fn icon_for_layout(canonical: &str) -> &'static str {
    match canonical {
        "tile" => "layout-tile-symbolic",
        "scroller" | "vertical_scroller" => "layout-scrolling-symbolic",
        "grid" | "vertical_grid" => "layout-grid-symbolic",
        "monocle" => "layout-monocle-symbolic",
        "deck" | "vertical_deck" => "layout-deck-symbolic",
        "center_tile" => "layout-center-symbolic",
        "right_tile" => "layout-right-symbolic",
        "vertical_tile" => "layout-tile-vertical-symbolic",
        "tgmix" => "layout-mix-symbolic",
        "canvas" => "layout-canvas-symbolic",
        "dwindle" => "layout-dwindle-symbolic",
        _ => "view-list-symbolic",
    }
}

fn run(command: &[String]) {
    let Some((bin, args)) = command.split_first() else {
        return;
    };
    if let Err(err) = Command::new(bin).args(args).spawn() {
        tracing::warn!(?err, ?command, "mctl action spawn failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_nothing() {
        let p = MctlProvider::new();
        assert!(p.search("").is_empty());
    }

    #[test]
    fn keyword_match_finds_twilight_night() {
        let p = MctlProvider::new();
        let items = p.search("night");
        // Should find twilight night + twilight midnight + maybe more.
        assert!(items.iter().any(|i| i.name == "Twilight: night (3500K)"));
    }

    #[test]
    fn wallpaper_keyword_finds_all_three() {
        let p = MctlProvider::new();
        let items = p.search("wallpaper");
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"Wallpaper: next"));
        assert!(names.contains(&"Wallpaper: previous"));
        assert!(names.contains(&"Wallpaper: random"));
    }

    #[test]
    fn layout_keyword_lists_all_14() {
        let p = MctlProvider::new();
        let items = p.search("layout");
        let layout_count = items.iter().filter(|i| i.id.starts_with("mctl:layout:")).count();
        assert_eq!(layout_count, 14);
    }

    #[test]
    fn layout_canonical_name_matches() {
        let p = MctlProvider::new();
        // User types "scroller" — should surface the scroller
        // layout without requiring "layout" prefix.
        let items = p.search("scroller");
        assert!(items.iter().any(|i| i.name == "Layout: Scroller"));
    }

    #[test]
    fn screenshot_keyword_finds_three_modes() {
        let p = MctlProvider::new();
        let items = p.search("screenshot");
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"Screenshot: region"));
        assert!(names.contains(&"Screenshot: full screen"));
        assert!(names.contains(&"Screenshot: focused window"));
    }

    #[test]
    fn nonmatching_query_returns_empty() {
        let p = MctlProvider::new();
        assert!(p.search("zzzunknown").is_empty());
    }

    #[test]
    fn label_prefix_scores_top() {
        let p = MctlProvider::new();
        let items = p.search("Twilight");
        // Label prefix on the new nucleo-comparable scale.
        let twilight = items.iter().find(|i| i.id.starts_with("mctl:twilight:")).unwrap();
        assert!(twilight.score >= 150.0);
    }
}
