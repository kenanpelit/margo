//! Shared scroll-offset math for read-only lists that don't need a
//! selection cursor — e.g. `screens::sync`'s plan details and
//! `screens::overview`'s config tree. Selectable lists (`screens::modules`,
//! `screens::packages`) use `ratatui::widgets::ListState` instead, which
//! tracks its own offset.
//!
//! Centralized here so the two manual-offset screens can't drift apart on
//! the clamp/hint arithmetic.

/// Clamp a scroll offset so the rendered window never runs past the end of
/// `total` items inside a viewport that can show `visible_height` items.
pub fn clamp_scroll(scroll: usize, total: usize, visible_height: usize) -> usize {
    scroll.min(total.saturating_sub(visible_height))
}

/// Build the `" (n/m) "` scroll-position hint shown in a list title, or an
/// empty string when everything already fits on screen.
pub fn scroll_hint(scroll: usize, total: usize, visible_height: usize) -> String {
    if total > visible_height {
        format!(
            " ({}/{}) ",
            scroll + 1,
            total.saturating_sub(visible_height) + 1
        )
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_scroll_caps_at_max_offset() {
        assert_eq!(clamp_scroll(100, 10, 5), 5);
    }

    #[test]
    fn clamp_scroll_zeroed_when_everything_already_fits() {
        // total (3) <= visible_height (5): nothing to scroll past, so any
        // requested offset clamps back to 0.
        assert_eq!(clamp_scroll(2, 3, 5), 0);
    }

    #[test]
    fn clamp_scroll_passthrough_within_range() {
        // total (20) > visible_height (5): an offset inside the valid
        // scrollable range (0..=15) passes through unchanged.
        assert_eq!(clamp_scroll(7, 20, 5), 7);
    }

    #[test]
    fn clamp_scroll_handles_empty_list() {
        assert_eq!(clamp_scroll(3, 0, 5), 0);
    }

    #[test]
    fn scroll_hint_empty_when_everything_fits() {
        assert_eq!(scroll_hint(0, 3, 5), "");
    }

    #[test]
    fn scroll_hint_shows_position_when_overflowing() {
        assert_eq!(scroll_hint(0, 10, 5), " (1/6) ");
        assert_eq!(scroll_hint(5, 10, 5), " (6/6) ");
    }
}
