//! How the Sessions page lists saved sessions: filtered, grouped by
//! project folder and sorted (pure; the page renders the result).

use super::LibSession;
use super::classify::LibStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SortKey {
    /// Newest first.
    #[default]
    Date,
    /// A to Z.
    Name,
    /// Projects with the most sessions first.
    Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Sort {
    pub(crate) key: SortKey,
    pub(crate) reversed: bool,
}

impl Sort {
    /// Choosing the active key again reverses it; a new key starts in its
    /// natural order.
    pub(crate) fn choose(self, key: SortKey) -> Sort {
        if key == self.key {
            Sort {
                key,
                reversed: !self.reversed,
            }
        } else {
            Sort {
                key,
                reversed: false,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Filter {
    #[default]
    All,
    /// Moved sessions (resume from their new folder).
    Moved,
    /// Orphan and misplaced sessions.
    Broken,
    /// Sessions with no prompt.
    Empty,
}

impl Filter {
    pub(crate) fn keeps(self, s: &LibSession) -> bool {
        match self {
            Filter::All => true,
            Filter::Moved => matches!(s.status, LibStatus::Moved(_)),
            Filter::Broken => matches!(s.status, LibStatus::Orphan { .. } | LibStatus::Misplaced),
            Filter::Empty => s.empty,
        }
    }
}

/// Sessions per filter button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Counts {
    pub(crate) all: usize,
    pub(crate) moved: usize,
    pub(crate) broken: usize,
    pub(crate) empty: usize,
    /// Empty sessions "Delete empty" would remove (not running, not
    /// locked).
    pub(crate) deletable_empty: usize,
}

pub(crate) fn counts(sessions: &[LibSession]) -> Counts {
    let n = |f: Filter| sessions.iter().filter(|s| f.keeps(s)).count();
    Counts {
        all: sessions.len(),
        moved: n(Filter::Moved),
        broken: n(Filter::Broken),
        empty: n(Filter::Empty),
        deletable_empty: sessions
            .iter()
            .filter(|s| s.empty && !s.running && !s.locked)
            .count(),
    }
}

/// One project folder's sessions.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Group {
    /// The folder shown (where a moved project is now).
    pub(crate) folder: String,
    pub(crate) account: String,
    pub(crate) sessions: Vec<LibSession>,
    pub(crate) last_ms: i64,
    /// One of its sessions is running.
    pub(crate) running: bool,
}

impl Group {
    /// Stable across refreshes (the page keeps it open).
    pub(crate) fn key(&self) -> String {
        format!("{}|{}", self.account, self.folder)
    }
}

/// Whether `s` matches every word of `query` in its title, first
/// prompt or folder (case-insensitive).
pub(crate) fn matches(s: &LibSession, query: &str) -> bool {
    let hay = format!("{}\n{}\n{}", s.title, s.first_prompt, s.folder()).to_lowercase();
    query
        .split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

/// Filter, group by folder and sort.
pub(crate) fn groups(
    sessions: &[LibSession],
    filter: Filter,
    query: &str,
    sort: Sort,
) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for s in sessions
        .iter()
        .filter(|s| filter.keeps(s) && matches(s, query))
    {
        let folder = s.folder();
        let g = match out
            .iter_mut()
            .position(|g| g.folder == folder && g.account == s.account)
        {
            Some(i) => &mut out[i],
            None => {
                out.push(Group {
                    folder,
                    account: s.account.clone(),
                    sessions: Vec::new(),
                    last_ms: 0,
                    running: false,
                });
                let last = out.len() - 1;
                &mut out[last]
            }
        };
        g.last_ms = g.last_ms.max(s.mtime_ms);
        g.running |= s.running;
        g.sessions.push(s.clone());
    }
    let lower = |t: &str| t.to_lowercase();
    for g in &mut out {
        match sort.key {
            SortKey::Name => g.sessions.sort_by(|a, b| {
                lower(&a.title)
                    .cmp(&lower(&b.title))
                    .then(b.mtime_ms.cmp(&a.mtime_ms))
            }),
            SortKey::Date | SortKey::Count => {
                g.sessions.sort_by_key(|s| std::cmp::Reverse(s.mtime_ms))
            }
        }
        if sort.reversed {
            g.sessions.reverse();
        }
    }
    match sort.key {
        SortKey::Date => out.sort_by_key(|g| std::cmp::Reverse(g.last_ms)),
        SortKey::Name => out.sort_by(|a, b| {
            lower(&a.folder)
                .cmp(&lower(&b.folder))
                .then(a.account.cmp(&b.account))
        }),
        SortKey::Count => out.sort_by(|a, b| {
            b.sessions
                .len()
                .cmp(&a.sessions.len())
                .then(b.last_ms.cmp(&a.last_ms))
        }),
    }
    if sort.reversed {
        out.reverse();
    }
    out
}

/// Single-quote `s` for a shell when it needs it.
pub(crate) fn shell_quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-+:@%=,".contains(&b));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::library::tests::session;
    use std::path::PathBuf;

    fn ids(gs: &[Group]) -> Vec<Vec<String>> {
        gs.iter()
            .map(|g| g.sessions.iter().map(|s| s.title.clone()).collect())
            .collect()
    }

    fn fixture() -> Vec<LibSession> {
        let mut v = vec![
            session("b-old", "/r/b", 10),
            session("a-new", "/r/b", 50),
            session("z", "/r/a", 40),
            session("moved", "/r/old", 30),
            session("orphan", "/r/gone", 20),
            session("empty", "/r/a", 5),
        ];
        v[3].status = LibStatus::Moved(PathBuf::from("/r/new"));
        v[4].status = LibStatus::Orphan { memory: false };
        v[5].empty = true;
        v[5].locked = true;
        v
    }

    #[test]
    fn date_groups_newest_first() {
        let gs = groups(&fixture(), Filter::All, "", Sort::default());
        let folders: Vec<_> = gs.iter().map(|g| g.folder.as_str()).collect();
        assert_eq!(folders, ["/r/b", "/r/a", "/r/new", "/r/gone"]);
        assert_eq!(ids(&gs)[0], ["a-new", "b-old"]);
        // Reversed: oldest first, at both levels.
        let rev = Sort::default().choose(SortKey::Date);
        assert!(rev.reversed);
        let gs = groups(&fixture(), Filter::All, "", rev);
        assert_eq!(gs[0].folder, "/r/gone");
        assert_eq!(ids(&gs).last().unwrap(), &["b-old", "a-new"]);
    }

    #[test]
    fn name_and_count() {
        let s = Sort::default().choose(SortKey::Name);
        assert!(!s.reversed);
        let gs = groups(&fixture(), Filter::All, "", s);
        let folders: Vec<_> = gs.iter().map(|g| g.folder.as_str()).collect();
        assert_eq!(folders, ["/r/a", "/r/b", "/r/gone", "/r/new"]);
        assert_eq!(ids(&gs)[1], ["a-new", "b-old"]);
        let gs = groups(
            &fixture(),
            Filter::All,
            "",
            Sort::default().choose(SortKey::Count),
        );
        // Two each for /r/a and /r/b: the more recent first.
        let folders: Vec<_> = gs.iter().map(|g| g.folder.as_str()).collect();
        assert_eq!(folders[..2], ["/r/b", "/r/a"]);
    }

    #[test]
    fn filters_counts_and_search() {
        let v = fixture();
        let c = counts(&v);
        assert_eq!(
            (c.all, c.moved, c.broken, c.empty, c.deletable_empty),
            (6, 1, 1, 1, 0)
        );
        let gs = groups(&v, Filter::Moved, "", Sort::default());
        assert_eq!(ids(&gs), [["moved"]]);
        let gs = groups(&v, Filter::Broken, "", Sort::default());
        assert_eq!(ids(&gs), [["orphan"]]);
        let gs = groups(&v, Filter::All, "R/B NEW", Sort::default());
        assert_eq!(ids(&gs), [["a-new"]]);
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("/home/u/app"), "/home/u/app");
        assert_eq!(shell_quote("/a b/it's"), r"'/a b/it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
