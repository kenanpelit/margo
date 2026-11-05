//! The tile widgets: one [`TileSlot`] per registry row (tile button, its
//! edit-mode overlay and corner check), built and wired from
//! [`TILES`](super::registry::TILES), plus the grid layout and the
//! edit-mode visibility rules.

use super::registry::{PrimaryClick, TILES, TileId, ordered_tile_ids};
use super::{ControlCenterTilesInput, ControlCenterTilesModel, ControlCenterTilesOutput};
use crate::menus::menu_widgets::control_center::tile::{TileWidget, build_expand_tile, build_tile};
use relm4::ComponentSender;
use relm4::gtk;
use relm4::gtk::prelude::{
    ButtonExt, Cast, CheckButtonExt, GestureExt, GestureSingleExt, GridExt, WidgetExt,
};
use std::collections::HashSet;

/// A tile button, the `gtk::Overlay` wrapping it in the grid, and the
/// edit-mode CheckButton in the overlay's corner.
pub(crate) struct TileSlot {
    pub(super) tile: TileWidget,
    overlay: gtk::Overlay,
    check: gtk::CheckButton,
}

/// The tile handles, indexed by `TileId as usize`. Every tile is held so
/// its click handlers stay alive (the colour picker has no state to show).
pub(crate) struct ControlCenterTilesWidgets {
    slots: Vec<TileSlot>,
}

impl ControlCenterTilesWidgets {
    /// Build every tile from the registry, wire its clicks and wrap it in
    /// an edit-mode overlay. Nothing is attached to the grid yet.
    pub(super) fn build(sender: &ComponentSender<ControlCenterTilesModel>) -> Self {
        let slots = TILES
            .iter()
            .map(|spec| {
                let tile = match spec.click {
                    PrimaryClick::Expand(_) => {
                        build_expand_tile(spec.icon, spec.title, spec.subtitle)
                    }
                    _ => build_tile(spec.icon, spec.title, spec.subtitle),
                };
                match spec.click {
                    PrimaryClick::Expand(page) => {
                        let s = sender.clone();
                        tile.button.connect_clicked(move |_| {
                            s.output(ControlCenterTilesOutput::ExpandPage(page)).ok();
                        });
                    }
                    PrimaryClick::Input(make) => {
                        let s = sender.clone();
                        tile.button.connect_clicked(move |_| s.input(make()));
                    }
                    PrimaryClick::None => {}
                }
                if let Some(make) = spec.secondary {
                    wire_secondary_toggle(&tile.button, sender, make);
                }
                let (overlay, check) = build_edit_overlay(&tile.button, sender, spec.id);
                TileSlot {
                    tile,
                    overlay,
                    check,
                }
            })
            .collect();
        Self { slots }
    }

    fn slot(&self, id: TileId) -> &TileSlot {
        &self.slots[id as usize]
    }

    /// The tile widget for `id`.
    pub(super) fn tile(&self, id: TileId) -> &TileWidget {
        &self.slot(id).tile
    }

    /// Show or hide a tile for the current mode. In edit mode every tile
    /// is shown with its check (unchecked = configured hidden, dimmed via
    /// `.disabled`); in normal mode only tiles that are `enabled` in the
    /// config and `available` (e.g. a battery is present) are shown.
    pub(super) fn update_visibility(&self, id: TileId, edit: bool, enabled: bool, available: bool) {
        let TileSlot {
            tile,
            overlay,
            check,
        } = self.slot(id);
        let tile_btn = &tile.button;
        if edit {
            overlay.set_visible(true);
            tile_btn.set_visible(true);
            if enabled {
                tile_btn.remove_css_class("disabled");
            } else {
                tile_btn.add_css_class("disabled");
            }
            check.set_visible(true);
            check.set_active(enabled);
        } else {
            overlay.set_visible(enabled && available);
            tile_btn.set_visible(true);
            tile_btn.remove_css_class("disabled");
            check.set_visible(false);
        }
    }

    /// Detach all tile overlays from `grid`, then reattach them in the order
    /// described by `tile_order`. Tiles whose id is in `wide_tiles` span 2
    /// columns. Each non-wide tile occupies one cell; we fill left-to-right,
    /// creating a new row whenever the current column would overflow or a
    /// wide tile needs a fresh row.
    pub(super) fn rebuild_grid(
        &self,
        grid: &gtk::Grid,
        tile_order: &[String],
        wide_tiles: &HashSet<String>,
    ) {
        // Detach only the overlays actually parented to this grid.
        // `TileId::all()` is the full universe of tiles, but hidden
        // tiles (id not in `tile_order`) were never attached, and on
        // the first build nothing is attached yet — an unconditional
        // `grid.remove` on those tripped `gtk_grid_remove: parent ==
        // grid` assertions (a burst of warnings per rebuild).
        let grid_widget: gtk::Widget = grid.clone().upcast();
        for id in TileId::all() {
            let overlay = &self.slot(id).overlay;
            if overlay.parent().as_ref() == Some(&grid_widget) {
                grid.remove(overlay);
            }
        }

        // Re-attach in config order, appending unknowns at the end.
        let ids = ordered_tile_ids(tile_order);
        let mut col: i32 = 0;
        let mut row: i32 = 0;
        for id in ids {
            let overlay = &self.slot(id).overlay;
            let is_wide = wide_tiles.contains(id.as_str());
            if is_wide {
                // Wide tiles always start at column 0 of a fresh row.
                if col != 0 {
                    row += 1;
                }
                grid.attach(overlay, 0, row, 2, 1);
                row += 1;
                col = 0;
            } else {
                grid.attach(overlay, col, row, 1, 1);
                col += 1;
                if col >= 2 {
                    col = 0;
                    row += 1;
                }
            }
        }
    }
}

// ── Secondary-click quick toggle ───────────────────────────────────────────────

/// Wire a secondary (right) click on a tile button to a quick-toggle
/// input. The button keeps its `connect_clicked` (left-click → open the
/// detail page); the right-click flips the state without leaving the
/// grid. The press is claimed so it doesn't bubble.
fn wire_secondary_toggle(
    button: &gtk::Button,
    sender: &ComponentSender<ControlCenterTilesModel>,
    make: impl Fn() -> ControlCenterTilesInput + 'static,
) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
    let sender = sender.clone();
    gesture.connect_pressed(move |g, _, _, _| {
        g.set_state(gtk::EventSequenceState::Claimed);
        sender.input(make());
    });
    button.add_controller(gesture);
}

// ── Edit-overlay builder ───────────────────────────────────────────────────────

fn build_edit_overlay(
    tile_btn: &gtk::Button,
    sender: &ComponentSender<ControlCenterTilesModel>,
    tile_id: TileId,
) -> (gtk::Overlay, gtk::CheckButton) {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(tile_btn));

    let check = gtk::CheckButton::new();
    check.add_css_class("control-center-edit-check");
    check.set_halign(gtk::Align::End);
    check.set_valign(gtk::Align::Start);
    check.set_visible(false);
    check.set_can_focus(false);

    overlay.add_overlay(&check);

    let s = sender.clone();
    check.connect_toggled(move |cb| {
        let active = cb.is_active();
        s.input(ControlCenterTilesInput::EditTileVisibility(tile_id, active));
    });

    (overlay, check)
}
