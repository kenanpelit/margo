//! Keyed reconciliation for the dashboard's polled lists.
//!
//! The Processes tab (and the iGPU process list) used to clear their
//! `ListBox` and build every row again on each 2 s poll: about 540 widgets
//! a tick on the GTK thread, a full style pass, lost hover / tooltips, and a
//! click on a row's ✕ could land on a row that had just been replaced.
//! [`KeyedRows`] keeps one row per key (a pid, a group name) across polls:
//! a row that is still wanted is updated in place, only appearing keys get
//! new widgets, only vanished keys are removed, and a re-sort moves the
//! fewest rows (everything on the longest run already in order stays put).
//!
//! The list itself is behind [`RowList`] so the bookkeeping is tested
//! without a display.

use relm4::gtk;
use relm4::gtk::prelude::{Cast, WidgetExt};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// The container the rows live in (a `gtk::ListBox`; a `Vec` in tests).
pub(crate) trait RowList {
    type Row;
    fn insert_at(&self, row: &Self::Row, index: usize);
    fn remove_row(&self, row: &Self::Row);
}

impl RowList for gtk::ListBox {
    type Row = gtk::ListBoxRow;

    fn insert_at(&self, row: &gtk::ListBoxRow, index: usize) {
        self.insert(row, i32::try_from(index).unwrap_or(-1));
    }

    fn remove_row(&self, row: &gtk::ListBoxRow) {
        // Only rows this list still owns (a row is never in two lists).
        if row.parent().as_ref() == Some(self.upcast_ref()) {
            self.remove(row);
        }
    }
}

/// How much one [`KeyedRows::sync`] touched the list.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SyncStats {
    /// Rows built for keys that just appeared.
    pub created: usize,
    /// Rows dropped for keys that vanished.
    pub removed: usize,
    /// Existing rows taken out and put back at a new position.
    pub moved: usize,
}

/// Rows keyed by `K`, each holding its widgets `R`, in on-screen order.
pub(crate) struct KeyedRows<K, R> {
    order: Vec<K>,
    rows: HashMap<K, R>,
}

impl<K, R> Default for KeyedRows<K, R> {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            rows: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash + Clone, R> KeyedRows<K, R> {
    /// Make `list` show `desired` (in that order). `create` builds the
    /// widgets for a new key; `update` refreshes a kept row in place (it is
    /// also called on fresh rows, so `create` may leave the values empty);
    /// `handle` gives the row widget the list holds. A key listed twice
    /// keeps its first position.
    pub(crate) fn sync<V, L: RowList>(
        &mut self,
        list: &L,
        desired: Vec<(K, V)>,
        mut create: impl FnMut(&K, &V) -> R,
        mut update: impl FnMut(&R, &V),
        handle: impl Fn(&R) -> &L::Row,
    ) -> SyncStats {
        let mut stats = SyncStats::default();
        let mut seen = HashSet::with_capacity(desired.len());
        let desired: Vec<(K, V)> = desired
            .into_iter()
            .filter(|(k, _)| seen.insert(k.clone()))
            .collect();
        let target: HashMap<&K, usize> = desired
            .iter()
            .enumerate()
            .map(|(i, (k, _))| (k, i))
            .collect();

        // 1. Drop rows whose key vanished.
        let mut kept = Vec::with_capacity(self.order.len());
        for k in std::mem::take(&mut self.order) {
            if target.contains_key(&k) {
                kept.push(k);
            } else if let Some(r) = self.rows.remove(&k) {
                list.remove_row(handle(&r));
                stats.removed += 1;
            }
        }

        // 2. Kept rows on the longest run already in the new order stay;
        //    the rest come out to be re-inserted.
        let positions: Vec<usize> = kept.iter().map(|k| target[k]).collect();
        let stay: HashSet<&K> = longest_increasing(&positions)
            .into_iter()
            .map(|i| &kept[i])
            .collect();
        let mut displaced: HashSet<K> = HashSet::new();
        for k in &kept {
            if !stay.contains(k) {
                if let Some(r) = self.rows.get(k) {
                    list.remove_row(handle(r));
                }
                displaced.insert(k.clone());
                stats.moved += 1;
            }
        }

        // 3. Walk the new order: the list now holds exactly the staying
        //    rows, in order, so inserting the others at their index lands
        //    every row in place.
        let mut order = Vec::with_capacity(desired.len());
        for (i, (k, v)) in desired.iter().enumerate() {
            match self.rows.get(k) {
                Some(r) => {
                    update(r, v);
                    if displaced.contains(k) {
                        list.insert_at(handle(r), i);
                    }
                }
                None => {
                    let r = create(k, v);
                    update(&r, v);
                    list.insert_at(handle(&r), i);
                    self.rows.insert(k.clone(), r);
                    stats.created += 1;
                }
            }
            order.push(k.clone());
        }
        self.order = order;
        stats
    }

    /// Rows currently shown.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.order.len()
    }
}

/// Indices into `seq` of one longest strictly increasing subsequence
/// (patience sorting, O(n log n)).
fn longest_increasing(seq: &[usize]) -> Vec<usize> {
    // tails[l] = index into seq of the smallest tail of a run of length l+1.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; seq.len()];
    for (i, &x) in seq.iter().enumerate() {
        let at = tails.partition_point(|&t| seq[t] < x);
        if at > 0 {
            prev[i] = Some(tails[at - 1]);
        }
        if at == tails.len() {
            tails.push(i);
        } else {
            tails[at] = i;
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut cur = tails.last().copied();
    while let Some(i) = cur {
        out.push(i);
        cur = prev[i];
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// A row "widget": an id that is unique per construction, so identity
    /// survives (or not) across syncs exactly like a GTK object would.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeRow {
        id: u32,
        text: Rc<RefCell<String>>,
    }

    #[derive(Default)]
    struct FakeList {
        rows: RefCell<Vec<u32>>,
        ops: Cell<usize>,
    }

    impl RowList for FakeList {
        type Row = FakeRow;
        fn insert_at(&self, row: &FakeRow, index: usize) {
            let mut rows = self.rows.borrow_mut();
            assert!(!rows.contains(&row.id), "row inserted twice");
            let at = index.min(rows.len());
            rows.insert(at, row.id);
            self.ops.set(self.ops.get() + 1);
        }
        fn remove_row(&self, row: &FakeRow) {
            self.rows.borrow_mut().retain(|r| *r != row.id);
            self.ops.set(self.ops.get() + 1);
        }
    }

    struct Harness {
        list: FakeList,
        keyed: KeyedRows<i32, FakeRow>,
        next_id: u32,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                list: FakeList::default(),
                keyed: KeyedRows::default(),
                next_id: 0,
            }
        }

        fn sync(&mut self, rows: &[(i32, &str)]) -> SyncStats {
            let desired = rows.iter().map(|(k, v)| (*k, v.to_string())).collect();
            let next = &mut self.next_id;
            self.keyed.sync(
                &self.list,
                desired,
                |_, _| {
                    *next += 1;
                    FakeRow {
                        id: *next,
                        text: Rc::new(RefCell::new(String::new())),
                    }
                },
                |r, v| *r.text.borrow_mut() = v.clone(),
                |r| r,
            )
        }

        /// Keys and texts in on-screen order.
        fn shown(&self) -> Vec<(i32, String)> {
            self.list
                .rows
                .borrow()
                .iter()
                .map(|id| {
                    let (k, r) = self
                        .keyed
                        .rows
                        .iter()
                        .find(|(_, r)| r.id == *id)
                        .expect("every listed row is tracked");
                    (*k, r.text.borrow().clone())
                })
                .collect()
        }

        fn ids(&self) -> Vec<u32> {
            self.list.rows.borrow().clone()
        }
    }

    #[test]
    fn same_keys_rebuild_nothing_and_update_in_place() {
        let mut h = Harness::new();
        let first = h.sync(&[(10, "a 1%"), (20, "b 2%"), (30, "c 3%")]);
        assert_eq!(first.created, 3);
        let ids = h.ids();
        let ops = h.list.ops.get();

        // Same pids, new values: no row built, removed or moved.
        let second = h.sync(&[(10, "a 5%"), (20, "b 0%"), (30, "c 9%")]);
        assert_eq!(second, SyncStats::default());
        assert_eq!(h.ids(), ids, "row identity kept");
        assert_eq!(h.list.ops.get(), ops, "the list was not touched");
        assert_eq!(
            h.shown(),
            vec![
                (10, "a 5%".into()),
                (20, "b 0%".into()),
                (30, "c 9%".into())
            ]
        );
    }

    #[test]
    fn only_appearing_and_vanishing_keys_change() {
        let mut h = Harness::new();
        h.sync(&[(1, "a"), (2, "b"), (3, "c")]);
        let ids = h.ids();
        let stats = h.sync(&[(1, "a"), (3, "c"), (4, "d")]);
        assert_eq!(
            stats,
            SyncStats {
                created: 1,
                removed: 1,
                moved: 0
            }
        );
        let now = h.ids();
        assert_eq!(now[0], ids[0]);
        assert_eq!(now[1], ids[2]);
        assert_eq!(
            h.shown().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            [1, 3, 4]
        );
    }

    #[test]
    fn a_resort_moves_the_fewest_rows() {
        let mut h = Harness::new();
        h.sync(&[(1, ""), (2, ""), (3, ""), (4, ""), (5, "")]);
        let ids = h.ids();
        // 5 jumps to the top: one move, not four.
        let stats = h.sync(&[(5, ""), (1, ""), (2, ""), (3, ""), (4, "")]);
        assert_eq!(stats.moved, 1);
        assert_eq!(stats.created + stats.removed, 0);
        assert_eq!(h.ids(), vec![ids[4], ids[0], ids[1], ids[2], ids[3]]);

        // A full reversal keeps one row and moves the rest; all reused.
        let stats = h.sync(&[(4, ""), (3, ""), (2, ""), (1, ""), (5, "")]);
        assert_eq!(stats.created, 0);
        assert_eq!(
            h.shown().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            [4, 3, 2, 1, 5]
        );
    }

    #[test]
    fn mixed_changes_land_in_order() {
        let mut h = Harness::new();
        h.sync(&[(1, ""), (2, ""), (3, ""), (4, ""), (5, ""), (6, "")]);
        h.sync(&[(7, ""), (6, ""), (2, ""), (8, ""), (1, ""), (4, "")]);
        assert_eq!(
            h.shown().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            [7, 6, 2, 8, 1, 4]
        );
        assert_eq!(h.keyed.len(), 6);
        // Emptying the list removes every row; refilling builds anew.
        let stats = h.sync(&[]);
        assert_eq!(stats.removed, 6);
        assert!(h.ids().is_empty());
        assert_eq!(h.sync(&[(1, "x")]).created, 1);
    }

    #[test]
    fn duplicate_keys_keep_the_first() {
        let mut h = Harness::new();
        h.sync(&[(1, "first"), (2, ""), (1, "again")]);
        assert_eq!(
            h.shown(),
            vec![(1, "first".to_string()), (2, String::new())]
        );
    }

    #[test]
    fn longest_increasing_run() {
        assert_eq!(longest_increasing(&[]), Vec::<usize>::new());
        assert_eq!(longest_increasing(&[0, 1, 2]), vec![0, 1, 2]);
        assert_eq!(longest_increasing(&[4, 0, 1, 2, 3]), vec![1, 2, 3, 4]);
        assert_eq!(longest_increasing(&[2, 1, 0]).len(), 1);
        let seq = [3, 0, 4, 1, 5, 2, 6];
        let run = longest_increasing(&seq);
        assert_eq!(run.len(), 4);
        assert!(run.windows(2).all(|w| seq[w[0]] < seq[w[1]]));
    }
}
