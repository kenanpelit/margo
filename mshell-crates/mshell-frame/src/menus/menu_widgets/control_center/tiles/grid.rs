//! The tile widgets: one [`TileSlot`] per registry row (tile button, its
//! edit-mode overlay and corner check), built and wired from
//! [`TILES`](super::registry::TILES), plus the grid layout and the
//! edit-mode visibility rules.

use super::dash_layout::{self, DashPrimary, Placement, TileGroup};
use super::registry::{PrimaryClick, TILES, TileId, TileSpec, ordered_tile_ids};
use super::{ControlCenterTilesInput, ControlCenterTilesModel, ControlCenterTilesOutput};
use crate::menus::menu_widgets::control_center::tile::{TileWidget, build_expand_tile, build_tile};
use relm4::ComponentSender;
use relm4::gtk;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, Cast, CheckButtonExt, GestureExt, GestureSingleExt, GridExt, WidgetExt,
};
use std::cell::RefCell;
use std::collections::HashSet;

mod internal_keyboard;

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
    /// mdash's Controls layout (`None` in the Control Center).
    dash: Option<DashGrid>,
}

/// Which arrangement + click grammar the tiles use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TilesLayout {
    /// The Control Center's two-column, user-ordered grid with edit mode.
    ControlCenter,
    /// mdash's Controls tab: captioned sections packed into full rows.
    Dashboard,
}

/// mdash's grid: the section captions and the visible set it was last
/// packed for (re-packed only when that set changes).
struct DashGrid {
    grid: gtk::Grid,
    captions: Vec<gtk::Label>,
    packed: RefCell<Option<Vec<bool>>>,
}

pub(crate) struct ControlCenterTilesInit {
    pub(crate) layout: TilesLayout,
    /// `false` only in render fixtures: skip the margo-backed wiring.
    pub(crate) live: bool,
}

impl ControlCenterTilesWidgets {
    /// Build every tile from the registry, wire its clicks and wrap it in
    /// an edit-mode overlay. Nothing is attached to the grid yet.
    pub(super) fn build(
        sender: &ComponentSender<ControlCenterTilesModel>,
        layout: TilesLayout,
        grid: &gtk::Grid,
        live: bool,
    ) -> Self {
        if layout == TilesLayout::Dashboard {
            return Self::build_dashboard(sender, grid, live);
        }
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
            .collect::<Vec<TileSlot>>();
        if live {
            internal_keyboard::wire(&slots[TileId::InternalKeyboard as usize].tile);
        }
        Self { slots, dash: None }
    }

    /// mdash's tiles: the same registry rows as split tiles — the tile
    /// button does the primary action, a chevron button (and a right
    /// click) opens the related menu.
    fn build_dashboard(
        sender: &ComponentSender<ControlCenterTilesModel>,
        grid: &gtk::Grid,
        live: bool,
    ) -> Self {
        // 12 equal units (see `dash_layout`), tighter gaps: a compact grid
        // that fills the Controls tab edge to edge.
        grid.remove_css_class("control-center-grid");
        grid.add_css_class("mdash-tiles");
        grid.set_row_spacing(6);
        grid.set_column_spacing(8);
        let slots = TILES
            .iter()
            .map(|spec| build_dash_slot(spec, sender))
            .collect::<Vec<TileSlot>>();
        if live {
            internal_keyboard::wire(&slots[TileId::InternalKeyboard as usize].tile);
        }
        let captions = TileGroup::ALL
            .iter()
            .map(|g| {
                let l = gtk::Label::new(Some(g.caption()));
                l.add_css_class("mdash-section-caption");
                l.set_halign(gtk::Align::Start);
                l.set_xalign(0.0);
                l
            })
            .collect();
        Self {
            slots,
            dash: Some(DashGrid {
                grid: grid.clone(),
                captions,
                packed: RefCell::new(None),
            }),
        }
    }

    /// mdash: lay the visible tiles out again when the visible set
    /// changed since the last pack (a battery appeared, a tile was hidden
    /// in the Control Center's edit mode, …).
    pub(super) fn repack_dashboard(&self) {
        let Some(dash) = &self.dash else {
            return;
        };
        let visible: Vec<bool> = self.slots.iter().map(|s| s.overlay.is_visible()).collect();
        if dash.packed.borrow().as_ref() == Some(&visible) {
            return;
        }
        let grid = &dash.grid;
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }
        for placement in dash_layout::pack(|id| visible[id as usize]) {
            match placement {
                Placement::Caption {
                    group,
                    col,
                    row,
                    span,
                } => {
                    let i = TileGroup::ALL.iter().position(|g| *g == group).unwrap_or(0);
                    grid.attach(&dash.captions[i], col, row, span, 1);
                }
                Placement::Tile { id, col, row, span } => {
                    grid.attach(&self.slot(id).overlay, col, row, span, 1);
                }
            }
        }
        *dash.packed.borrow_mut() = Some(visible);
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
        if let Some(dash) = &self.dash {
            // The Controls tab ignores the Control Center's order / wide
            // tiles; a config change only forces a re-pack.
            *dash.packed.borrow_mut() = None;
            self.repack_dashboard();
            return;
        }
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

// ── mdash split tile ───────────────────────────────────────────────────────────

/// The Controls tab's shorter title where the Control Center's would not
/// fit a quarter-width tile.
fn dash_title(id: TileId, title: &'static str) -> &'static str {
    match id {
        TileId::MicDenoise => "Noise Filter",
        TileId::InternalKeyboard => "Keyboard",
        _ => title,
    }
}

fn build_dash_slot(spec: &TileSpec, sender: &ComponentSender<ControlCenterTilesModel>) -> TileSlot {
    let (primary, menu) = dash_layout::dash_actions(spec.id);
    let title = dash_title(spec.id, spec.title);
    // A tile whose click opens a menu shows the Control Center's inline
    // chevron; a toggle with a related menu gets a separate chevron button.
    let mut tile = if matches!(primary, DashPrimary::Open(_)) {
        build_expand_tile(spec.icon, title, spec.subtitle)
    } else {
        build_tile(spec.icon, title, spec.subtitle)
    };
    tile.button.add_css_class("mdash-tile");
    tile.button.set_hexpand(true);
    match primary {
        DashPrimary::Input(make) => {
            let s = sender.clone();
            tile.button.connect_clicked(move |_| s.input(make()));
        }
        DashPrimary::Open(id) => {
            let s = sender.clone();
            tile.button.connect_clicked(move |_| {
                s.output(ControlCenterTilesOutput::OpenMenu(id)).ok();
            });
        }
        DashPrimary::Own => {}
    }
    let related = menu.or(match primary {
        DashPrimary::Open(id) => Some(id),
        _ => None,
    });
    if let Some(id) = related {
        let s = sender.clone();
        wire_secondary(&tile.button, move || {
            s.output(ControlCenterTilesOutput::OpenMenu(id)).ok();
        });
    }

    let frame = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    frame.add_css_class("mdash-tile-frame");
    frame.set_hexpand(true);
    frame.append(&tile.button);
    if let Some(id) = menu {
        let more = gtk::Button::from_icon_name("go-next-symbolic");
        more.add_css_class("mdash-tile-more");
        mshell_common::a11y::name(&more, &format!("{title} settings"));
        let s = sender.clone();
        more.connect_clicked(move |_| {
            s.output(ControlCenterTilesOutput::OpenMenu(id)).ok();
        });
        frame.append(&more);
    }
    tile.frame = Some(frame.clone());

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&frame));
    let check = gtk::CheckButton::new();
    check.set_visible(false);
    TileSlot {
        tile,
        overlay,
        check,
    }
}

/// A right click on `button` runs `f` (claimed so it doesn't bubble).
fn wire_secondary(button: &gtk::Button, f: impl Fn() + 'static) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
    gesture.connect_pressed(move |g, _, _, _| {
        g.set_state(gtk::EventSequenceState::Claimed);
        f();
    });
    button.add_controller(gesture);
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
