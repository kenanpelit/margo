//! The Control Center tiles as mdash's **Controls** tab lays them out:
//! labelled sections (Connectivity, Display, …) packed into a 12-unit grid
//! so every row is full — no hole at the end of a short row, no empty half
//! column — and what a click does there (a toggle first, the related menu
//! on the chevron / right click).
//!
//! Same tiles, same model, same probes as the Control Center (the registry
//! in `registry.rs`): only the arrangement and the click grammar differ, so
//! a toggle here and in the Control Center always agree.

use super::ControlCenterTilesInput;
use super::registry::TileId;
use crate::frame::MenuId;
#[cfg(test)]
use mshell_config::schema::menu_widgets::QuickActionWidget;

/// Grid width in units. 12 divides by 1, 2, 3 and 4, so a row of 1–4
/// tiles always fills it exactly.
pub(crate) const UNITS: i32 = 12;
/// Most tiles a row holds.
pub(crate) const ROW_MAX: usize = 4;

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

/// A band: a caption row and the tile row(s) under it. A group of up to
/// four tiles shares its band with the groups that still fit; a bigger
/// group gets its own band and spreads evenly over its rows.
#[derive(Debug, Default)]
struct Band {
    /// (group, its visible tiles) in band order.
    groups: Vec<(TileGroup, Vec<TileId>)>,
}

impl Band {
    fn width(&self) -> usize {
        self.groups.iter().map(|(_, t)| t.len()).sum()
    }
    fn is_single_row(&self) -> bool {
        self.width() <= ROW_MAX
    }
}

/// How `n` tiles split over the fewest rows of at most [`ROW_MAX`], as
/// evenly as possible (6 → 3 + 3, 5 → 3 + 2, 7 → 4 + 3).
fn split_rows(n: usize) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let rows = n.div_ceil(ROW_MAX);
    let base = n / rows;
    let extra = n % rows;
    (0..rows).map(|i| base + usize::from(i < extra)).collect()
}

/// Lay out the visible tiles (`visible(id)`): captions + tiles, every tile
/// row exactly [`UNITS`] wide. Empty groups vanish. Groups keep their order
/// except that a small group may join an earlier band that still has room
/// (first fit), which is what keeps the rows full.
pub(crate) fn pack(visible: impl Fn(TileId) -> bool) -> Vec<Placement> {
    let mut bands: Vec<Band> = Vec::new();
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
        if tiles.len() <= ROW_MAX
            && let Some(band) = bands
                .iter_mut()
                .find(|b| b.is_single_row() && b.width() + tiles.len() <= ROW_MAX)
        {
            band.groups.push((group, tiles));
            continue;
        }
        bands.push(Band {
            groups: vec![(group, tiles)],
        });
    }

    let mut out = Vec::new();
    let mut row = 0;
    for band in bands {
        if band.is_single_row() {
            let span = UNITS / band.width() as i32;
            let mut col = 0;
            for (group, tiles) in &band.groups {
                out.push(Placement::Caption {
                    group: *group,
                    col,
                    row,
                    span: span * tiles.len() as i32,
                });
                for (i, &id) in tiles.iter().enumerate() {
                    out.push(Placement::Tile {
                        id,
                        col: col + span * i as i32,
                        row: row + 1,
                        span,
                    });
                }
                col += span * tiles.len() as i32;
            }
            row += 2;
        } else {
            let (group, tiles) = &band.groups[0];
            out.push(Placement::Caption {
                group: *group,
                col: 0,
                row,
                span: UNITS,
            });
            row += 1;
            let mut it = tiles.iter();
            for count in split_rows(tiles.len()) {
                let span = UNITS / count as i32;
                for i in 0..count {
                    if let Some(&id) = it.next() {
                        out.push(Placement::Tile {
                            id,
                            col: span * i as i32,
                            row,
                            span,
                        });
                    }
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

    /// Every tile row is exactly UNITS wide and no two widgets overlap.
    fn assert_full_rows(p: &[Placement]) {
        let mut rows: HashMap<i32, i32> = HashMap::new();
        let mut cells: HashSet<(i32, i32)> = HashSet::new();
        for pl in p {
            let (col, row, span, is_tile) = match *pl {
                Placement::Caption { col, row, span, .. } => (col, row, span, false),
                Placement::Tile { col, row, span, .. } => (col, row, span, true),
            };
            assert!(span > 0 && col >= 0 && col + span <= UNITS, "{pl:?}");
            for c in col..col + span {
                assert!(cells.insert((c, row)), "overlap at {c},{row}");
            }
            if is_tile {
                *rows.entry(row).or_default() += span;
            }
        }
        for (row, width) in rows {
            assert_eq!(width, UNITS, "tile row {row} is not full");
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
    fn all_tiles_pack_into_full_rows() {
        let p = pack(|_| true);
        assert_full_rows(&p);
        assert_eq!(tiles_of(&p).len(), TileId::all().len());
        // 22 tiles, seven sections → six tile rows, five caption rows.
        let max_row = p
            .iter()
            .map(|p| match *p {
                Placement::Caption { row, .. } | Placement::Tile { row, .. } => row,
            })
            .max();
        assert_eq!(max_row, Some(10));
    }

    #[test]
    fn hidden_tiles_leave_no_holes_and_empty_groups_vanish() {
        // A desktop: no battery, no ThinkPad LED, no custom tile.
        let hidden = [TileId::Battery, TileId::ThinkpadLed, TileId::CustomShortcut];
        let p = pack(|t| !hidden.contains(&t));
        assert_full_rows(&p);
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
    fn a_big_group_spreads_evenly_over_its_rows() {
        assert_eq!(split_rows(6), [3, 3]);
        assert_eq!(split_rows(5), [3, 2]);
        assert_eq!(split_rows(4), [4]);
        assert_eq!(split_rows(7), [4, 3]);
        assert!(split_rows(0).is_empty());
        let p = pack(|t| TileGroup::Connectivity.tiles().contains(&t));
        assert_full_rows(&p);
        assert!(matches!(p[0], Placement::Caption { span: UNITS, .. }));
    }

    #[test]
    fn small_groups_share_a_band() {
        let p = pack(|t| {
            TileGroup::Focus.tiles().contains(&t) || TileGroup::Services.tiles().contains(&t)
        });
        assert_full_rows(&p);
        let captions: Vec<_> = p
            .iter()
            .filter_map(|p| match *p {
                Placement::Caption { group, row, .. } => Some((group, row)),
                _ => None,
            })
            .collect();
        assert_eq!(
            captions,
            [(TileGroup::Focus, 0), (TileGroup::Services, 0)],
            "two two-tile groups sit side by side"
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
