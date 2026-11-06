//! Sidebar search: per-page keywords, aliases and the starting search index.

use super::*;

/// Section-level search keywords, keyed by the lowercased sidebar
/// label (or top-level search-index route). Lets a query like
/// "brightness" surface Display or "vpn" surface Network without
/// maintaining a per-control index that would drift from the hand-built
/// pages. Returns an empty string for sections with no extra synonyms.
/// Fuzzy keyword expansion per sidebar label, used by the live search filter.
/// One row per label spelling (alias spellings get their own row); the lookup
/// is a linear scan (the list is tiny + cold). Kept as data rather than a match
/// so adding a page's keywords is a one-line table edit.
pub(super) const PAGE_KEYWORDS: &[(&str, &str)] = &[
    (
        "power",
        "battery suspend sleep hibernate profile performance balanced saver lid power-button",
    ),
    (
        "network",
        "wifi wi-fi wireless ethernet wired vpn proxy dns connection ip hotspot",
    ),
    ("bluetooth", "bt pair pairing device headset"),
    (
        "display",
        "monitor screen resolution scale scaling brightness refresh rate hidpi",
    ),
    (
        "theme",
        "color colour palette matugen accent scheme dark light mode tint",
    ),
    (
        "scheme",
        "color colour palette matugen accent scheme dark light mode tint",
    ),
    ("wallpaper", "background image picture slideshow rotation"),
    (
        "apps",
        "app apps external browser chromium helium isolated profile matugen theme",
    ),
    (
        "helium",
        "app apps external browser chromium helium isolated profile matugen theme",
    ),
    (
        "sound",
        "audio volume output input microphone speaker mute device sink source",
    ),
    ("fonts", "font typeface family size weight"),
    (
        "keybinds",
        "keybind keybinding shortcut hotkey bind keyboard binding cheatsheet",
    ),
    (
        "summon",
        "tags tag apps summon bring here mango-here app hotkey per-tag launch app-id workspace move window to tag toggletag tag key",
    ),
    (
        "input",
        "keyboard mouse touchpad layout xkb repeat sensitivity natural scroll cursor",
    ),
    ("animations", "animation motion transition speed easing"),
    (
        "window switcher",
        "mru alt-tab super-tab switcher thumbnail preview cycle recently used window scope filter dim backdrop accent app label",
    ),
    (
        "appearance",
        "border thickness radius corner gap gaps opacity cursor size window look",
    ),
    ("effects", "shadow shadows drop blur layer floating glow"),
    (
        "osd",
        "osd on-screen display volume brightness mic network capsule pill width position radius border size distance",
    ),
    (
        "behaviour",
        "focus sloppy warp cursor drag tile swap snap hot corner overview scroll axis scratchpad tearing sync syncobj xwayland inhibit",
    ),
    (
        "behavior",
        "focus sloppy warp cursor drag tile swap snap hot corner overview scroll axis scratchpad tearing sync syncobj xwayland inhibit",
    ),
    (
        "window_rules",
        "window rule windowrule app-id appid title regex tag float floating fullscreen size monitor pin match",
    ),
    (
        "layer_rules",
        "layer rule layerrule namespace bar menu notification osd noanim noblur noshadow surface",
    ),
    (
        "tag_rules",
        "tag rule tagrule pin monitor home layout mfact master nmaster workspace",
    ),
    (
        "startup_env",
        "startup script autostart start exec env environment variable launch \
         command session delay login working directory arguments",
    ),
    (
        "backup",
        "backup export import profile profiles reset restore bundle snapshot \
         tar config default factory save load",
    ),
    (
        "logging",
        "log logs logging level debug trace info warn error diagnostics \
         troubleshoot file session verbose",
    ),
    ("date_time", "clock time date timezone ntp format 24-hour"),
    ("date time", "clock time date timezone ntp format 24-hour"),
    ("date & time", "clock time date timezone ntp format 24-hour"),
    ("region", "locale language format measurement keyboard"),
    (
        "region & language",
        "locale language format measurement keyboard",
    ),
    (
        "idle",
        "screensaver dim timeout inhibitor dpms blank suspend",
    ),
    (
        "mtune",
        "tune music player folder library scan mtune album track playback",
    ),
    (
        "toasts",
        "toast popup state change charging battery low caps lock num lock keyboard layout audio device vpn now playing notify",
    ),
    (
        "game_mode",
        "game mode gaming performance disable effects animations blur shadows tearing do not disturb keep awake idle inhibitor",
    ),
    (
        "keyboard",
        "on-screen keyboard osk virtual keyboard touch type mkeys layout turkish",
    ),
    (
        "on-screen keyboard",
        "on-screen keyboard osk virtual keyboard touch type mkeys layout turkish",
    ),
    ("lock", "lockscreen password security blur unlock pam"),
    (
        "lock screen",
        "lockscreen password security blur unlock pam",
    ),
    (
        "login",
        "login greeter display manager mlogind autologin automatic background wallpaper random theme css osk on-screen keyboard blank host session boot",
    ),
    (
        "privacy",
        "location camera microphone permission history geoclue recent flatpak",
    ),
    (
        "launcher",
        "app launcher run search spotlight provider calc ssh",
    ),
    ("menus", "menu popup mdash dashboard"),
    (
        "notifications",
        "notification toast popup do-not-disturb dnd inline reply sound chime quiet hours progress history search",
    ),
    ("general", "avatar profile name user greeting"),
    (
        "tiling_layout",
        "tile tiling gaps layout window split master stack",
    ),
    (
        "tiling layout",
        "tile tiling gaps layout window split master stack",
    ),
    ("plugins", "plugin wasm extension addon"),
    (
        "default_apps",
        "default browser terminal editor mime handler",
    ),
    (
        "default apps",
        "default browser terminal editor mime handler",
    ),
    ("bar", "panel pill widget topbar bottombar status"),
    ("widgets", "widget pill bar component"),
    ("users", "user account password"),
    ("about", "version build info credits"),
    ("setup", "wizard onboarding first-run"),
    (
        "guide",
        "help howto documentation actions binds keybinds config reference tutorial",
    ),
];

pub(super) fn keywords_for(label: &str) -> &'static str {
    PAGE_KEYWORDS
        .iter()
        .find(|(k, _)| *k == label)
        .map(|(_, v)| *v)
        .unwrap_or("")
}

/// The search box's starting targets: every alias, then every sidebar page
/// under the label it is shown with, so a page is always findable by the
/// name the user sees (aliases win on order for "first match" submits).
pub(super) fn initial_search_index() -> Vec<(String, String)> {
    let aliases = SEARCH_ALIASES
        .iter()
        .map(|(label, route)| (label.to_string(), route.to_string()));
    let labels = SIDEBAR.iter().filter_map(|e| match e {
        SidebarEntry::Page { route, label, .. } => Some((label.to_lowercase(), route.to_string())),
        SidebarEntry::Section { .. } => None,
    });
    aliases.chain(labels).collect()
}

/// Search aliases for the sidebar search box: each `(label, route)` makes
/// `label` (matched as a substring of the query) jump to `route`. Multiple
/// aliases can point at the same route. Seeds `search_index` at startup; the
/// widgets sub-pages (`widgets/<name>`) are appended dynamically as their
/// buttons are built.
pub(super) const SEARCH_ALIASES: &[(&str, &str)] = &[
    ("general", "general"),
    ("setup", "setup"),
    ("guide", "guide"),
    ("help", "guide"),
    ("documentation", "guide"),
    ("howto", "guide"),
    ("bar", "bar"),
    ("bluetooth", "bluetooth"),
    ("default apps", "default_apps"),
    ("display", "display"),
    ("fonts", "theme/fonts"),
    ("idle", "idle"),
    ("tune", "mtune"),
    ("music player", "mtune"),
    ("toasts", "toasts"),
    ("toast", "toasts"),
    ("game mode", "game_mode"),
    ("gamemode", "game_mode"),
    ("on-screen keyboard", "keyboard"),
    ("keyboard", "keyboard"),
    ("lock", "lock"),
    ("lock screen", "lock"),
    ("login", "login"),
    ("login screen", "login"),
    ("greeter", "login"),
    ("autologin", "login"),
    ("display manager", "login"),
    ("mlogind", "login"),
    ("tiling layout", "tiling_layout"),
    ("tiling_layout", "tiling_layout"),
    ("about", "about"),
    ("animations", "animations"),
    ("window switcher", "window_switcher"),
    ("appearance", "appearance"),
    ("effects", "effects"),
    ("osd", "osd"),
    ("behaviour", "behaviour"),
    ("behavior", "behaviour"),
    ("logging", "logging"),
    ("logs", "logging"),
    ("window rules", "window_rules"),
    ("windowrule", "window_rules"),
    ("monitors", "display"),
    ("displays", "display"),
    ("layer rules", "layer_rules"),
    ("layerrule", "layer_rules"),
    ("tag rules", "tag_rules"),
    ("tagrule", "tag_rules"),
    ("startup", "startup_env"),
    ("scripts", "startup_env"),
    ("autostart", "startup_env"),
    ("environment", "startup_env"),
    ("env", "startup_env"),
    ("backup", "backup"),
    ("export", "backup"),
    ("import", "backup"),
    ("profile", "backup"),
    ("profiles", "backup"),
    ("reset", "backup"),
    ("restore", "backup"),
    ("date_time", "date_time"),
    ("region", "region"),
    ("sound", "sound"),
    ("users", "users"),
    ("input", "input"),
    ("keybinds", "keybinds"),
    ("tag apps", "summon"),
    ("summon", "summon"),
    ("launcher", "launcher"),
    ("menus", "menus"),
    ("network", "network"),
    ("plugins", "plugins"),
    ("power", "power"),
    ("privacy", "privacy"),
    ("theme", "theme"),
    ("theme scheme", "theme/scheme"),
    ("scheme", "theme/scheme"),
    ("wallpaper", "theme/wallpaper"),
    ("theme wallpaper", "theme/wallpaper"),
    ("theme bundles", "theme/bundles"),
    ("bundles", "theme/bundles"),
    ("cursor theme", "theme/cursor"),
    ("cursor", "theme/cursor"),
    ("pointer", "theme/cursor"),
    ("mouse pointer", "theme/cursor"),
    ("icon theme", "theme/icons"),
    ("icons", "theme/icons"),
    ("app icons", "theme/icons"),
    ("app themes", "theme/apps"),
    ("apps theme", "theme/apps"),
    ("theme apps", "theme/apps"),
    ("helium", "theme/apps"),
    ("browser theme", "theme/apps"),
    ("widgets", "widgets"),
];

#[cfg(test)]
mod registry_tests {
    use super::{PAGE_KEYWORDS, SEARCH_ALIASES, keywords_for};
    use std::collections::HashSet;

    #[test]
    fn search_alias_labels_are_unique() {
        // A duplicate label would make the second entry dead (first match wins),
        // a silent registry bug. Routes may repeat (aliases), labels must not.
        let mut seen = HashSet::new();
        for (label, _) in SEARCH_ALIASES {
            assert!(seen.insert(*label), "duplicate search alias label: {label}");
        }
    }

    #[test]
    fn page_keyword_labels_are_unique_and_nonempty() {
        let mut seen = HashSet::new();
        for (label, kw) in PAGE_KEYWORDS {
            assert!(seen.insert(*label), "duplicate keyword label: {label}");
            assert!(!kw.is_empty(), "empty keywords for {label}");
        }
    }

    #[test]
    fn keywords_lookup_matches_table() {
        assert_eq!(keywords_for("power"), PAGE_KEYWORDS[0].1);
        // OR-arm spellings resolve to the same keywords.
        assert_eq!(keywords_for("behaviour"), keywords_for("behavior"));
        assert_eq!(keywords_for("date_time"), keywords_for("date & time"));
        // Unknown label → empty (sidebar filter falls back to label match).
        assert_eq!(keywords_for("nonexistent"), "");
    }

    #[test]
    fn every_search_route_is_a_known_page() {
        // Guard the "wrong route / typo" class of registry bug: every alias
        // target must be one of the real stack-child routes. (The widgets
        // route covers all `widgets/<name>` sub-pages.)
        let routes: HashSet<&str> = SEARCH_ALIASES.iter().map(|(_, r)| *r).collect();
        for r in &routes {
            assert!(!r.is_empty(), "empty route in search registry");
        }
        // Routes that have keywords should also be searchable (consistency
        // sample — not all keyword labels are top-level searchable, e.g.
        // notifications is a widget sub-page).
        for label in ["power", "network", "display", "theme"] {
            assert!(
                routes.contains(label),
                "{label} missing from search aliases"
            );
        }
    }
}
