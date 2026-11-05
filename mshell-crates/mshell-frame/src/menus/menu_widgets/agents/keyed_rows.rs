//! Row lists that survive a poll.
//!
//! The Agents and GitHub menus refresh every few seconds. Tearing the rows
//! down and building them again on every poll dropped the keyboard focus
//! (the focused button was destroyed) and could jump the list. A
//! [`KeyedList`] instead keeps one row per key: a row whose key is still
//! present is updated in place and moved if its position changed; only
//! rows for new keys are built and only rows for gone keys are removed.

use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use std::collections::HashMap;
use std::hash::Hash;

/// Where a row of the new list comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// The existing row at this index of the old list.
    Keep(usize),
    /// A new row for the item at this index of the new list.
    Create(usize),
}

/// How to get from the rows for `old` keys to the rows for `new` keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowPlan {
    /// Old rows to drop (indices into `old`), ascending.
    pub(crate) remove: Vec<usize>,
    /// The new list, in order.
    pub(crate) order: Vec<Slot>,
}

/// Match rows by key. A key that appears twice in `new` keeps its row for
/// the first occurrence and gets a fresh row for the others.
pub(crate) fn plan_rows<K: Eq + Hash>(old: &[K], new: &[K]) -> RowPlan {
    let mut by_key: HashMap<&K, usize> = HashMap::with_capacity(old.len());
    for (i, k) in old.iter().enumerate() {
        by_key.entry(k).or_insert(i);
    }
    let mut kept = vec![false; old.len()];
    let order = new
        .iter()
        .enumerate()
        .map(|(j, k)| match by_key.remove(k) {
            Some(i) => {
                kept[i] = true;
                Slot::Keep(i)
            }
            None => Slot::Create(j),
        })
        .collect();
    let remove = (0..old.len()).filter(|i| !kept[*i]).collect();
    RowPlan { remove, order }
}

/// One row of a [`KeyedList`].
pub(crate) trait KeyedRow {
    type Item;
    type Key: Eq + Hash + Clone;

    fn key(item: &Self::Item) -> Self::Key;
    fn widget(&self) -> gtk::Widget;
    /// Show `item` (same key as the row's) without rebuilding the row.
    fn update(&self, item: &Self::Item);
}

/// The rows of one `gtk::Box`, kept by key across refreshes. The box must
/// hold only these rows.
pub(crate) struct KeyedList<R: KeyedRow> {
    container: gtk::Box,
    rows: Vec<(R::Key, R)>,
}

impl<R: KeyedRow> KeyedList<R> {
    pub(crate) fn new(container: gtk::Box) -> Self {
        KeyedList {
            container,
            rows: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Make the rows show `items`, in order. `create` builds a row for a key
    /// that has none yet; existing rows get [`KeyedRow::update`].
    pub(crate) fn sync(&mut self, items: &[R::Item], mut create: impl FnMut(&R::Item) -> R) {
        let old_keys: Vec<R::Key> = self.rows.iter().map(|(k, _)| k.clone()).collect();
        let new_keys: Vec<R::Key> = items.iter().map(R::key).collect();
        let plan = plan_rows(&old_keys, &new_keys);
        let mut old: Vec<Option<(R::Key, R)>> = self.rows.drain(..).map(Some).collect();
        for i in plan.remove {
            if let Some((_, row)) = old[i].take() {
                self.container.remove(&row.widget());
            }
        }
        let mut prev: Option<gtk::Widget> = None;
        for (j, slot) in plan.order.into_iter().enumerate() {
            let item = &items[j];
            let entry = match slot {
                Slot::Keep(i) => old[i].take().inspect(|(_, row)| row.update(item)),
                Slot::Create(_) => {
                    let row = create(item);
                    self.container.append(&row.widget());
                    Some((new_keys[j].clone(), row))
                }
            };
            let Some(entry) = entry else { continue };
            let w = entry.1.widget();
            // Only move a row that is out of place: moving the focused
            // row is harmless, but there is no reason to touch the rest.
            if w.prev_sibling() != prev {
                self.container.reorder_child_after(&w, prev.as_ref());
            }
            prev = Some(w);
            self.rows.push(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_keys_keep_their_rows() {
        let p = plan_rows(&["a", "b", "c"], &["a", "b", "c"]);
        assert_eq!(p.remove, Vec::<usize>::new());
        assert_eq!(p.order, [Slot::Keep(0), Slot::Keep(1), Slot::Keep(2)]);
    }

    #[test]
    fn new_gone_and_moved_keys() {
        // "b" ended, "d" started, "c" moved to the top.
        let p = plan_rows(&["a", "b", "c"], &["c", "a", "d"]);
        assert_eq!(p.remove, [1]);
        assert_eq!(p.order, [Slot::Keep(2), Slot::Keep(0), Slot::Create(2)]);
    }

    #[test]
    fn empty_lists_and_duplicate_keys() {
        assert_eq!(plan_rows::<&str>(&[], &["x"]).order, [Slot::Create(0)]);
        assert_eq!(plan_rows(&["x", "y"], &[]).remove, [0, 1]);
        let p = plan_rows(&["x"], &["x", "x"]);
        assert_eq!(p.order, [Slot::Keep(0), Slot::Create(1)]);
        assert!(p.remove.is_empty());
    }
}
