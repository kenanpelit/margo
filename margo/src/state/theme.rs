//! Theme preset baseline — captured Config snapshot used by
//! `MargoState::apply_theme_preset("default")` to revert to "what
//! the config file said". Reset on `mctl reload` so the baseline
//! always tracks the latest parse.
//!
//! Lives outside `state.rs` as a step in the W4.2 split — pure
//! Config-snapshot data, no `MargoState` coupling, lift-and-shift
//! away from the 6800-line state.rs reduces incremental rebuild
//! cost when adding theme fields.

use margo_config::Config;

/// Snapshot of the theme-relevant `Config` fields. Captured the
/// first time `apply_theme_preset` runs so `mctl theme default`
/// can revert to "what the config file said". Reset to `None` on
/// `mctl reload` so the baseline always tracks the latest parse.
#[derive(Debug, Clone)]
pub(crate) struct ThemeBaseline {
    pub(crate) borderpx: u32,
    pub(crate) border_radius: i32,
    pub(crate) shadows: bool,
    pub(crate) layer_shadows: bool,
    pub(crate) shadow_only_floating: bool,
    pub(crate) shadows_size: u32,
    pub(crate) shadows_blur: f32,
    pub(crate) blur: bool,
    pub(crate) blur_layer: bool,
}

impl ThemeBaseline {
    pub(crate) fn capture(c: &Config) -> Self {
        Self {
            borderpx: c.borderpx,
            border_radius: c.border_radius,
            shadows: c.shadows,
            layer_shadows: c.layer_shadows,
            shadow_only_floating: c.shadow_only_floating,
            shadows_size: c.shadows_size,
            shadows_blur: c.shadows_blur,
            blur: c.blur,
            blur_layer: c.blur_layer,
        }
    }

    pub(crate) fn apply_to(&self, c: &mut Config) {
        c.borderpx = self.borderpx;
        c.border_radius = self.border_radius;
        c.shadows = self.shadows;
        c.layer_shadows = self.layer_shadows;
        c.shadow_only_floating = self.shadow_only_floating;
        c.shadows_size = self.shadows_size;
        c.shadows_blur = self.shadows_blur;
        c.blur = self.blur;
        c.blur_layer = self.blur_layer;
    }
}

#[cfg(test)]
mod theme_baseline_tests {
    use super::*;

    #[test]
    fn round_trip_preserves_every_captured_field() {
        let mut c = Config {
            borderpx: 3,
            border_radius: 8,
            shadows: true,
            layer_shadows: true,
            shadow_only_floating: true,
            shadows_size: 22,
            shadows_blur: 14.0,
            blur: true,
            blur_layer: false,
            ..Config::default()
        };

        let baseline = ThemeBaseline::capture(&c);

        // Stomp every field with a different value.
        c.borderpx = 1;
        c.border_radius = 0;
        c.shadows = false;
        c.layer_shadows = false;
        c.shadow_only_floating = false;
        c.shadows_size = 0;
        c.shadows_blur = 0.0;
        c.blur = false;
        c.blur_layer = true;

        baseline.apply_to(&mut c);

        assert_eq!(c.borderpx, 3);
        assert_eq!(c.border_radius, 8);
        assert!(c.shadows);
        assert!(c.layer_shadows);
        assert!(c.shadow_only_floating);
        assert_eq!(c.shadows_size, 22);
        assert!((c.shadows_blur - 14.0).abs() < f32::EPSILON);
        assert!(c.blur);
        assert!(!c.blur_layer);
    }
}

// ── apply_theme_preset (moved from state.rs, roadmap Q1 split) ──────────
// The live application of a theme preset onto MargoState; the ThemeBaseline
// data above is its companion. Glue method, kept beside the baseline it reads.
use super::MargoState;

impl MargoState {
    /// Live-swap the visual theme without touching `~/.config/margo/config.conf`.
    ///
    /// Three built-in presets:
    ///   * `default` — restore the values parsed from the config file at
    ///     startup (or the most recent `mctl reload`).
    ///   * `minimal` — borders thin, shadows off, blur off, square corners.
    ///     Good for low-end GPUs or anyone who likes a flat look.
    ///   * `gaudy`   — chunky borders, deep drop shadows, rounded corners,
    ///     blur on. Demo / screenshot mode.
    ///
    /// The first call captures the current config values into
    /// `theme_baseline` so `default` always means "what was on disk
    /// before the user started swapping". `mctl reload` re-invalidates
    /// the baseline so reload + `default` gives the freshly-parsed
    /// values.
    ///
    /// Returns `Err(reason)` for an unknown preset name; the dispatch
    /// handler turns this into a user-visible warning.
    pub fn apply_theme_preset(&mut self, name: &str) -> Result<(), String> {
        // Lazy capture — first preset switch establishes the
        // "what the config file said" baseline.
        if self.theme_baseline.is_none() {
            self.theme_baseline = Some(ThemeBaseline::capture(&self.config));
        }
        let baseline = self.theme_baseline.as_ref().unwrap().clone();

        match name {
            "default" => baseline.apply_to(&mut self.config),
            "minimal" => {
                self.config.shadows = false;
                self.config.layer_shadows = false;
                self.config.shadow_only_floating = false;
                self.config.blur = false;
                self.config.blur_layer = false;
                self.config.border_radius = 0;
                self.config.borderpx = 1;
            }
            "gaudy" => {
                self.config.shadows = true;
                self.config.layer_shadows = true;
                self.config.shadows_size = 32;
                self.config.shadows_blur = 18.0;
                self.config.border_radius = 14;
                self.config.borderpx = 4;
            }
            other => {
                return Err(format!(
                    "unknown theme preset `{other}` — try `default`, `minimal`, or `gaudy`"
                ));
            }
        }

        // Border / shadow / blur all read straight off `self.config`
        // every frame, so an arrange + repaint is enough — no
        // per-client mutation, no animation re-bake.
        self.arrange_all();
        self.request_repaint();
        tracing::info!(target: "theme", "applied preset `{name}`");
        Ok(())
    }
}
