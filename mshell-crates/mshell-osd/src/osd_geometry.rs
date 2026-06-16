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

/// Anchor `root` to the edge named by `position`, with `distance` px of margin
/// from that edge (`distance` is ignored for `Center`). All four anchors +
/// margins are set explicitly so the call is idempotent.
pub fn apply(root: &gtk::Window, position: &OsdPosition, distance: i32) {
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        root.set_anchor(edge, false);
        root.set_margin(edge, 0);
    }
    match position {
        OsdPosition::Top => {
            root.set_anchor(Edge::Top, true);
            root.set_margin(Edge::Top, distance);
        }
        OsdPosition::Bottom => {
            root.set_anchor(Edge::Bottom, true);
            root.set_margin(Edge::Bottom, distance);
        }
        OsdPosition::Left => {
            root.set_anchor(Edge::Left, true);
            root.set_margin(Edge::Left, distance);
        }
        OsdPosition::Right => {
            root.set_anchor(Edge::Right, true);
            root.set_margin(Edge::Right, distance);
        }
        // No anchors → layer-shell centres the window on the output.
        OsdPosition::Center => {}
    }
}
