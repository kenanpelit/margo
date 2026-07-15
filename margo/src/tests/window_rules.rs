#![allow(clippy::field_reassign_with_default)] // Config has 100+ fields

//! Window-rule snapshot tests (W1.3).
//!
//! Locks the `app_id × title → matched-rule-set` decision table for
//! a representative window-rule config. Regressions like "Electron
//! leaked from tag 5" or "Spotify lost its no_animation rule"
//! become a single-line text diff at PR review time.
//!
//! Approach mirrors `layout::snapshot_tests`:
//! - construct a fixture `Config` with a curated set of rules,
//! - feed a `(app_id, title)` candidate matrix through the matcher,
//! - format the resulting matched-rules report as plain text,
//! - lock with `insta::assert_snapshot!`.
//!
//! `cargo insta review` accepts intentional changes; an unintended
//! rule-matcher change shows up as a hunk in the snapshot diff
//! against `margo/src/tests/snapshots/`.
//!
//! Why drive `MargoState::matching_window_rules` directly instead
//! of running the full xdg_shell deferred-map flow: the matcher is
//! a pure function of `(app_id, title, config)`, so we can sweep
//! 100+ candidates per second without paying the wayland-protocol
//! roundtrip cost. Integration coverage of the rule-application
//! sequencing lives in `tests::xdg_shell` (set_app_id_and_title_…).

use insta::assert_snapshot;
use margo_config::{Config, WindowRule};

use super::fixture::Fixture;

/// A curated mini-rulebook designed to hit every matcher branch:
/// positive id, positive title, exclude_id, exclude_title, regex
/// alternation, both-sides positive, etc. Field changes cover the
/// payload that `apply_matched_window_rules` writes back.
fn rules_under_test() -> Vec<WindowRule> {
    vec![
        // 1. Tag-pin Spotify to tag 5, no animations.
        WindowRule {
            id: Some("Spotify".into()),
            tags: 0b0001_0000, // tag 5
            no_animation: Some(true),
            ..Default::default()
        },
        // 2. Float every chooser-style modal by title.
        WindowRule {
            title: Some("^(Open|Save|Choose) .*".into()),
            is_floating: Some(true),
            ..Default::default()
        },
        // 3. Picture-in-picture: float + no_border + no_shadow.
        //    Exclude "Mozilla Firefox" main window so a literal
        //    "Picture-in-Picture" string in the title doesn't
        //    snag the parent toplevel.
        WindowRule {
            title: Some("Picture-in-Picture".into()),
            exclude_title: Some("Mozilla Firefox".into()),
            is_floating: Some(true),
            no_border: Some(true),
            no_shadow: Some(true),
            ..Default::default()
        },
        // 4. Electron / Chromium family: regex alternation on id.
        WindowRule {
            id: Some("^(Helium|Chromium|Brave-browser)$".into()),
            allow_csd: Some(true),
            ..Default::default()
        },
        // 5. CopyQ as a named scratchpad: tag rule + scratchpad bit.
        WindowRule {
            id: Some("com.github.hluk.copyq".into()),
            is_named_scratchpad: Some(true),
            no_focus: Some(true),
            ..Default::default()
        },
        // 6. Negative-only: exclude_id matches kill the rule even if
        //    the positive id wasn't set, when title also matches.
        WindowRule {
            title: Some("Settings".into()),
            exclude_id: Some("dev.zed.Zed".into()),
            no_blur: Some(true),
            ..Default::default()
        },
    ]
}

/// Candidates are chosen so each rule is exercised by at least one
/// match and at least one near-miss (same id, different title or
/// vice-versa). This is what protects against an over-broad
/// regex regression — e.g. tightening rule 4 to require the exact
/// version suffix would show up here as Helium losing rule 4.
const CANDIDATES: &[(&str, &str)] = &[
    ("Spotify", "Spotify Premium"),
    ("Spotify", "Now Playing — Some Track"),
    ("firefox", "Picture-in-Picture"),
    ("firefox", "Mozilla Firefox"),
    ("firefox", "Open File"),
    ("Helium", "DuckDuckGo — Privacy"),
    ("Brave-browser", "New Tab"),
    ("com.github.hluk.copyq", "CopyQ"),
    ("dev.zed.Zed", "Settings"),
    ("nemo", "Settings"),
    ("kitty", "Choose Theme"),
    ("kitty", "kitty"),
];

/// Format the matched-rule indices and the cumulative deltas the
/// rules would apply to a fresh client. Order is stable: rules are
/// matched in declaration order (matching the runtime
/// `apply_matched_window_rules` iteration). The report drops fields
/// no rule touched, keeping the snapshot tight.
fn format_match_report(state: &crate::state::MargoState, rules: &[WindowRule]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for (app_id, title) in CANDIDATES {
        let matched: Vec<(usize, &WindowRule)> = rules
            .iter()
            .enumerate()
            .filter(|(_, r)| state.window_rule_matches(r, app_id, title))
            .collect();
        if matched.is_empty() {
            writeln!(out, "{app_id:32} | {title:34} | (no rule)").unwrap();
            continue;
        }
        let mut deltas = Vec::new();
        let mut tags = 0u32;
        for (i, r) in &matched {
            deltas.push(format!("#{i}"));
            if r.tags != 0 {
                tags = r.tags;
            }
            if let Some(v) = r.is_floating {
                deltas.push(format!("floating={v}"));
            }
            if let Some(v) = r.no_border {
                deltas.push(format!("no_border={v}"));
            }
            if let Some(v) = r.no_shadow {
                deltas.push(format!("no_shadow={v}"));
            }
            if let Some(v) = r.no_animation {
                deltas.push(format!("no_animation={v}"));
            }
            if let Some(v) = r.no_focus {
                deltas.push(format!("no_focus={v}"));
            }
            if let Some(v) = r.no_blur {
                deltas.push(format!("no_blur={v}"));
            }
            if let Some(v) = r.is_named_scratchpad {
                deltas.push(format!("scratchpad={v}"));
            }
            if let Some(v) = r.allow_csd {
                deltas.push(format!("csd={v}"));
            }
        }
        if tags != 0 {
            deltas.push(format!("tags={tags:#x}"));
        }
        writeln!(out, "{app_id:32} | {title:34} | {}", deltas.join(" ")).unwrap();
    }
    out
}

#[test]
fn window_rule_matches_against_curated_candidates() {
    // Single shared fixture for the whole sweep — `matching_window_rules`
    // doesn't mutate state, so per-candidate Fixture::new() would be
    // pure overhead. The fixture supplies a real MargoState (with
    // `Config` injected) so the matcher walks the same code path
    // arrange / new_toplevel hits at runtime.
    let mut config = Config::default();
    config.window_rules = rules_under_test();
    let fx = Fixture::with_config(config.clone());

    let report = format_match_report(&fx.server.state, &config.window_rules);
    assert_snapshot!(report);
}

/// Same matrix, but every rule is hit through the live runtime
/// path: parse a fresh client through new_toplevel + commit and
/// snapshot the resulting `MargoClient` field deltas. Catches
/// the regression class where the matcher is right but the
/// applier wires the wrong field — e.g. swap of `no_border` and
/// `no_shadow` in `apply_matched_window_rules`.
#[test]
fn window_rule_application_via_xdg_shell_flow() {
    use std::fmt::Write;
    let mut config = Config::default();
    config.window_rules = rules_under_test();

    let mut report = String::new();
    for (app_id, title) in CANDIDATES {
        let mut fx = Fixture::with_config(config.clone());
        fx.add_output("DP-1", (1920, 1080));
        let id = fx.add_client();

        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id((*app_id).into());
        toplevel.set_title((*title).into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);

        let client = fx
            .server
            .state
            .clients
            .first()
            .expect("client should exist after first commit");

        let mut deltas = Vec::new();
        // Always print tags so a tag-pinning regression (rule
        // matched but tags=0 wrote nothing back) is visible. The
        // default for a fresh fixture monitor is 0x1 (tag 1).
        deltas.push(format!("tags={:#x}", client.tags));
        if client.is_floating {
            deltas.push("floating=true".into());
        }
        if client.no_border {
            deltas.push("no_border=true".into());
        }
        if client.no_shadow {
            deltas.push("no_shadow=true".into());
        }
        if client.no_animation {
            deltas.push("no_animation=true".into());
        }
        if client.no_focus {
            deltas.push("no_focus=true".into());
        }
        if client.no_blur {
            deltas.push("no_blur=true".into());
        }
        if client.is_named_scratchpad {
            deltas.push("scratchpad=true".into());
        }
        if client.allow_csd {
            deltas.push("csd=true".into());
        }
        writeln!(report, "{app_id:32} | {title:34} | {}", deltas.join(" ")).unwrap();
    }
    assert_snapshot!(report);
}

// ── T1 expansion: matcher edge-case unit tests ───────────────────────────────
//
// Focused on `matches_rule_text` + `window_rule_matches`'s positive /
// negative pattern interactions. The snapshot tests above lock the
// integration shape across a curated candidate matrix; these lock
// the algebra cell-by-cell so a regression in any single branch
// (anchor handling, exclude precedence, empty-pattern semantics)
// flags here even if the curated matrix doesn't happen to exercise
// the broken cell.

#[cfg(test)]
mod edge_cases {
    use super::*;

    /// Build a one-rule fixture and ask the matcher whether the rule
    /// applies to the given `(app_id, title)`. Hides the Fixture
    /// boilerplate.
    fn matches(rule: WindowRule, app_id: &str, title: &str) -> bool {
        let mut config = Config::default();
        config.window_rules = vec![rule.clone()];
        let fx = Fixture::with_config(config);
        fx.server.state.window_rule_matches(&rule, app_id, title)
    }

    // ── id-pattern semantics ─────────────────────────────────────────────────

    #[test]
    fn anchored_id_matches_exact_only() {
        let rule = WindowRule {
            id: Some("^Spotify$".into()),
            ..Default::default()
        };
        assert!(matches(rule.clone(), "Spotify", ""));
        assert!(!matches(rule.clone(), "SpotifyPremium", ""));
        assert!(!matches(rule, "FooSpotify", ""));
    }

    #[test]
    fn unanchored_id_matches_substring() {
        let rule = WindowRule {
            id: Some("Spotify".into()),
            ..Default::default()
        };
        assert!(matches(rule.clone(), "Spotify", ""));
        assert!(matches(rule.clone(), "Spotify Premium", ""));
        assert!(matches(rule, "MySpotifyApp", ""));
    }

    #[test]
    fn id_matching_is_case_sensitive() {
        // Regex semantics: no /i flag means literal case match.
        let rule = WindowRule {
            id: Some("Spotify".into()),
            ..Default::default()
        };
        assert!(!matches(rule, "spotify", ""));
    }

    #[test]
    fn regex_alternation_in_id_works() {
        let rule = WindowRule {
            id: Some("^(Helium|Chromium|Brave-browser)$".into()),
            ..Default::default()
        };
        assert!(matches(rule.clone(), "Helium", ""));
        assert!(matches(rule.clone(), "Chromium", ""));
        assert!(matches(rule.clone(), "Brave-browser", ""));
        assert!(!matches(rule.clone(), "Firefox", ""));
        // Anchored: substring inside one of the alternatives must NOT match.
        assert!(!matches(rule, "Helium-stable", ""));
    }

    #[test]
    fn character_class_in_title_matches() {
        let rule = WindowRule {
            title: Some("^Tab [0-9]+$".into()),
            ..Default::default()
        };
        assert!(matches(rule.clone(), "app", "Tab 5"));
        assert!(matches(rule.clone(), "app", "Tab 123"));
        assert!(!matches(rule.clone(), "app", "Tab five"));
        assert!(!matches(rule, "app", "Tab 5 — Discord"));
    }

    // ── empty / absent pattern semantics ─────────────────────────────────────

    #[test]
    fn rule_with_no_patterns_matches_everything() {
        let rule = WindowRule::default();
        assert!(matches(rule.clone(), "anything", ""));
        assert!(matches(rule.clone(), "", "anything"));
        assert!(matches(rule, "", ""));
    }

    #[test]
    fn empty_pattern_string_matches_anything() {
        // `Some("")` should be filtered out by the matcher's
        // `filter(|p| !p.is_empty())` — i.e. equivalent to None.
        let rule = WindowRule {
            id: Some(String::new()),
            title: Some(String::new()),
            ..Default::default()
        };
        assert!(matches(rule.clone(), "Spotify", "anything"));
        // Even an empty (app_id, title) shouldn't be rejected, since
        // there's no positive constraint.
        assert!(matches(rule, "", ""));
    }

    #[test]
    fn empty_value_against_non_empty_pattern_fails() {
        let rule = WindowRule {
            id: Some("Spotify".into()),
            ..Default::default()
        };
        // Empty value can't match a non-empty pattern — protects
        // against the "newly-mapped Electron toplevel before app_id
        // settles" corner case.
        assert!(!matches(rule, "", ""));
    }

    // ── multi-field AND semantics ────────────────────────────────────────────

    #[test]
    fn id_and_title_both_must_match() {
        let rule = WindowRule {
            id: Some("Spotify".into()),
            title: Some("Premium".into()),
            ..Default::default()
        };
        assert!(matches(rule.clone(), "Spotify", "Spotify Premium"));
        assert!(!matches(rule.clone(), "Spotify", "Free Edition"));
        assert!(!matches(rule.clone(), "Firefox", "Premium"));
        assert!(!matches(rule, "Firefox", "Free Edition"));
    }

    #[test]
    fn id_only_rule_ignores_title() {
        let rule = WindowRule {
            id: Some("Spotify".into()),
            ..Default::default()
        };
        assert!(matches(rule.clone(), "Spotify", "any title"));
        assert!(matches(rule, "Spotify", ""));
    }

    #[test]
    fn title_only_rule_ignores_id() {
        let rule = WindowRule {
            title: Some("Picture-in-Picture".into()),
            ..Default::default()
        };
        assert!(matches(rule.clone(), "anything", "Picture-in-Picture"));
        assert!(matches(rule, "", "Picture-in-Picture"));
    }

    // ── exclude_* precedence ─────────────────────────────────────────────────

    #[test]
    fn exclude_id_blocks_otherwise_matching_rule() {
        let rule = WindowRule {
            title: Some("Settings".into()),
            exclude_id: Some("dev.zed.Zed".into()),
            ..Default::default()
        };
        // Positive `title` would match; exclude_id should veto.
        assert!(!matches(rule.clone(), "dev.zed.Zed", "Settings"));
        // Non-Zed Settings windows match cleanly.
        assert!(matches(rule, "nemo", "Settings"));
    }

    #[test]
    fn exclude_title_blocks_otherwise_matching_rule() {
        let rule = WindowRule {
            title: Some("Picture-in-Picture".into()),
            exclude_title: Some("Mozilla Firefox".into()),
            ..Default::default()
        };
        // PiP window without Mozilla in the title: matches.
        assert!(matches(rule.clone(), "firefox", "Picture-in-Picture"));
        // PiP with Mozilla Firefox in the title (e.g. main window
        // showing the PiP indicator text): blocked.
        assert!(!matches(
            rule,
            "firefox",
            "Mozilla Firefox — Picture-in-Picture"
        ));
    }

    #[test]
    fn exclude_id_unmatched_does_not_block() {
        let rule = WindowRule {
            id: Some("Spotify".into()),
            exclude_id: Some("FooApp".into()),
            ..Default::default()
        };
        // exclude_id pattern doesn't match → positive id wins.
        assert!(matches(rule, "Spotify", ""));
    }

    // ── invalid-regex fallback path ──────────────────────────────────────────

    #[test]
    fn invalid_regex_falls_back_to_substring() {
        // `[invalid` is an unclosed character class — regex::Regex
        // refuses to compile. The matcher's fallback strips anchors
        // and treats the rest as substring.
        let rule = WindowRule {
            id: Some("[invalid".into()),
            ..Default::default()
        };
        // The fallback `value.contains("[invalid")` would only hit
        // an app_id containing that literal substring; "[invalidApp"
        // does, "OtherApp" doesn't.
        assert!(matches(rule.clone(), "[invalidApp", ""));
        assert!(!matches(rule, "OtherApp", ""));
    }

    #[test]
    fn invalid_regex_with_anchors_strips_them() {
        // Even when the regex is invalid, the leading `^` and
        // trailing `$` get stripped before the substring compare,
        // so a quoted "^[invalid$" pattern still recognises
        // `[invalid` inside the app_id.
        let rule = WindowRule {
            id: Some("^[invalid$".into()),
            ..Default::default()
        };
        assert!(matches(rule, "[invalid", ""));
    }
}

// ── Float-dialog placement: centre over the focused window ────────────────────
//
// A rule-floated dialog (keyring / polkit prompt) should open over the window
// that triggered it — the focused window — rather than the monitor centre. That
// matters most in the scroller layout, where the focused window is rarely at
// the work-area centre. These lock the `rule_float_geometry_for` +
// `focus_anchor_for` contract: anchor-centring, on-screen clamping, and the
// offset-rule escape hatch that keeps corner-pinned windows (copyq / PiP) put.

#[cfg(test)]
mod float_placement {
    use super::*;
    use crate::layout::Rect;
    use crate::state::MargoState;

    /// A single 1080p output whose `work_area` equals its full area (a bare
    /// fixture reserves no bar space). Returns the fixture and that work area.
    fn fixture_1080p() -> (Fixture, Rect) {
        let mut fx = Fixture::with_config(Config::default());
        fx.add_output("DP-1", (1920, 1080));
        let area = fx.server.state.monitors[0].work_area;
        (fx, area)
    }

    #[test]
    fn empty_rule_no_anchor_centres_on_work_area() {
        // Legacy behaviour, unchanged: no anchor → 60%-of-work-area, centred.
        let (fx, area) = fixture_1080p();
        let rule = WindowRule::default();
        let g = MargoState::rule_float_geometry_for(&fx.server.state.monitors, 0, &rule, None);
        let w = (area.width as f32 * 0.6) as i32;
        let h = (area.height as f32 * 0.6) as i32;
        assert_eq!(
            g,
            Rect::new(
                area.x + (area.width - w) / 2,
                area.y + (area.height - h) / 2,
                w,
                h,
            )
        );
    }

    #[test]
    fn sized_rule_centres_over_focus_anchor() {
        // A 400×200 dialog centres over the focused window's rect, not the
        // monitor centre, when the anchor sits well inside the work area.
        let (fx, area) = fixture_1080p();
        let rule = WindowRule {
            width: 400,
            height: 200,
            ..Default::default()
        };
        let anchor = Rect::new(area.x + 600, area.y + 400, 200, 200);
        let g =
            MargoState::rule_float_geometry_for(&fx.server.state.monitors, 0, &rule, Some(anchor));
        assert_eq!(g.width, 400);
        assert_eq!(g.height, 200);
        assert_eq!(g.x, anchor.x + (anchor.width - 400) / 2);
        assert_eq!(g.y, anchor.y + (anchor.height - 200) / 2);
    }

    #[test]
    fn focus_anchor_placement_clamps_to_work_area() {
        // A dialog centred over an edge-hugging window must stay fully
        // on-screen — clamp to the work-area inner edges, never spill.
        let (fx, area) = fixture_1080p();
        let rule = WindowRule {
            width: 400,
            height: 200,
            ..Default::default()
        };
        // Top-left corner window → clamp to the top-left work-area edge.
        let tl = Rect::new(area.x, area.y, 60, 60);
        let g = MargoState::rule_float_geometry_for(&fx.server.state.monitors, 0, &rule, Some(tl));
        assert_eq!(g.x, area.x);
        assert_eq!(g.y, area.y);
        // Bottom-right corner window → clamp to the bottom-right inner edge.
        let br = Rect::new(area.x + area.width - 60, area.y + area.height - 60, 60, 60);
        let g = MargoState::rule_float_geometry_for(&fx.server.state.monitors, 0, &rule, Some(br));
        assert_eq!(g.x, area.x + area.width - 400);
        assert_eq!(g.y, area.y + area.height - 200);
    }

    #[test]
    fn offset_rule_ignores_focus_anchor() {
        // A rule that pins an explicit screen offset keeps its exact
        // work-area-relative placement — the anchor is ignored so
        // corner-pinned windows (copyq / PiP) never drift onto the focus.
        let (fx, area) = fixture_1080p();
        let rule = WindowRule {
            width: 400,
            height: 200,
            offset_x: 50,
            offset_y: -30,
            ..Default::default()
        };
        let anchor = Rect::new(area.x + 600, area.y + 400, 200, 200);
        let with =
            MargoState::rule_float_geometry_for(&fx.server.state.monitors, 0, &rule, Some(anchor));
        let without =
            MargoState::rule_float_geometry_for(&fx.server.state.monitors, 0, &rule, None);
        assert_eq!(with, without);
        assert_eq!(with.x, area.x + (area.width - 400) / 2 + 50);
        assert_eq!(with.y, area.y + (area.height - 200) / 2 - 30);
    }

    #[test]
    fn focus_anchor_for_selection_guards() {
        // `focus_anchor_for` returns the selected window's geom, except when
        // the selection is the new window itself, the monitor is out of range,
        // there is no selection, or the geom is degenerate.
        let (mut fx, _area) = fixture_1080p();
        let id = fx.add_client();
        let (toplevel, surface) = fx.client(id).create_toplevel();
        toplevel.set_app_id("kitty".into());
        toplevel.set_title("kitty".into());
        surface.commit();
        fx.client(id).flush();
        fx.roundtrip(id);

        // No selection yet → None (work-area fallback), and an out-of-range
        // monitor index is None, never a panic.
        assert_eq!(fx.server.state.focus_anchor_for(0, 0), None);
        assert_eq!(fx.server.state.focus_anchor_for(99, 0), None);

        // Give the mapped client a known geometry and select it.
        let cidx = 0usize;
        fx.server.state.clients[cidx].monitor = 0;
        fx.server.state.clients[cidx].geom = Rect::new(700, 500, 300, 250);
        fx.server.state.monitors[0].selected = Some(cidx);

        // A different new window anchors over the selected client…
        assert_eq!(
            fx.server.state.focus_anchor_for(0, 999),
            Some(Rect::new(700, 500, 300, 250))
        );
        // …but a window can never anchor over itself.
        assert_eq!(fx.server.state.focus_anchor_for(0, cidx), None);

        // A degenerate (0×0) selection falls back to work-area centring.
        fx.server.state.clients[cidx].geom = Rect::new(0, 0, 0, 0);
        assert_eq!(fx.server.state.focus_anchor_for(0, 999), None);
    }
}
