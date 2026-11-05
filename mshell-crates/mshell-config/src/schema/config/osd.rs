//! The volume / brightness OSD capsule config (split out of `config.rs`
//! to keep that file under its size cap).

use super::*;

/// On-screen-display capsule (volume / brightness / mic / network pulse)
/// geometry + chrome. `radius` and `border_width` drive CSS vars (live);
/// `width`, `position` and `distance` are applied to the layer-shell
/// windows when they're (re)created — a shell restart picks up changes,
/// like the screen-corner overlays. Border colour follows the matugen
/// `--outline` role — the same one the compositor paints idle window
/// borders with — so the OSD ring matches the window border (DESIGN.md:
/// surfaces over hardcoded colours).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Osd {
    /// Capsule width in px (the OSD's only user-meaningful dimension —
    /// height is content-driven). Drives `--osd-width` (min-width).
    pub width: i32,
    /// Which screen edge the capsule docks against.
    pub position: OsdPosition,
    /// Margin from the docked edge in px (ignored for `Center`).
    pub distance: i32,
    /// Corner radius in px. Drives `--osd-radius`.
    pub radius: i32,
    /// Border thickness in px (`0` hides it). Drives `--osd-border-width`.
    pub border_width: i32,
}

impl Default for Osd {
    fn default() -> Self {
        Self {
            width: 280,
            position: OsdPosition::Bottom,
            distance: 48,
            radius: 24,
            border_width: 2,
        }
    }
}
