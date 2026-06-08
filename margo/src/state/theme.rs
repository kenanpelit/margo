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
