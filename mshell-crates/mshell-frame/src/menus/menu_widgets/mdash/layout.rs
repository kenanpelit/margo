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

/// **Today**: two columns of cards. Left: calendar over weather (both
/// fixed height); right: agenda over media over timers, where the agenda
/// is the flexible card that absorbs the difference, so both columns end
/// level. A column whose cards are all hidden disappears and the other
/// takes the whole width.
pub(crate) fn today(shows: impl Fn(DashboardCard) -> bool) -> Vec<Vec<DashboardCard>> {
    use DashboardCard::{Agenda, Calendar, Media, Timers, Weather};
    [vec![Calendar, Weather], vec![Agenda, Media, Timers]]
        .into_iter()
        .map(|col| col.into_iter().filter(|c| shows(*c)).collect::<Vec<_>>())
        .filter(|col| !col.is_empty())
        .collect()
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
    fn today_is_calendar_and_weather_beside_agenda_media_timers() {
        assert_eq!(
            today(|_| true),
            vec![vec![Calendar, Weather], vec![Agenda, Media, Timers]]
        );
    }

    #[test]
    fn today_every_subset_keeps_every_shown_card_and_no_empty_column() {
        let cards = [Calendar, Agenda, Weather, Media, Timers];
        for mask in 0u32..32 {
            let shown: Vec<_> = cards
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, c)| *c)
                .collect();
            let cols = today(|c| shown.contains(&c));
            assert!(cols.iter().all(|c| !c.is_empty()), "mask {mask:b}");
            let placed: Vec<_> = cols.concat();
            assert_eq!(placed.len(), shown.len(), "mask {mask:b}");
            assert!(shown.iter().all(|c| placed.contains(c)));
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
        // A whole column hidden: the other one spans the tab.
        assert_eq!(today(|c| c == Agenda), vec![vec![Agenda]]);
        let s = system(|c| c != Temperature);
        assert!(s.iter().filter(|s| s.row == 0).all(|s| s.width == 4));
    }
}
