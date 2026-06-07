//! Pure mapping from mdock config enums to gtk4-layer-shell geometry.
//! No GTK widgets here — just the rules, so they're unit-testable.

use gtk4_layer_shell::Edge;
use mshell_config::schema::config::{DockBehavior, DockPosition};
use relm4::gtk::Orientation;

/// The screen edge the standalone dock anchors to.
pub(crate) fn edge_for(p: DockPosition) -> Edge {
    match p {
        DockPosition::Top => Edge::Top,
        DockPosition::Bottom => Edge::Bottom,
        DockPosition::Left => Edge::Left,
        DockPosition::Right => Edge::Right,
    }
}

/// The dock strip's box orientation for a given edge.
pub(crate) fn orientation_for(p: DockPosition) -> Orientation {
    match p {
        DockPosition::Top | DockPosition::Bottom => Orientation::Horizontal,
        DockPosition::Left | DockPosition::Right => Orientation::Vertical,
    }
}

/// Only an always-on dock reserves an exclusive zone (tiled windows shrink
/// to make room). Auto-hide / toggle float over the windows.
pub(crate) fn reserves_exclusive_zone(b: DockBehavior) -> bool {
    matches!(b, DockBehavior::Always)
}

/// Only auto-hide needs the thin edge trigger surface.
pub(crate) fn uses_edge_trigger(b: DockBehavior) -> bool {
    matches!(b, DockBehavior::AutoHide)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_maps_to_edge_and_orientation() {
        assert_eq!(edge_for(DockPosition::Bottom), Edge::Bottom);
        assert_eq!(edge_for(DockPosition::Left), Edge::Left);
        assert_eq!(orientation_for(DockPosition::Top), Orientation::Horizontal);
        assert_eq!(orientation_for(DockPosition::Left), Orientation::Vertical);
    }

    #[test]
    fn behavior_reserves_zone_only_when_always() {
        assert!(reserves_exclusive_zone(DockBehavior::Always));
        assert!(!reserves_exclusive_zone(DockBehavior::AutoHide));
        assert!(!reserves_exclusive_zone(DockBehavior::Toggle));
    }

    #[test]
    fn autohide_is_the_only_behavior_with_a_trigger() {
        assert!(uses_edge_trigger(DockBehavior::AutoHide));
        assert!(!uses_edge_trigger(DockBehavior::Always));
        assert!(!uses_edge_trigger(DockBehavior::Toggle));
    }
}
