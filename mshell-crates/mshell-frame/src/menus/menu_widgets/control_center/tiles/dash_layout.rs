//! The Control Center tiles as mdash's **Controls** tab lays them out:
//! labelled sections (Connectivity, Display, …) on a grid of four equal
//! columns — every tile the same size — and what a click does there (a
//! toggle first, the related menu on the chevron / right click).
//!
//! Same tiles, same model, same probes as the Control Center (the registry
//! in `registry.rs`): only the arrangement and the click grammar differ, so
//! a toggle here and in the Control Center always agree.

use super::ControlCenterTilesInput;
use super::registry::TileId;
use crate::frame::MenuId;
#[cfg(test)]
use mshell_config::schema::menu_widgets::QuickActionWidget;

/// Columns of the Controls grid: every tile is one column wide, so all
/// tiles have the same size.
pub(crate) const COLUMNS: usize = 4;

/// A captioned section of the Controls tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TileGroup {
    Connectivity,
    Display,
    Focus,
    Audio,
    Services,
    Hardware,
    Custom,
}

impl TileGroup {
    pub(crate) const ALL: [TileGroup; 7] = [
        TileGroup::Connectivity,
        TileGroup::Display,
        TileGroup::Focus,
        TileGroup::Audio,
        TileGroup::Services,
        TileGroup::Hardware,
        TileGroup::Custom,
    ];

    pub(crate) fn caption(self) -> &'static str {
        match self {
            TileGroup::Connectivity => "Connectivity",
            TileGroup::Display => "Display",
            TileGroup::Focus => "Focus",
            TileGroup::Audio => "Audio",
            TileGroup::Services => "Services",
            TileGroup::Hardware => "Hardware",
            TileGroup::Custom => "Custom",
        }
    }

    /// The group's tiles in display order.
    pub(crate) fn tiles(self) -> &'static [TileId] {
        use TileId as T;
        match self {
            TileGroup::Connectivity => &[
                T::Wifi,
                T::Bluetooth,
                T::AirplaneMode,
                T::Vpn,
                T::Hotspot,
                T::Valent,
            ],
            TileGroup::Display => &[T::DarkMode, T::NightLight, T::Mirror, T::ColorPicker],
            TileGroup::Focus => &[T::Dnd, T::KeepAwake],
            TileGroup::Audio => &[T::AudioOut, T::Mic, T::MicDenoise],
            TileGroup::Services => &[T::Ufw, T::Podman],
            TileGroup::Hardware => &[T::Battery, T::Disk, T::InternalKeyboard, T::ThinkpadLed],
            TileGroup::Custom => &[T::CustomShortcut],
        }
    }
}

/// One widget's spot in the Controls grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    Caption {
        group: TileGroup,
        col: i32,
        row: i32,
        span: i32,
    },
    Tile {
        id: TileId,
        col: i32,
        row: i32,
        span: i32,
    },
}

/// Lay out the visible tiles (`visible(id)`) on a [`COLUMNS`]-wide grid of
/// equal cells: each section is a caption line over its tiles, filled left
/// to right. Sections keep their order; two (or more) consecutive sections
/// that fit one row together share it, each over its own whole columns
/// (Focus 2 + Audio 2). Empty sections vanish.
pub(crate) fn pack(visible: impl Fn(TileId) -> bool) -> Vec<Placement> {
    // Bands of consecutive sections that share their rows.
    let mut bands: Vec<Vec<(TileGroup, Vec<TileId>)>> = Vec::new();
    for group in TileGroup::ALL {
        let tiles: Vec<TileId> = group
            .tiles()
            .iter()
            .copied()
            .filter(|&t| visible(t))
            .collect();
        if tiles.is_empty() {
            continue;
        }
        if let Some(band) = bands.last_mut() {
            let used: usize = band.iter().map(|(_, t)| t.len()).sum();
            if used + tiles.len() <= COLUMNS {
                band.push((group, tiles));
                continue;
            }
        }
        bands.push(vec![(group, tiles)]);
    }

    let mut out = Vec::new();
    let mut row = 0;
    for band in bands {
        if band.len() > 1 || band[0].1.len() <= COLUMNS {
            // One row: captions side by side over their own columns.
            let mut col = 0;
            for (group, tiles) in &band {
                out.push(Placement::Caption {
                    group: *group,
                    col,
                    row,
                    span: tiles.len() as i32,
                });
                for &id in tiles {
                    out.push(Placement::Tile {
                        id,
                        col,
                        row: row + 1,
                        span: 1,
                    });
                    col += 1;
                }
            }
            row += 2;
        } else {
            let (group, tiles) = &band[0];
            out.push(Placement::Caption {
                group: *group,
                col: 0,
                row,
                span: COLUMNS as i32,
            });
            row += 1;
            for chunk in tiles.chunks(COLUMNS) {
                for (col, &id) in chunk.iter().enumerate() {
                    out.push(Placement::Tile {
                        id,
                        col: col as i32,
                        row,
                        span: 1,
                    });
                }
                row += 1;
            }
        }
    }
    out
}

/// What a left click on a Controls tile does.
#[derive(Clone, Copy)]
pub(crate) enum DashPrimary {
    /// Flip / run it in place (same input the Control Center sends).
    Input(fn() -> ControlCenterTilesInput),
    /// Open the related menu (closes mdash).
    Open(MenuId),
    /// The tile wires its own click (Internal Keyboard).
    Own,
}

/// A tile's click grammar on the Controls tab: the primary action, and the
/// related menu the chevron and a right click open (`None` = no chevron).
pub(crate) fn dash_actions(id: TileId) -> (DashPrimary, Option<MenuId>) {
    use ControlCenterTilesInput as In;
    use DashPrimary::{Input, Open, Own};
    match id {
        TileId::Wifi => (Open(MenuId::Network), None),
        TileId::Bluetooth => (Input(|| In::ToggleBluetooth), Some(MenuId::Bluetooth)),
        TileId::AudioOut | TileId::Mic => (Open(MenuId::AudioDashboard), None),
        TileId::Vpn => (Open(MenuId::Dns), None),
        TileId::Valent => (Open(MenuId::Valent), None),
        TileId::Battery => (Open(MenuId::Power), None),
        TileId::KeepAwake => (Input(|| In::ClickKeepAwake), Some(MenuId::KeepAwake)),
        TileId::Dnd => (Input(|| In::ClickDnd), Some(MenuId::Notification)),
        TileId::AirplaneMode => (Input(|| In::ClickAirplaneMode), Some(MenuId::Network)),
        TileId::DarkMode => (Input(|| In::ClickDarkMode), None),
        TileId::NightLight => (Input(|| In::ToggleTwilight), Some(MenuId::Twilight)),
        TileId::ColorPicker => (Input(|| In::ClickColorPicker), Some(MenuId::ColorPicker)),
        TileId::Disk => (Open(MenuId::CpuDashboard), None),
        TileId::Ufw => (Input(|| In::ToggleUfw), Some(MenuId::Ufw)),
        // Stopping machines is not a toggle: the tile opens the menu.
        TileId::Podman => (Open(MenuId::Podman), None),
        TileId::Mirror => (Input(|| In::ClickMirror), None),
        TileId::MicDenoise => (Input(|| In::ToggleDenoise), Some(MenuId::AudioDashboard)),
        TileId::Hotspot => (Input(|| In::ToggleHotspot), Some(MenuId::Network)),
        TileId::ThinkpadLed => (Input(|| In::ToggleLed), None),
        TileId::CustomShortcut => (Input(|| In::ClickCustom), None),
        TileId::InternalKeyboard => (Own, None),
    }
}

/// The toggle-style quick actions (the old mdash button grid) and the tile
/// that already is that toggle — the Controls tab shows the tile once.
/// The menu-launcher quick actions live on the Tools tab and the power
/// ones in the header's power popover.
#[cfg(test)]
pub(crate) const QUICK_ACTION_TILES: &[(QuickActionWidget, TileId)] = &[
    (QuickActionWidget::AirplaneMode, TileId::AirplaneMode),
    (QuickActionWidget::DoNotDisturb, TileId::Dnd),
    (QuickActionWidget::ColorPicker, TileId::ColorPicker),
    (QuickActionWidget::IdleInhibitor, TileId::KeepAwake),
    (QuickActionWidget::Nightlight, TileId::NightLight),
];

/// The tile that provides quick action `qa`, if it is a toggle.
#[cfg(test)]
pub(crate) fn tile_for_quick_action(qa: &QuickActionWidget) -> Option<TileId> {
    QUICK_ACTION_TILES
        .iter()
        .find(|(q, _)| q == qa)
        .map(|(_, t)| *t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn tiles_of(p: &[Placement]) -> Vec<TileId> {
        p.iter()
            .filter_map(|p| match p {
                Placement::Tile { id, .. } => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// Every tile is one cell, nothing overlaps or leaves the grid, and
    /// each row's tiles start at column 0 with no hole between them.
    fn assert_equal_cells(p: &[Placement]) {
        let mut cells: HashSet<(i32, i32)> = HashSet::new();
        let mut rows: HashMap<i32, Vec<i32>> = HashMap::new();
        for pl in p {
            let (col, row, span) = match *pl {
                Placement::Caption { col, row, span, .. } => (col, row, span),
                Placement::Tile { col, row, span, .. } => {
                    assert_eq!(span, 1, "{pl:?}: tiles are one column");
                    rows.entry(row).or_default().push(col);
                    (col, row, span)
                }
            };
            assert!(
                span > 0 && col >= 0 && col + span <= COLUMNS as i32,
                "{pl:?}"
            );
            for c in col..col + span {
                assert!(cells.insert((c, row)), "overlap at {c},{row}");
            }
        }
        for (row, mut cols) in rows {
            cols.sort_unstable();
            let expect: Vec<i32> = (0..cols.len() as i32).collect();
            assert_eq!(cols, expect, "tile row {row} has a hole");
        }
    }

    /// Every tile is in exactly one group.
    #[test]
    fn groups_cover_every_tile_once() {
        let mut seen = HashSet::new();
        for g in TileGroup::ALL {
            for t in g.tiles() {
                assert!(seen.insert(t.as_str()), "{t:?} in two groups");
            }
        }
        assert_eq!(seen.len(), TileId::all().len());
    }

    #[test]
    fn all_tiles_pack_into_equal_cells_in_section_order() {
        let p = pack(|_| true);
        assert_equal_cells(&p);
        assert_eq!(tiles_of(&p).len(), TileId::all().len());
        let captions: Vec<TileGroup> = p
            .iter()
            .filter_map(|p| match *p {
                Placement::Caption { group, .. } => Some(group),
                _ => None,
            })
            .collect();
        assert_eq!(captions, TileGroup::ALL, "sections flow in order");
    }

    #[test]
    fn hidden_tiles_leave_no_holes_and_empty_groups_vanish() {
        // A desktop: no battery, no ThinkPad LED, no custom tile.
        let hidden = [TileId::Battery, TileId::ThinkpadLed, TileId::CustomShortcut];
        let p = pack(|t| !hidden.contains(&t));
        assert_equal_cells(&p);
        let tiles = tiles_of(&p);
        assert_eq!(tiles.len(), TileId::all().len() - hidden.len());
        assert!(hidden.iter().all(|h| !tiles.contains(h)));
        assert!(!p.iter().any(|p| matches!(
            p,
            Placement::Caption {
                group: TileGroup::Custom,
                ..
            }
        )));
    }

    #[test]
    fn a_big_group_fills_rows_of_four() {
        let p = pack(|t| TileGroup::Connectivity.tiles().contains(&t));
        assert_equal_cells(&p);
        assert!(matches!(
            p[0],
            Placement::Caption {
                span: 4,
                row: 0,
                ..
            }
        ));
        let rows: Vec<i32> = p
            .iter()
            .filter_map(|p| match *p {
                Placement::Tile { row, .. } => Some(row),
                _ => None,
            })
            .collect();
        assert_eq!(rows, [1, 1, 1, 1, 2, 2], "6 tiles: a row of 4, then 2");
    }

    #[test]
    fn small_consecutive_groups_share_a_row() {
        let p = pack(|t| {
            TileGroup::Focus.tiles().contains(&t) || TileGroup::Audio.tiles().contains(&t)
        });
        assert_equal_cells(&p);
        let captions: Vec<_> = p
            .iter()
            .filter_map(|p| match *p {
                Placement::Caption {
                    group,
                    row,
                    col,
                    span,
                } => Some((group, row, col, span)),
                _ => None,
            })
            .collect();
        // Focus (2) + Audio (3) don't fit one row of four: two bands.
        assert_eq!(
            captions,
            [(TileGroup::Focus, 0, 0, 2), (TileGroup::Audio, 2, 0, 3)]
        );
        let p = pack(|t| {
            TileGroup::Focus.tiles().contains(&t)
                || (TileGroup::Audio.tiles().contains(&t) && t != TileId::MicDenoise)
        });
        let captions: Vec<_> = p
            .iter()
            .filter_map(|p| match *p {
                Placement::Caption { group, col, .. } => Some((group, col)),
                _ => None,
            })
            .collect();
        assert_eq!(
            captions,
            [(TileGroup::Focus, 0), (TileGroup::Audio, 2)],
            "2 + 2 share a row, each over its own columns"
        );
    }

    /// The Controls tab is the union of the Control Center tiles and the
    /// QuickActions toggles: every toggle-style quick action maps onto a
    /// tile (one Wi-Fi, one night light, …) and nothing is duplicated.
    #[test]
    fn quick_action_toggles_dedupe_onto_tiles() {
        let mut mapped = HashSet::new();
        for (qa, tile) in QUICK_ACTION_TILES {
            assert_eq!(tile_for_quick_action(qa), Some(*tile), "{qa:?}");
            mapped.insert(tile.as_str());
        }
        assert_eq!(
            tile_for_quick_action(&QuickActionWidget::Shutdown),
            None,
            "power actions live in the header, not among the toggles"
        );
        let p = pack(|_| true);
        let ids = tiles_of(&p);
        let unique: HashSet<_> = ids.iter().map(|t| t.as_str()).collect();
        assert_eq!(unique.len(), ids.len(), "a tile shows twice");
        assert!(mapped.iter().all(|m| unique.contains(m)));
    }

    /// Only the Internal Keyboard tile (its own confirm flow) wires its
    /// own click; a toggle never also opens its menu on the primary click.
    #[test]
    fn only_the_keyboard_tile_wires_its_own_click() {
        for id in TileId::all() {
            let (primary, more) = dash_actions(id);
            assert_eq!(
                matches!(primary, DashPrimary::Own),
                id == TileId::InternalKeyboard
            );
            if let DashPrimary::Open(_) = primary {
                assert!(more.is_none(), "{id:?}: a menu tile needs no chevron");
            }
        }
    }
}
