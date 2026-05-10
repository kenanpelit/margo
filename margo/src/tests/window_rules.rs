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
