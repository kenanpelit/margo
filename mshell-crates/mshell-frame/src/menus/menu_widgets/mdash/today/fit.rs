//! How many whole rows of the Today card fit its height (pure).
//!
//! The Today card is the right column's one flexible card: it gets the
//! height the left column leaves. Its content is a list of sections
//! (Events, then Notifications), each a run of items — captions, rows, or
//! one empty-state line. A row is shown whole or not at all: whatever does
//! not fit is folded into one "+N more" line per section, so the card never
//! shows a row cut in half. Sections fill the height in order, but every
//! section keeps at least one line — its empty state, or a summary line
//! ("3 new notifications") standing in for its rows — so a busy day never
//! pushes the notifications out of sight.
//!
//! When everything fits and room remains, the leftover is reported so the
//! caller can hand it to a neighbour (the idle media card grows more
//! station rows) instead of leaving an empty card.

/// What an item is; only [`Kind::Row`] items are counted in "+N more".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A heading (a day, "Notifications"): never shown without the row
    /// that follows it.
    Caption,
    Row,
    /// A one-line empty state ("No events this week").
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) kind: Kind,
    pub(crate) height: i32,
}

/// One section's content and the height of its "+N more" line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Section {
    pub(crate) items: Vec<Item>,
    pub(crate) more_height: i32,
    /// Rows past the built items (never shown, always in "+N more").
    pub(crate) beyond: usize,
}

/// What one section shows: the first `shown` items, then — when `more`
/// rows are hidden — a "+`more` more" line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct SectionPlan {
    pub(crate) shown: usize,
    pub(crate) more: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) sections: Vec<SectionPlan>,
    /// Height left unused below the last shown line (≥ 0).
    pub(crate) leftover: i32,
}

impl Plan {
    /// Every item is shown (nothing folded into "+N more").
    pub(crate) fn complete(&self) -> bool {
        self.sections.iter().all(|s| s.more == 0)
    }
}

/// Rows hidden when only `shown` items of `section` are.
fn hidden_rows(section: &Section, shown: usize) -> usize {
    section.items[shown..]
        .iter()
        .filter(|i| i.kind == Kind::Row)
        .count()
        + section.beyond
}

/// The cost of a section showing `shown` items (each child costs its
/// height plus one `spacing`; the box's trailing spacing is paid once by
/// the caller).
fn cost(section: &Section, shown: usize, spacing: i32) -> i32 {
    let items: i32 = section.items[..shown]
        .iter()
        .map(|i| i.height + spacing)
        .sum();
    let more = if hidden_rows(section, shown) > 0 {
        section.more_height + spacing
    } else {
        0
    };
    items + more
}

/// The prefix lengths a section may stop at: never right after a caption
/// (a heading without its row), never inside nothing-but-captions.
fn stops(section: &Section) -> impl DoubleEndedIterator<Item = usize> + '_ {
    (0..=section.items.len()).filter(move |&n| n == 0 || section.items[n - 1].kind != Kind::Caption)
}

/// The smallest showing of a section: nothing but its summary line when
/// it has rows to fold; a section without rows (an empty state) whole.
fn minimum(section: &Section) -> usize {
    if hidden_rows(section, 0) > 0 {
        0
    } else {
        section.items.len()
    }
}

/// Fit `sections` into `avail` px of a vertical box with `spacing`
/// between children.
pub(crate) fn plan(sections: &[Section], avail: i32, spacing: i32) -> Plan {
    // Paying one spacing per child, the box holds `avail + spacing`.
    let budget = avail + spacing;
    let full: i32 = sections
        .iter()
        .map(|s| cost(s, s.items.len(), spacing))
        .sum();
    if full <= budget {
        return Plan {
            sections: sections
                .iter()
                .map(|s| SectionPlan {
                    shown: s.items.len(),
                    more: s.beyond,
                })
                .collect(),
            leftover: (budget - full).max(0),
        };
    }

    let floor: Vec<i32> = sections
        .iter()
        .map(|s| cost(s, minimum(s), spacing))
        .collect();
    let mut used = 0;
    let mut out = Vec::with_capacity(sections.len());
    for (i, section) in sections.iter().enumerate() {
        let reserve: i32 = floor[i + 1..].iter().sum();
        let room = budget - used - reserve;
        // The longest stop that fits; else just "+N more"; else nothing.
        let shown = stops(section)
            .rev()
            .find(|&n| cost(section, n, spacing) <= room);
        let shown = match shown {
            Some(n) => n,
            None => {
                out.push(SectionPlan::default());
                continue;
            }
        };
        used += cost(section, shown, spacing);
        out.push(SectionPlan {
            shown,
            more: hidden_rows(section, shown),
        });
    }
    Plan {
        sections: out,
        leftover: (budget - used).max(0),
    }
}

/// Filler after everything else fits (the "Earlier" — read —
/// notifications): the longest prefix of `items` that fits `room` px
/// (each item costing its height + `spacing`), never ending on a caption
/// and with no fold line — filler simply stops.
pub(crate) fn filler(items: &[Item], room: i32, spacing: i32) -> usize {
    let section = Section {
        items: items.to_vec(),
        more_height: 0,
        beyond: 0,
    };
    stops(&section)
        .rev()
        .find(|&n| {
            section.items[..n]
                .iter()
                .map(|i| i.height + spacing)
                .sum::<i32>()
                <= room
        })
        .unwrap_or(0)
}

/// How many extra rows (each `row` px, `spacing` apart) a neighbour can
/// take from `leftover`, at most `available`.
pub(crate) fn spill_rows(leftover: i32, row: i32, spacing: i32, available: usize) -> usize {
    if row + spacing <= 0 {
        return 0;
    }
    ((leftover.max(0) / (row + spacing)) as usize).min(available)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i32 = 4;

    fn cap() -> Item {
        Item {
            kind: Kind::Caption,
            height: 16,
        }
    }
    fn row() -> Item {
        Item {
            kind: Kind::Row,
            height: 24,
        }
    }
    fn empty() -> Item {
        Item {
            kind: Kind::Empty,
            height: 20,
        }
    }
    fn section(items: Vec<Item>) -> Section {
        Section {
            items,
            more_height: 20,
            beyond: 0,
        }
    }
    /// The pixel height a plan's shown lines take.
    fn height(sections: &[Section], plan: &Plan) -> i32 {
        let mut children = Vec::new();
        for (s, p) in sections.iter().zip(&plan.sections) {
            children.extend(s.items[..p.shown].iter().map(|i| i.height));
            if p.more > 0 {
                children.push(s.more_height);
            }
        }
        children.iter().sum::<i32>() + S * (children.len() as i32 - 1).max(0)
    }

    #[test]
    fn everything_fits_and_the_rest_is_leftover() {
        let s = [section(vec![row(), row()]), section(vec![empty()])];
        let p = plan(&s, 200, S);
        assert!(p.complete());
        assert_eq!(p.sections[0].shown, 2);
        assert_eq!(p.leftover, 200 - height(&s, &p));
    }

    /// The laptop shot's bug: a third event half visible. Now: two whole
    /// rows and "+1 more", never a cut row.
    #[test]
    fn rows_are_whole_or_folded_into_more() {
        let s = [section(vec![row(), row(), row()])];
        // Room for two rows + the more line, not three rows.
        let avail = 24 + S + 24 + S + 20;
        let p = plan(&s, avail, S);
        assert_eq!(p.sections, [SectionPlan { shown: 2, more: 1 }]);
        assert!(height(&s, &p) <= avail);
    }

    #[test]
    fn a_caption_never_ends_a_section() {
        // Today's row, then "Tomorrow" + its row: room for the first row,
        // the caption and "+N more" but not the second row.
        let s = [section(vec![row(), cap(), row()])];
        let avail = 24 + S + 16 + S + 20;
        let p = plan(&s, avail, S);
        assert_eq!(
            p.sections[0],
            SectionPlan { shown: 1, more: 1 },
            "the caption goes with its row"
        );
    }

    #[test]
    fn busy_events_fill_first_and_the_notifications_keep_one_line() {
        let events = section(vec![row(); 10]);
        let notes = section(vec![cap(), row(), row(), row()]);
        let s = [events, notes];
        let avail = 200;
        let p = plan(&s, avail, S);
        // Five events + "+5 more", then "3 new notifications".
        assert_eq!(p.sections[0], SectionPlan { shown: 5, more: 5 });
        assert_eq!(p.sections[1], SectionPlan { shown: 0, more: 3 });
        assert!(height(&s, &p) <= avail);
        assert!(p.leftover < 24 + S);
    }

    #[test]
    fn room_left_after_the_events_goes_to_the_notifications() {
        let events = section(vec![row(), row()]);
        let notes = section(vec![cap(), row(), row(), row()]);
        let s = [events, notes];
        // Both events, the caption, one notification, "+2 more".
        let avail = 2 * (24 + S) + (16 + S) + (24 + S) + 20;
        let p = plan(&s, avail, S);
        assert_eq!(p.sections[0], SectionPlan { shown: 2, more: 0 });
        assert_eq!(p.sections[1], SectionPlan { shown: 2, more: 2 });
    }

    #[test]
    fn plans_never_overflow_and_rows_are_counted() {
        for avail in 0..400 {
            for n_events in 0..6 {
                for n_notes in 0..5 {
                    let mut ev: Vec<Item> = Vec::new();
                    for i in 0..n_events {
                        if i % 2 == 1 {
                            ev.push(cap());
                        }
                        ev.push(row());
                    }
                    if ev.is_empty() {
                        ev.push(empty());
                    }
                    let notes = if n_notes == 0 {
                        vec![empty()]
                    } else {
                        std::iter::once(cap())
                            .chain(std::iter::repeat_n(row(), n_notes))
                            .collect()
                    };
                    let s = [section(ev), section(notes)];
                    let p = plan(&s, avail, S);
                    let h = height(&s, &p);
                    // Only an impossible minimum may overflow: never more
                    // than the floors themselves.
                    let floors: i32 = s.iter().map(|x| cost(x, minimum(x), S)).sum::<i32>() - S;
                    assert!(
                        h <= avail.max(floors),
                        "avail {avail} ev {n_events} notes {n_notes}: {h}"
                    );
                    for (sec, sp) in s.iter().zip(&p.sections) {
                        // (A section with no room even for its fold line
                        // shows nothing at all.)
                        if *sp != SectionPlan::default() {
                            assert_eq!(sp.more, hidden_rows(sec, sp.shown));
                        }
                        if sp.shown > 0 {
                            assert_ne!(sec.items[sp.shown - 1].kind, Kind::Caption);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rows_past_the_built_ones_are_always_counted() {
        let mut notes = section(vec![cap(), row()]);
        notes.beyond = 5;
        let p = plan(std::slice::from_ref(&notes), 500, S);
        assert_eq!(p.sections[0], SectionPlan { shown: 2, more: 5 });
        assert!(!p.complete(), "a folded section is not complete");
        assert!(height(std::slice::from_ref(&notes), &p) <= 500);
    }

    #[test]
    fn filler_stops_at_whole_rows_and_never_on_its_caption() {
        let items = [cap(), row(), row(), row()];
        assert_eq!(filler(&items, 0, S), 0);
        assert_eq!(filler(&items, 16 + S, S), 0, "a caption alone is not shown");
        assert_eq!(filler(&items, 16 + S + 24 + S, S), 2);
        assert_eq!(filler(&items, 16 + S + 2 * (24 + S) + 10, S), 3);
        assert_eq!(filler(&items, 1000, S), 4);
    }

    #[test]
    fn spill_takes_whole_rows_only() {
        assert_eq!(spill_rows(0, 28, 6, 5), 0);
        assert_eq!(spill_rows(33, 28, 6, 5), 0);
        assert_eq!(spill_rows(34, 28, 6, 5), 1);
        assert_eq!(spill_rows(1000, 28, 6, 2), 2, "capped by what exists");
        assert_eq!(spill_rows(-5, 28, 6, 2), 0);
    }
}
