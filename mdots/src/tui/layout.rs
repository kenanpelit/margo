use ratatui::layout::Rect;

/// Geometry recorded by the most recent [`crate::tui::ui::render`].
///
/// Mouse hit-testing used to re-derive the sidebar's position from constants
/// copied out of the render code, which rots silently the moment the layout
/// changes. Recording the rects that were actually drawn keeps input and
/// output in step by construction — the click handler can only ever hit what
/// the user can actually see.
#[derive(Clone, Copy, Default)]
pub struct LayoutSnapshot {
    /// Inner area of the sidebar block (border excluded), or `None` when the
    /// sidebar is collapsed and therefore not drawn at all.
    pub sidebar_items: Option<Rect>,
}

/// Whether a terminal cell falls inside `area`.
fn contains(area: Rect, col: u16, row: u16) -> bool {
    col >= area.x
        && col < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

impl LayoutSnapshot {
    /// Index of the sidebar item under (`col`, `row`), if that cell is inside
    /// the drawn sidebar and lands on one of the `item_count` item rows.
    pub fn sidebar_index_at(&self, col: u16, row: u16, item_count: usize) -> Option<usize> {
        let area = self.sidebar_items?;
        if !contains(area, col, row) {
            return None;
        }
        let index = usize::from(row - area.y);
        (index < item_count).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sidebar whose items start at row 4, 18 cells wide, 7 rows tall.
    fn snapshot() -> LayoutSnapshot {
        LayoutSnapshot {
            sidebar_items: Some(Rect::new(1, 4, 18, 7)),
        }
    }

    #[test]
    fn first_sidebar_row_maps_to_index_zero() {
        assert_eq!(snapshot().sidebar_index_at(5, 4, 7), Some(0));
    }

    #[test]
    fn later_sidebar_rows_map_to_their_offset() {
        assert_eq!(snapshot().sidebar_index_at(5, 7, 7), Some(3));
    }

    #[test]
    fn click_above_the_sidebar_items_misses() {
        assert_eq!(snapshot().sidebar_index_at(5, 3, 7), None);
    }

    #[test]
    fn click_right_of_the_sidebar_misses() {
        assert_eq!(snapshot().sidebar_index_at(19, 5, 7), None);
    }

    #[test]
    fn click_past_the_last_item_misses_even_inside_the_block() {
        // The block is 7 rows tall but only 3 items are listed.
        assert_eq!(snapshot().sidebar_index_at(5, 9, 3), None);
    }

    #[test]
    fn collapsed_sidebar_never_matches() {
        let collapsed = LayoutSnapshot {
            sidebar_items: None,
        };
        assert_eq!(collapsed.sidebar_index_at(5, 4, 7), None);
    }
}
