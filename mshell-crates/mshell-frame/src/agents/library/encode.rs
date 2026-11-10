//! Claude Code's `projects/` folder names, and finding the folder a name
//! stands for after the project was renamed or moved.
//!
//! Claude Code names a session's project folder after its working
//! directory with every UTF-16 code unit that is not an ASCII letter or
//! digit replaced by `-` (2.1.322: `cwd.replace(/[^a-zA-Z0-9]/g, "-")`), so
//! an astral character becomes two dashes. Names over 200 characters are cut
//! to 200 and get `-<hash>` appended; the hash is not reproduced here, so
//! such names are compared by their first 200 characters only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Longest folder name Claude Code writes without a hash suffix.
pub(crate) const MAX_NAME: usize = 200;

/// Claude Code's project folder name for `cwd`.
pub(crate) fn encode_cwd(cwd: &str) -> String {
    let mut out = String::with_capacity(cwd.len());
    for c in cwd.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            for _ in 0..c.len_utf16() {
                out.push('-');
            }
        }
    }
    out
}

/// Whether `project_dir` is the folder name Claude Code gives `cwd`.
pub(crate) fn names_match(cwd: &str, project_dir: &str) -> bool {
    let enc = encode_cwd(cwd);
    if enc.len() <= MAX_NAME {
        return enc == project_dir;
    }
    project_dir.len() > MAX_NAME + 1
        && project_dir.as_bytes().get(MAX_NAME) == Some(&b'-')
        && project_dir.get(..MAX_NAME) == enc.get(..MAX_NAME)
}

/// What [`Walker::resolve`] found for a project folder name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Found {
    /// This folder encodes to the name.
    At(PathBuf),
    /// No folder does.
    Nothing,
    /// The search could not finish (budget, deadline, an unreadable
    /// directory, or a name that cannot be searched): we cannot tell.
    Incomplete,
}

/// Directory entries one walk may look at.
pub(crate) const WALK_BUDGET: usize = 20_000;

/// Finds the folder whose encoded path is a project folder name, by
/// walking down from `/` into only the directories whose encoded name,
/// followed by `-`, starts the rest of the name — so the walk stays small.
/// Listings are cached for the whole scan (a shared parent is read once).
/// The remainder shrinks at every level, so a symlink loop cannot recur
/// forever.
pub(crate) struct Walker<L: FnMut(&Path) -> Option<Vec<String>>> {
    list: L,
    cache: HashMap<PathBuf, Option<Vec<String>>>,
    deadline: Option<Instant>,
    left: usize,
}

impl<L: FnMut(&Path) -> Option<Vec<String>>> Walker<L> {
    /// `list(dir)` gives the names of `dir`'s subdirectories (following
    /// symlinks), `None` when it can't be read.
    pub(crate) fn new(list: L, deadline: Option<Instant>) -> Self {
        Walker {
            list,
            cache: HashMap::new(),
            deadline,
            left: 0,
        }
    }

    /// The folder `project_dir` (a `projects/` folder name) stands for.
    pub(crate) fn resolve(&mut self, project_dir: &str) -> Found {
        if project_dir.len() <= 1 || project_dir.len() > MAX_NAME {
            return Found::Incomplete;
        }
        let Some(rest) = project_dir.strip_prefix('-') else {
            return Found::Nothing;
        };
        if self.deadline.is_some_and(|d| Instant::now() > d) {
            return Found::Incomplete;
        }
        self.left = WALK_BUDGET;
        self.walk(Path::new("/"), rest)
    }

    fn listing(&mut self, dir: &Path) -> Option<Vec<String>> {
        if let Some(hit) = self.cache.get(dir) {
            return hit.clone();
        }
        let got = (self.list)(dir);
        if let Some(names) = &got {
            if names.len() >= self.left {
                // Not cached: a later walk with a fresh budget may read it.
                return None;
            }
            self.left -= names.len();
        }
        self.cache.insert(dir.to_path_buf(), got.clone());
        got
    }

    fn walk(&mut self, dir: &Path, rest: &str) -> Found {
        let Some(names) = self.listing(dir) else {
            return Found::Incomplete;
        };
        let mut result = Found::Nothing;
        for name in names {
            let enc = encode_cwd(&name);
            if enc == rest {
                return Found::At(dir.join(&name));
            }
            if let Some(tail) = rest
                .strip_prefix(enc.as_str())
                .and_then(|t| t.strip_prefix('-'))
            {
                match self.walk(&dir.join(&name), tail) {
                    Found::At(p) => return Found::At(p),
                    Found::Incomplete => result = Found::Incomplete,
                    Found::Nothing => {}
                }
            }
        }
        result
    }
}

/// The subdirectory names of `dir` (symlinks to directories included),
/// `None` when it can't be read. Names that aren't UTF-8 are left out:
/// Claude Code could not have recorded them either.
pub(crate) fn list_subdirs(dir: &Path) -> Option<Vec<String>> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut out = Vec::new();
    for e in entries {
        let e = e.ok()?;
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir()) {
            out.push(name);
        }
    }
    out.sort();
    Some(out)
}

/// Display-only guess at the folder of a project name, for a session
/// that never recorded its cwd (`-` is ambiguous: it may be `/`).
pub(crate) fn decode_guess(project_dir: &str) -> Option<String> {
    let rest = project_dir.strip_prefix('-')?;
    Some(format!("/{}", rest.replace('-', "/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_claude_code() {
        // Real pairs from ~/.claude/projects on this machine.
        assert_eq!(encode_cwd("/home/kenan/.cachy"), "-home-kenan--cachy");
        assert_eq!(
            encode_cwd("/repo/archive/.eng/android"),
            "-repo-archive--eng-android"
        );
        assert_eq!(
            encode_cwd("/others/tor/dizi/11.22.63"),
            "-others-tor-dizi-11-22-63"
        );
        // One dash per UTF-16 unit: BMP letters one, astral two.
        assert_eq!(encode_cwd("/a/ş"), "-a--");
        assert_eq!(encode_cwd("/a/😀"), "-a---");
    }

    #[test]
    fn long_names_match_on_their_first_200_characters() {
        let cwd = format!("/{}", "x".repeat(250));
        let enc = encode_cwd(&cwd);
        let stored = format!("{}-1abc", &enc[..MAX_NAME]);
        assert!(names_match(&cwd, &stored));
        assert!(!names_match(&cwd, &enc[..MAX_NAME]));
        let other = format!("/{}", "y".repeat(250));
        assert!(!names_match(&other, &stored));
        assert!(names_match("/a/b", "-a-b"));
        assert!(!names_match("/a/b", "-a-c"));
    }

    fn fake(tree: &[&str]) -> impl FnMut(&Path) -> Option<Vec<String>> {
        let dirs: Vec<PathBuf> = tree.iter().map(PathBuf::from).collect();
        move |dir: &Path| {
            if dir.to_str() == Some("/unreadable") {
                return None;
            }
            let mut v: Vec<String> = dirs
                .iter()
                .filter(|d| d.parent() == Some(dir))
                .filter_map(|d| d.file_name()?.to_str().map(String::from))
                .collect();
            v.sort();
            v.dedup();
            Some(v)
        }
    }

    #[test]
    fn finds_a_renamed_project_folder() {
        let tree = [
            "/home",
            "/home/kenan",
            "/home/kenan/.eng",
            "/home/kenan/.eng/android",
            "/home/kenan/eng",
            "/repo",
        ];
        let mut w = Walker::new(fake(&tree), None);
        assert_eq!(
            w.resolve("-home-kenan--eng-android"),
            Found::At(PathBuf::from("/home/kenan/.eng/android"))
        );
        // `a-b` and `a/b` both encode to `a-b`: the walk tries both.
        let tree = ["/w", "/w/a-b", "/w/a", "/w/a/c"];
        let mut w = Walker::new(fake(&tree), None);
        assert_eq!(w.resolve("-w-a-c"), Found::At(PathBuf::from("/w/a/c")));
        assert_eq!(w.resolve("-w-a-b"), Found::At(PathBuf::from("/w/a-b")));
        assert_eq!(w.resolve("-w-zz"), Found::Nothing);
    }

    #[test]
    fn an_unfinished_walk_is_unknown_not_missing() {
        let tree = ["/unreadable", "/x"];
        let mut w = Walker::new(fake(&tree), None);
        assert_eq!(w.resolve("-unreadable-p"), Found::Incomplete);
        assert_eq!(w.resolve("-"), Found::Incomplete, "/ itself");
        let long = format!("-{}", "a".repeat(MAX_NAME));
        assert_eq!(w.resolve(&long), Found::Incomplete);
        assert_eq!(w.resolve("not-a-path"), Found::Nothing);
        let past = Instant::now() - std::time::Duration::from_secs(1);
        let mut w = Walker::new(fake(&tree), Some(past));
        assert_eq!(w.resolve("-x"), Found::Incomplete, "deadline");
        // A listing larger than the budget stops the walk.
        let mut w = Walker::new(
            |_: &Path| Some((0..WALK_BUDGET).map(|i| i.to_string()).collect()),
            None,
        );
        assert_eq!(w.resolve("-x"), Found::Incomplete);
    }

    #[test]
    fn lists_real_subdirectories_and_symlinked_ones() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("real")).unwrap();
        std::fs::write(d.path().join("file"), "").unwrap();
        std::os::unix::fs::symlink(d.path().join("real"), d.path().join("link")).unwrap();
        assert_eq!(list_subdirs(d.path()).unwrap(), ["link", "real"]);
        assert_eq!(list_subdirs(&d.path().join("gone")), None);
        assert_eq!(decode_guess("-a-b").as_deref(), Some("/a/b"));
        assert_eq!(decode_guess("x"), None);
    }
}
