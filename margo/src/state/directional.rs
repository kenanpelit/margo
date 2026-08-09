//! Spatial directional window focus, with a workspace-switch fallback.
//!
//! Ports mango 0.15.5's `focus_window_or_workspace`: focus the nearest window
//! in a given direction, or — when there is no window that way (the focused
//! window sits at the edge of its tag) — switch to the adjacent workspace
//! instead. margo's existing `focusdir` is stack-cycling (`focus_stack`), which
//! wraps and so never "runs out" at an edge; this adds the true spatial-
//! neighbour selection the workspace fallback needs.
//!
//! The workspace fallback itself ports mango 0.15.6's `focus_window_or_workspace
//! re impl`: instead of blindly switching to the next/previous tag (which can
//! land on an empty one), it first looks for the *nearest* tag in that
//! direction that actually has clients, with `tag_carousel` wraparound, and
//! only falls back to the blind next/prev switch if none are occupied.

use super::*;
use crate::layout::Rect;
use margo_config::Direction;

/// Doubled centre of a rect (`2x+w`, `2y+h`) — keeps the midpoint
/// integer-exact so neighbour comparisons never round.
fn dcenter(r: Rect) -> (i64, i64) {
    (
        2 * r.x as i64 + r.width as i64,
        2 * r.y as i64 + r.height as i64,
    )
}

/// Index of the nearest `candidates` rect in `dir` from `focused`, or `None`
/// if nothing lies that way.
///
/// A candidate counts as "in `dir`" when its centre is strictly past the
/// focused centre on that axis; among those the winner minimises primary-axis
/// distance plus twice the perpendicular offset, so a window level with the
/// origin beats a farther diagonal one (going Right from the master lands on
/// the stack window beside it, not a bottom corner). `Direction::None` and an
/// empty candidate set both yield `None`.
pub(crate) fn spatial_neighbor(
    focused: Rect,
    candidates: &[(usize, Rect)],
    dir: Direction,
) -> Option<usize> {
    let (fx, fy) = dcenter(focused);
    candidates
        .iter()
        .filter_map(|&(idx, r)| {
            let (cx, cy) = dcenter(r);
            let (primary, perp) = match dir {
                Direction::Left if cx < fx => (fx - cx, (cy - fy).abs()),
                Direction::Right if cx > fx => (cx - fx, (cy - fy).abs()),
                Direction::Up if cy < fy => (fy - cy, (cx - fx).abs()),
                Direction::Down if cy > fy => (cy - fy, (cx - fx).abs()),
                _ => return None,
            };
            Some((idx, primary + 2 * perp))
        })
        .min_by_key(|&(_, score)| score)
        .map(|(idx, _)| idx)
}

/// Nearest tag number in `dir` from `current` (both 1-indexed) that
/// `occupied` reports as non-empty, or `None` if none are. When nothing
/// is found in `dir` but `carousel` is enabled, wraps to search from the
/// far end back toward (but not including) `current`. The returned bool
/// is `true` for a wrapped hit — the caller uses it to pick the carousel
/// slide direction, same as mango 0.15.6's `view_shift_tag_have_client`.
fn nearest_occupied_tag(
    current: u32,
    max: u32,
    dir: Direction,
    carousel: bool,
    occupied: impl Fn(u32) -> bool,
) -> Option<(u32, bool)> {
    match dir {
        Direction::Left | Direction::Up => {
            for n in (1..current).rev() {
                if occupied(n) {
                    return Some((n, false));
                }
            }
            if carousel {
                for n in (current + 1..=max).rev() {
                    if occupied(n) {
                        return Some((n, true));
                    }
                }
            }
            None
        }
        Direction::Right | Direction::Down => {
            for n in current + 1..=max {
                if occupied(n) {
                    return Some((n, false));
                }
            }
            if carousel {
                for n in 1..current {
                    if occupied(n) {
                        return Some((n, true));
                    }
                }
            }
            None
        }
        Direction::None => None,
    }
}

impl MargoState {
    /// Focus the window in `dir`; if none is there (edge of the tag), jump to
    /// the nearest tag in that direction that has clients, falling back to
    /// the blind next/previous tag if none are occupied. Ports mango 0.15.5's
    /// `focus_window_or_workspace`, updated with 0.15.6's re-implementation
    /// of the workspace fallback (see module docs).
    pub fn focus_window_or_workspace(&mut self, dir: Direction) {
        let mon_idx = self.focused_monitor();
        if mon_idx >= self.monitors.len() {
            return;
        }
        let tagset = self.monitors[mon_idx].current_tagset();

        let focused = self
            .focused_client_idx()
            .filter(|&i| self.clients[i].is_visible_on(mon_idx, tagset))
            .map(|i| (i, self.clients[i].geom));

        if let Some((focused_idx, focused_geom)) = focused {
            let candidates: Vec<(usize, Rect)> = self
                .clients
                .iter()
                .enumerate()
                .filter(|(i, c)| *i != focused_idx && c.is_visible_on(mon_idx, tagset))
                .map(|(i, c)| (i, c.geom))
                .collect();
            if let Some(idx) = spatial_neighbor(focused_geom, &candidates, dir) {
                self.monitors[mon_idx].prev_selected = self.monitors[mon_idx].selected;
                self.monitors[mon_idx].selected = Some(idx);
                let window = self.clients[idx].window.clone();
                self.focus_surface(Some(FocusTarget::Window(window)));
                self.arrange_monitor(mon_idx);
                return;
            }
        }

        // No neighbour that way (or nothing focused) — jump to the nearest
        // occupied tag in that direction rather than blindly stepping one
        // tag over, which can land on an empty one.
        let max = crate::MAX_TAGS as u32;
        let current_tag = if tagset.count_ones() == 1 {
            tagset.trailing_zeros() + 1
        } else {
            1
        };
        let carousel = self.config.tag_carousel;
        let found = nearest_occupied_tag(current_tag, max, dir, carousel, |n| {
            self.clients
                .iter()
                .any(|c| c.is_visible_on(mon_idx, 1u32 << (n - 1)))
        });
        if let Some((target, wrapped)) = found {
            if wrapped {
                self.tag_carousel_dir = match dir {
                    Direction::Left | Direction::Up => -1,
                    Direction::Right | Direction::Down => 1,
                    Direction::None => 0,
                };
            }
            self.view_tag(1u32 << (target - 1));
            return;
        }

        // Nothing occupied that way either — blind next/previous tag.
        match dir {
            Direction::Left | Direction::Up => self.view_relative(-1),
            Direction::Right | Direction::Down => self.view_relative(1),
            Direction::None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    // Classic master (left, full height) + two stacked windows on the right.
    //   idx 0: master  (0,0 100x300)
    //   idx 1: stack-top (100,0 100x150)
    //   idx 2: stack-bottom (100,150 100x150)
    fn master_stack() -> (Rect, Vec<(usize, Rect)>) {
        let master = r(0, 0, 100, 300);
        let cands = vec![(1, r(100, 0, 100, 150)), (2, r(100, 150, 100, 150))];
        (master, cands)
    }

    #[test]
    fn right_from_master_picks_the_aligned_stack_window() {
        let (master, cands) = master_stack();
        // master centre y is level with the *seam* between the two stack
        // windows; the tie-break weights perpendicular offset equally for
        // both, so this asserts a member of the stack column, not the master.
        let got = spatial_neighbor(master, &cands, Direction::Right);
        assert!(matches!(got, Some(1) | Some(2)), "got {got:?}");
    }

    #[test]
    fn left_from_master_has_no_neighbour() {
        let (master, cands) = master_stack();
        assert_eq!(spatial_neighbor(master, &cands, Direction::Left), None);
    }

    #[test]
    fn left_from_stack_top_picks_master() {
        let (master, _) = master_stack();
        let stack_top = r(100, 0, 100, 150);
        let cands = vec![(0, master), (2, r(100, 150, 100, 150))];
        assert_eq!(
            spatial_neighbor(stack_top, &cands, Direction::Left),
            Some(0)
        );
    }

    #[test]
    fn down_picks_lower_up_picks_higher_within_the_stack() {
        let stack_top = r(100, 0, 100, 150);
        let cands = vec![(0, r(0, 0, 100, 300)), (2, r(100, 150, 100, 150))];
        assert_eq!(
            spatial_neighbor(stack_top, &cands, Direction::Down),
            Some(2)
        );
        // Nothing sits above the top stack window.
        assert_eq!(spatial_neighbor(stack_top, &cands, Direction::Up), None);
    }

    #[test]
    fn right_prefers_the_vertically_aligned_neighbour() {
        // Origin centred at y=150. Candidate A is level with it; B is far below.
        let focused = r(0, 100, 100, 100);
        let cands = vec![
            (10, r(200, 100, 100, 100)), // level → perp 0
            (11, r(200, 300, 100, 100)), // far below → large perp
        ];
        assert_eq!(
            spatial_neighbor(focused, &cands, Direction::Right),
            Some(10)
        );
    }

    #[test]
    fn no_candidates_is_none() {
        let focused = r(0, 0, 100, 100);
        for dir in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            assert_eq!(spatial_neighbor(focused, &[], dir), None);
        }
    }

    #[test]
    fn none_direction_is_none() {
        let (master, cands) = master_stack();
        assert_eq!(spatial_neighbor(master, &cands, Direction::None), None);
    }

    #[test]
    fn a_window_at_the_same_centre_is_in_no_direction() {
        let focused = r(0, 0, 100, 100);
        let cands = vec![(9, r(0, 0, 100, 100))]; // identical centre
        for dir in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            assert_eq!(spatial_neighbor(focused, &cands, dir), None, "{dir:?}");
        }
    }

    fn occupied_set(tags: &[u32]) -> impl Fn(u32) -> bool + '_ {
        move |n| tags.contains(&n)
    }

    #[test]
    fn skips_empty_tags_to_the_nearest_occupied_one() {
        // On tag 5, tags 6-9 empty, tag 8 occupied — Right should land on 8,
        // not blindly step to the empty tag 6.
        let occ = occupied_set(&[8]);
        assert_eq!(
            nearest_occupied_tag(5, 9, Direction::Right, false, occ),
            Some((8, false))
        );
    }

    #[test]
    fn adjacent_occupied_tag_wins_over_a_farther_one() {
        let occ = occupied_set(&[3, 7]);
        assert_eq!(
            nearest_occupied_tag(5, 9, Direction::Left, false, occ),
            Some((3, false))
        );
    }

    #[test]
    fn no_occupied_tag_that_way_is_none_without_carousel() {
        let occ = occupied_set(&[2]);
        assert_eq!(
            nearest_occupied_tag(5, 9, Direction::Right, false, occ),
            None
        );
    }

    #[test]
    fn carousel_wraps_to_the_far_end_when_nothing_found_directly() {
        // From tag 8 going Right, nothing occupied in 9; wraps to the
        // nearest occupied tag counting in from tag 1 — tag 2.
        let occ = occupied_set(&[2, 4]);
        assert_eq!(
            nearest_occupied_tag(8, 9, Direction::Right, true, occ),
            Some((2, true))
        );
    }

    #[test]
    fn none_direction_finds_nothing() {
        let occ = occupied_set(&[1, 2, 3]);
        assert_eq!(nearest_occupied_tag(5, 9, Direction::None, true, occ), None);
    }
}
