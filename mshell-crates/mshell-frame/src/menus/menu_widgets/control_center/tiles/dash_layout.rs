//! The Control Center tiles as mdash's **Controls** tab lays them out:
//! labelled sections (Connectivity, Display, …) flowing through one grid of
//! four equal columns — every tile the same size, every row full — and what a click does there (a
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
/// equal cells as ONE continuous flow in section order: tiles fill rows of
/// four left to right, so every row is full except possibly the last, and
/// a section may start in the middle of a row. A section's caption sits on
/// a caption line above its first tile and spans that section's tiles on
/// that row (Focus 2 + Audio 2 share a row; Valent ends Connectivity's
/// second row beside Display's captioned three). A row on which no section
/// starts has no caption line. Sections keep their order, their tiles stay
/// contiguous, and empty sections vanish.
pub(crate) fn pack(visible: impl Fn(TileId) -> bool) -> Vec<Placement> {
    let flow: Vec<(TileGroup, TileId)> = TileGroup::ALL
        .iter()
        .flat_map(|&g| g.tiles().iter().map(move |&t| (g, t)))
        .filter(|&(_, t)| visible(t))
        .collect();

    let mut out = Vec::new();
    let mut row = 0;
    let mut captioned: Vec<TileGroup> = Vec::new();
    for chunk in flow.chunks(COLUMNS) {
        // Runs of one section on this row: (group, first column, span).
        let mut runs: Vec<(TileGroup, i32, i32)> = Vec::new();
        for (col, &(group, _)) in chunk.iter().enumerate() {
            match runs.last_mut() {
                Some((g, _, span)) if *g == group => *span += 1,
                _ => runs.push((group, col as i32, 1)),
            }
        }
        let mut has_caption = false;
        for &(group, col, span) in &runs {
            if !captioned.contains(&group) {
                captioned.push(group);
                out.push(Placement::Caption {
                    group,
                    col,
                    row,
                    span,
                });
                has_caption = true;
            }
        }
        if has_caption {
            row += 1;
        }
        for (col, &(_, id)) in chunk.iter().enumerate() {
            out.push(Placement::Tile {
                id,
                col: col as i32,
                row,
                span: 1,
            });
        }
        row += 1;
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
        // The second row continues the section: no caption line above it.
        assert_eq!(captions_of(&p).len(), 1);
    }

    fn captions_of(p: &[Placement]) -> Vec<(TileGroup, i32, i32, i32)> {
        p.iter()
            .filter_map(|p| match *p {
                Placement::Caption {
                    group,
                    row,
                    col,
                    span,
                } => Some((group, row, col, span)),
                _ => None,
            })
            .collect()
    }

    /// Tile rows (grid row → tile count), in order.
    fn tile_rows(p: &[Placement]) -> Vec<(i32, usize)> {
        let mut rows: Vec<(i32, usize)> = Vec::new();
        for pl in p {
            if let Placement::Tile { row, .. } = *pl {
                match rows.last_mut() {
                    Some((r, n)) if *r == row => *n += 1,
                    _ => rows.push((row, 1)),
                }
            }
        }
        rows
    }

    #[test]
    fn small_consecutive_groups_share_a_row() {
        let p = pack(|t| {
            TileGroup::Focus.tiles().contains(&t) || TileGroup::Audio.tiles().contains(&t)
        });
        assert_equal_cells(&p);
        // Focus (2) + Audio (3): one flow — Focus and two Audio tiles on
        // the first row, Mic Denoise continues Audio on the next (no
        // caption line of its own).
        assert_eq!(
            captions_of(&p),
            [(TileGroup::Focus, 0, 0, 2), (TileGroup::Audio, 0, 2, 2)]
        );
        assert_eq!(tile_rows(&p), [(1, 4), (2, 1)]);
    }

    /// The laptop fixture's set (Connectivity 5, Display 3, Focus 2,
    /// Audio 2, Services 2, Hardware 2): four full rows, Valent finishing
    /// Connectivity's second row beside Display's caption.
    #[test]
    fn sections_flow_into_full_rows() {
        let shown = [
            TileId::Wifi,
            TileId::Bluetooth,
            TileId::AirplaneMode,
            TileId::Vpn,
            TileId::Valent,
            TileId::DarkMode,
            TileId::NightLight,
            TileId::ColorPicker,
            TileId::Dnd,
            TileId::KeepAwake,
            TileId::AudioOut,
            TileId::Mic,
            TileId::Ufw,
            TileId::Podman,
            TileId::Battery,
            TileId::Disk,
        ];
        let p = pack(|t| shown.contains(&t));
        assert_equal_cells(&p);
        assert_eq!(
            captions_of(&p),
            [
                (TileGroup::Connectivity, 0, 0, 4),
                (TileGroup::Display, 2, 1, 3),
                (TileGroup::Focus, 4, 0, 2),
                (TileGroup::Audio, 4, 2, 2),
                (TileGroup::Services, 6, 0, 2),
                (TileGroup::Hardware, 6, 2, 2),
            ]
        );
        assert_eq!(tile_rows(&p), [(1, 4), (3, 4), (5, 4), (7, 4)]);
        assert_eq!(tiles_of(&p), shown, "tiles keep section order");
    }

    /// Any visible subset: every tile row but the last is full, tiles keep
    /// the section order, each shown section is captioned exactly once,
    /// on the caption line right above its first tile, spanning its run.
    #[test]
    fn every_subset_fills_rows_of_four_in_order() {
        let all: Vec<TileId> = TileId::all().collect();
        // A deterministic sample of masks over the 21 tiles (+ the edge
        // cases all / none / each single tile).
        let mut masks: Vec<u32> = vec![0, (1 << all.len()) - 1];
        masks.extend((0..all.len()).map(|i| 1 << i));
        let mut x: u32 = 0x9e37_79b9;
        for _ in 0..4000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            masks.push(x & ((1 << all.len()) - 1));
        }
        let order: Vec<TileId> = TileGroup::ALL
            .iter()
            .flat_map(|g| g.tiles().iter().copied())
            .collect();
        for mask in masks {
            let shown = |t: TileId| {
                let i = all.iter().position(|a| *a == t).unwrap_or(0);
                mask & (1 << i) != 0
            };
            let p = pack(shown);
            assert_equal_cells(&p);
            let expect: Vec<TileId> = order.iter().copied().filter(|t| shown(*t)).collect();
            assert_eq!(tiles_of(&p), expect, "mask {mask:b}");
            let rows = tile_rows(&p);
            if let Some((_, init)) = rows.split_last() {
                assert!(
                    init.iter().all(|(_, n)| *n == COLUMNS),
                    "mask {mask:b}: part-filled row {rows:?}"
                );
            }
            let caps = captions_of(&p);
            let groups: Vec<TileGroup> = caps.iter().map(|c| c.0).collect();
            let expect_groups: Vec<TileGroup> = TileGroup::ALL
                .iter()
                .copied()
                .filter(|g| g.tiles().iter().any(|t| shown(*t)))
                .collect();
            assert_eq!(groups, expect_groups, "mask {mask:b}");
            for (group, row, col, span) in caps {
                // The caption's run: the tiles right under it.
                let under: Vec<TileId> = p
                    .iter()
                    .filter_map(|pl| match *pl {
                        Placement::Tile {
                            id, col: c, row: r, ..
                        } if r == row + 1 && c >= col && c < col + span => Some(id),
                        _ => None,
                    })
                    .collect();
                assert_eq!(under.len(), span as usize, "mask {mask:b}");
                assert!(under.iter().all(|t| group.tiles().contains(t)));
                let first = group.tiles().iter().copied().find(|t| shown(*t));
                assert_eq!(under.first().copied(), first, "caption over the first tile");
            }
        }
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
