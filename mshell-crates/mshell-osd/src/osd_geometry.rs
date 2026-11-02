//! Shared layer-shell placement for the OSD capsules (volume / brightness /
//! mic / network). The capsule's *chrome* — width, corner radius, border —
//! is CSS-driven (`--osd-width` / `--osd-radius` / `--osd-border-width`,
//! injected live by `mshell-style`), so it isn't handled here. This module
//! only owns the bits CSS can't express: which screen edge the layer-shell
//! window anchors to and its margin from that edge.
//!
//! Read once when each OSD window is created (a shell restart picks up
//! position changes — the same contract as the screen-corner overlays).

use gtk4_layer_shell::{Edge, LayerShell};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, OsdStoreFields};
use mshell_config::schema::position::OsdPosition;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk;

/// `(position, distance)` from the live `osd` config.
pub fn read() -> (OsdPosition, i32) {
    (
        config_manager().config().osd().position().get_untracked(),
        config_manager().config().osd().distance().get_untracked(),
    )
}

/// The single edge `position` anchors to, or `None` for `Center` (no anchors
/// → layer-shell centres the window on the output).
pub(crate) fn anchor_edge(position: &OsdPosition) -> Option<Edge> {
    match position {
        OsdPosition::Top => Some(Edge::Top),
        OsdPosition::Bottom => Some(Edge::Bottom),
        OsdPosition::Left => Some(Edge::Left),
        OsdPosition::Right => Some(Edge::Right),
        OsdPosition::Center => None,
    }
}

/// Anchor `root` to the edge named by `position`, with `distance` px of margin
/// from that edge (`distance` is ignored for `Center`). All four anchors +
/// margins are set explicitly so the call is idempotent.
pub fn apply(root: &gtk::Window, position: &OsdPosition, distance: i32) {
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        root.set_anchor(edge, false);
        root.set_margin(edge, 0);
    }
    if let Some(edge) = anchor_edge(position) {
        root.set_anchor(edge, true);
        root.set_margin(edge, distance);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_side_position_anchors_its_own_edge() {
        assert_eq!(anchor_edge(&OsdPosition::Top), Some(Edge::Top));
        assert_eq!(anchor_edge(&OsdPosition::Bottom), Some(Edge::Bottom));
        assert_eq!(anchor_edge(&OsdPosition::Left), Some(Edge::Left));
        assert_eq!(anchor_edge(&OsdPosition::Right), Some(Edge::Right));
    }

    #[test]
    fn center_anchors_nothing() {
        assert_eq!(anchor_edge(&OsdPosition::Center), None);
    }
}
