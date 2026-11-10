//! Where each card sits on the Today and System tabs. Pure: a hidden card
//! hands its room to its neighbour, so a tab never shows a hole or a short
//! column (DESIGN §7, the owner's "proportional, no empty space" rule).

use mshell_config::schema::dashboard::DashboardCard;

/// Grid width in units (divisible by 1–4).
pub(crate) const UNITS: i32 = 12;

/// One card's cell on a tab grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Slot {
    pub(crate) card: DashboardCard,
    pub(crate) col: i32,
    pub(crate) row: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

/// Lay the shown cards of one row side by side, sharing [`UNITS`] evenly.
fn row(cards: &[DashboardCard], row: i32, shows: &impl Fn(DashboardCard) -> bool) -> Vec<Slot> {
    let shown: Vec<DashboardCard> = cards.iter().copied().filter(|c| shows(*c)).collect();
    if shown.is_empty() {
        return Vec::new();
    }
    let width = UNITS / shown.len() as i32;
    shown
        .into_iter()
        .enumerate()
        .map(|(i, card)| Slot {
            card,
            col: width * i as i32,
            row,
            width,
            height: 1,
        })
        .collect()
}

/// **Today**: calendar | agenda on top; weather on the left beside media
/// over timers below (the weather card is about as tall as the two
/// smaller cards together, so both columns end level).
pub(crate) fn today(shows: impl Fn(DashboardCard) -> bool) -> Vec<Slot> {
    use DashboardCard::{Agenda, Calendar, Media, Timers, Weather};
    let mut out = row(&[Calendar, Agenda], 0, &shows);
    let half = UNITS / 2;
    let right: Vec<DashboardCard> = [Media, Timers].into_iter().filter(|c| shows(*c)).collect();
    if !shows(Weather) {
        out.extend(row(&right, 1, &shows));
        return out;
    }
    if right.is_empty() {
        out.extend(row(&[Weather], 1, &shows));
        return out;
    }
    out.push(Slot {
        card: Weather,
        col: 0,
        row: 1,
        width: half,
        height: 2,
    });
    if right.len() == 1 {
        out.push(Slot {
            card: right[0],
            col: half,
            row: 1,
            width: half,
            height: 2,
        });
    } else {
        for (i, card) in right.into_iter().enumerate() {
            out.push(Slot {
                card,
                col: half,
                row: 1 + i as i32,
                width: half,
                height: 1,
            });
        }
    }
    out
}

/// **System**: four metric cards across, then battery & power | network,
/// then updates | AI usage.
pub(crate) fn system(shows: impl Fn(DashboardCard) -> bool) -> Vec<Slot> {
    use DashboardCard::{AiUsage, Battery, Cpu, Disk, Memory, Network, Temperature, Updates};
    let rows: [&[DashboardCard]; 3] = [
        &[Cpu, Memory, Temperature, Disk],
        &[Battery, Network],
        &[Updates, AiUsage],
    ];
    let mut out = Vec::new();
    let mut r = 0;
    for cards in rows {
        let slots = row(cards, r, &shows);
        if !slots.is_empty() {
            r += 1;
            out.extend(slots);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use DashboardCard::*;
    use std::collections::HashSet;

    /// No overlap, nothing outside the grid, and every used row is full.
    fn assert_tiled(slots: &[Slot]) {
        let mut cells = HashSet::new();
        let mut rows = std::collections::BTreeMap::<i32, i32>::new();
        for s in slots {
            assert!(s.col >= 0 && s.col + s.width <= UNITS, "{s:?}");
            for c in s.col..s.col + s.width {
                for r in s.row..s.row + s.height {
                    assert!(cells.insert((c, r)), "overlap at {c},{r}");
                    *rows.entry(r).or_default() += 1;
                }
            }
        }
        for (r, used) in rows {
            assert_eq!(used, UNITS, "row {r} has a hole");
        }
    }

    #[test]
    fn today_full_is_two_level_columns() {
        let s = today(|_| true);
        assert_tiled(&s);
        let weather = s.iter().find(|s| s.card == Weather).expect("weather");
        assert_eq!((weather.row, weather.height), (1, 2));
        let media = s.iter().find(|s| s.card == Media).expect("media");
        let timers = s.iter().find(|s| s.card == Timers).expect("timers");
        assert_eq!(media.row + 1, timers.row);
        assert_eq!(timers.row + timers.height, weather.row + weather.height);
    }

    #[test]
    fn today_every_subset_tiles_without_holes() {
        let cards = [Calendar, Agenda, Weather, Media, Timers];
        for mask in 0u32..32 {
            let shown: Vec<_> = cards
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, c)| *c)
                .collect();
            let s = today(|c| shown.contains(&c));
            assert_tiled(&s);
            assert_eq!(s.len(), shown.len(), "mask {mask:b}");
        }
    }

    #[test]
    fn system_every_subset_tiles_without_holes() {
        let cards = [
            Cpu,
            Memory,
            Temperature,
            Disk,
            Battery,
            Network,
            Updates,
            AiUsage,
        ];
        for mask in 0u32..256 {
            let shown: Vec<_> = cards
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, c)| *c)
                .collect();
            let s = system(|c| shown.contains(&c));
            assert_tiled(&s);
            assert_eq!(s.len(), shown.len());
            // Rows are packed from the top: no empty row in between.
            let used: HashSet<i32> = s.iter().map(|s| s.row).collect();
            assert!(used.iter().all(|r| *r < used.len() as i32), "mask {mask:b}");
        }
    }

    #[test]
    fn a_hidden_card_gives_its_room_to_its_neighbour() {
        let s = today(|c| c != Agenda);
        let cal = s.iter().find(|s| s.card == Calendar).expect("calendar");
        assert_eq!(cal.width, UNITS);
        let s = today(|c| c != Media);
        let timers = s.iter().find(|s| s.card == Timers).expect("timers");
        assert_eq!(timers.height, 2, "timers fill the right column");
        let s = system(|c| c != Temperature);
        assert!(s.iter().filter(|s| s.row == 0).all(|s| s.width == 4));
    }
}
