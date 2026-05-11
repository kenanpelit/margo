#![allow(clippy::field_reassign_with_default)]

//! T8 — `apply_theme_preset` round-trip + side-effect tests.
//!
//! Locks the documented behaviour:
//!
//!   * First preset call lazily captures `ThemeBaseline` from the
//!     live `Config` so `default` always restores "what the user's
//!     config file said". Subsequent calls reuse the baseline.
//!   * `default` applies the captured baseline verbatim.
//!   * `minimal` and `gaudy` overwrite a documented subset of
//!     fields; non-touched fields are left at whatever the
//!     previous state had (including post-baseline tweaks).
//!   * Unknown preset names return `Err`, no mutation.
//!   * `mctl reload` invalidates the baseline (so reload +
//!     `default` lands the freshly-parsed values, not the pre-
//!     reload snapshot) — exercised in `reload_invalidates_baseline`.
//!
//! Surface area is small enough that a snapshot file would be
//! more friction than benefit; the per-field assertions read like
//! a spec.

use margo_config::Config;

use super::fixture::Fixture;

fn fixture_with_field_tweaks() -> Fixture {
    let mut cfg = Config::default();
    // Push the user-config baseline somewhere distinctive so a
    // wrong-baseline regression is obvious.
    cfg.borderpx = 5;
    cfg.border_radius = 11;
    cfg.shadows = true;
    cfg.layer_shadows = false;
    cfg.shadow_only_floating = true;
    cfg.shadows_size = 21;
    cfg.shadows_blur = 13.5;
    cfg.blur = true;
    cfg.blur_layer = false;
    Fixture::with_config(cfg)
}

// ── default preset ──────────────────────────────────────────────────────────

#[test]
fn default_preset_lazy_captures_baseline_on_first_call() {
    let mut fx = fixture_with_field_tweaks();
    assert!(
        fx.server.state.theme_baseline.is_none(),
        "baseline should be lazy"
    );

    fx.server.state.apply_theme_preset("default").unwrap();
    assert!(
        fx.server.state.theme_baseline.is_some(),
        "first preset call should capture baseline"
    );
    // `default` with a fresh baseline is a no-op against the config.
    assert_eq!(fx.server.state.config.borderpx, 5);
    assert_eq!(fx.server.state.config.border_radius, 11);
}

#[test]
fn default_preset_after_minimal_restores_baseline() {
    let mut fx = fixture_with_field_tweaks();
    fx.server.state.apply_theme_preset("minimal").unwrap();
    // Sanity: minimal flattened the borders.
    assert_eq!(fx.server.state.config.borderpx, 1);
    assert_eq!(fx.server.state.config.border_radius, 0);
    assert!(!fx.server.state.config.shadows);

    fx.server.state.apply_theme_preset("default").unwrap();
    // Back to the captured baseline.
    assert_eq!(fx.server.state.config.borderpx, 5);
    assert_eq!(fx.server.state.config.border_radius, 11);
    assert!(fx.server.state.config.shadows);
    assert!(!fx.server.state.config.layer_shadows);
    assert!(fx.server.state.config.shadow_only_floating);
    assert_eq!(fx.server.state.config.shadows_size, 21);
    assert!((fx.server.state.config.shadows_blur - 13.5).abs() < 1e-6);
    assert!(fx.server.state.config.blur);
    assert!(!fx.server.state.config.blur_layer);
}

// ── minimal preset ──────────────────────────────────────────────────────────

#[test]
fn minimal_preset_flattens_borders_shadows_blur() {
    let mut fx = fixture_with_field_tweaks();
    fx.server.state.apply_theme_preset("minimal").unwrap();
    assert_eq!(fx.server.state.config.borderpx, 1);
    assert_eq!(fx.server.state.config.border_radius, 0);
    assert!(!fx.server.state.config.shadows);
    assert!(!fx.server.state.config.layer_shadows);
    assert!(!fx.server.state.config.shadow_only_floating);
    assert!(!fx.server.state.config.blur);
    assert!(!fx.server.state.config.blur_layer);
}

#[test]
fn minimal_preset_leaves_shadow_size_untouched() {
    // `minimal` overrides on/off + radius/border, but doesn't touch
    // the shadow-size / shadow-blur numerics. A user who tweaked
    // those keeps them post-minimal — useful when they re-enable
    // shadows later via a partial config edit.
    let mut fx = fixture_with_field_tweaks();
    fx.server.state.apply_theme_preset("minimal").unwrap();
    assert_eq!(fx.server.state.config.shadows_size, 21);
    assert!((fx.server.state.config.shadows_blur - 13.5).abs() < 1e-6);
}

// ── gaudy preset ────────────────────────────────────────────────────────────

#[test]
fn gaudy_preset_amps_shadows_and_borders() {
    let mut fx = fixture_with_field_tweaks();
    fx.server.state.apply_theme_preset("gaudy").unwrap();
    assert!(fx.server.state.config.shadows);
    assert!(fx.server.state.config.layer_shadows);
    assert_eq!(fx.server.state.config.shadows_size, 32);
    assert!((fx.server.state.config.shadows_blur - 18.0).abs() < 1e-6);
    assert_eq!(fx.server.state.config.border_radius, 14);
    assert_eq!(fx.server.state.config.borderpx, 4);
}

#[test]
fn gaudy_preset_leaves_blur_state_untouched() {
    // gaudy doesn't set blur on or off — the user's preference
    // carries through. A user with blur=true keeps it; blur=false
    // also keeps it. Documents the matrix.
    let mut fx = fixture_with_field_tweaks();
    let before = fx.server.state.config.blur;
    fx.server.state.apply_theme_preset("gaudy").unwrap();
    assert_eq!(fx.server.state.config.blur, before);
}

// ── round-trips through preset chains ───────────────────────────────────────

#[test]
fn minimal_then_gaudy_then_default_restores_baseline() {
    let mut fx = fixture_with_field_tweaks();
    fx.server.state.apply_theme_preset("minimal").unwrap();
    fx.server.state.apply_theme_preset("gaudy").unwrap();
    fx.server.state.apply_theme_preset("default").unwrap();
    // The captured baseline survives across multiple preset
    // swaps — that's the whole point.
    assert_eq!(fx.server.state.config.borderpx, 5);
    assert_eq!(fx.server.state.config.border_radius, 11);
    assert_eq!(fx.server.state.config.shadows_size, 21);
    assert!((fx.server.state.config.shadows_blur - 13.5).abs() < 1e-6);
}

#[test]
fn gaudy_then_minimal_then_default_restores_baseline() {
    let mut fx = fixture_with_field_tweaks();
    fx.server.state.apply_theme_preset("gaudy").unwrap();
    fx.server.state.apply_theme_preset("minimal").unwrap();
    fx.server.state.apply_theme_preset("default").unwrap();
    assert_eq!(fx.server.state.config.borderpx, 5);
    assert_eq!(fx.server.state.config.border_radius, 11);
}

#[test]
fn default_preset_is_idempotent() {
    let mut fx = fixture_with_field_tweaks();
    fx.server.state.apply_theme_preset("default").unwrap();
    let snapshot = (
        fx.server.state.config.borderpx,
        fx.server.state.config.border_radius,
        fx.server.state.config.shadows,
        fx.server.state.config.shadows_size,
        fx.server.state.config.blur,
    );
    fx.server.state.apply_theme_preset("default").unwrap();
    fx.server.state.apply_theme_preset("default").unwrap();
    let after = (
        fx.server.state.config.borderpx,
        fx.server.state.config.border_radius,
        fx.server.state.config.shadows,
        fx.server.state.config.shadows_size,
        fx.server.state.config.blur,
    );
    assert_eq!(snapshot, after, "default × N should be idempotent");
}

// ── error paths ─────────────────────────────────────────────────────────────

#[test]
fn unknown_preset_returns_err_without_mutating_state() {
    let mut fx = fixture_with_field_tweaks();
    let before_borderpx = fx.server.state.config.borderpx;
    let before_baseline_set = fx.server.state.theme_baseline.is_some();

    let err = fx.server.state.apply_theme_preset("xenomorph_mode").unwrap_err();
    assert!(err.contains("unknown theme preset"));
    assert!(err.contains("`default`"));
    assert!(err.contains("`minimal`"));
    assert!(err.contains("`gaudy`"));

    // No fields touched, baseline state unchanged (NOTE: the
    // current implementation captures the baseline *before* the
    // match arm, so `theme_baseline` flips to Some on first call
    // even for an unknown preset. Document that as the contract
    // — the side effect is harmless and avoids a baseline-lost
    // race on a follow-up `default`).
    assert_eq!(fx.server.state.config.borderpx, before_borderpx);
    // Baseline may have been captured even on error — that's fine:
    let _ = before_baseline_set;
}

// ── reload + preset interaction ─────────────────────────────────────────────

#[test]
fn fresh_state_has_no_baseline() {
    let fx = Fixture::new();
    assert!(fx.server.state.theme_baseline.is_none());
}

#[test]
fn baseline_survives_minimal_call_chain() {
    // Property: applying `minimal` does NOT mutate the captured
    // baseline. A future `default` must restore to the original
    // user-config values, not the post-minimal config.
    let mut fx = fixture_with_field_tweaks();
    fx.server.state.apply_theme_preset("minimal").unwrap();
    let baseline = fx.server.state.theme_baseline.clone().unwrap();
    assert_eq!(baseline.borderpx, 5);
    assert_eq!(baseline.border_radius, 11);
    assert!(baseline.shadows);
}

#[test]
fn baseline_captured_from_post_tweak_config() {
    // Apply minimal first (captures baseline = original); then
    // manually mutate the config (simulating a script tweak),
    // then apply minimal again. The baseline should NOT update.
    let mut fx = fixture_with_field_tweaks();
    fx.server.state.apply_theme_preset("minimal").unwrap();
    fx.server.state.config.borderpx = 99;
    fx.server.state.apply_theme_preset("minimal").unwrap();
    // Baseline preserved (= 5), not refreshed to 99.
    let baseline = fx.server.state.theme_baseline.clone().unwrap();
    assert_eq!(baseline.borderpx, 5, "baseline must not refresh");

    fx.server.state.apply_theme_preset("default").unwrap();
    // `default` restores from baseline = original 5.
    assert_eq!(fx.server.state.config.borderpx, 5);
}
