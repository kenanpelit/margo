//! Whether a saved session can resume as it is (the Sessions page's
//! status glyph). Pure: the scan gathers the facts, this decides.
//!
//! Two passes, like claude-cockpit's `list-claude-sessions`: the first
//! settles what one transcript alone can tell; the rest need the folder
//! its project folder name stands for, searched once per project folder
//! ([`super::encode::Walker`]).

use super::encode::{Found, names_match};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LibStatus {
    /// Resumes from its recorded folder.
    Ok,
    /// The project folder was renamed or moved; it is now here, and the
    /// session resumes from there.
    Moved(PathBuf),
    /// No folder matches the session. `memory`: its project folder still
    /// holds memory files.
    Orphan { memory: bool },
    /// The transcript sits in a project folder no folder matches, but its
    /// recorded working directory exists (repairable).
    Misplaced,
    /// The scan could not check the folder.
    Unknown,
}

impl LibStatus {
    /// Short name for chips and filter buttons.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            LibStatus::Ok => "ok",
            LibStatus::Moved(_) => "moved",
            LibStatus::Orphan { .. } => "orphan",
            LibStatus::Misplaced => "misplaced",
            LibStatus::Unknown => "unknown",
        }
    }

    /// Why the session can't resume as it is (the glyph's tooltip).
    pub(crate) fn reason(&self, cwd: &str) -> String {
        match self {
            LibStatus::Ok => String::new(),
            LibStatus::Moved(to) => format!(
                "Moved: {} was renamed or moved — resumes from {}",
                super::super::sessions::tilde(cwd),
                super::super::sessions::tilde(&to.to_string_lossy())
            ),
            LibStatus::Orphan { memory: false } => {
                "Orphan: no folder matches this session".to_string()
            }
            LibStatus::Orphan { memory: true } => {
                "Orphan: no folder matches this session (its project folder still has memory files)"
                    .to_string()
            }
            LibStatus::Misplaced => format!(
                "Misplaced: the transcript is in a project folder no folder matches, \
                 but {} exists — Repair moves it there",
                super::super::sessions::tilde(cwd)
            ),
            LibStatus::Unknown => "Unknown: the scan could not check the folder".to_string(),
        }
    }

    /// The glyph shown before the title.
    pub(crate) fn icon(&self) -> Option<&'static str> {
        match self {
            LibStatus::Ok => None,
            LibStatus::Moved(_) => Some("folder-symbolic"),
            LibStatus::Orphan { .. } => Some("dialog-warning-symbolic"),
            LibStatus::Misplaced => Some("dialog-warning-symbolic"),
            LibStatus::Unknown => Some("dialog-question-symbolic"),
        }
    }
}

/// What the first pass knows about one transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FirstPass {
    Settled(LibStatus),
    /// Needs the project folder searched; the recorded cwd exists.
    CwdExists,
    /// Needs the project folder searched; the recorded cwd is gone.
    CwdGone,
}

/// The first pass. `cwd` is the session's recorded working directory
/// (`""` when it never recorded one), `project_dir` the `projects/`
/// folder its transcript sits in, `cwd_is_dir` whether the cwd exists,
/// and `same_folder` whether the cwd's own project folder (followed
/// through symlinks) is the folder holding the transcript — a project
/// folder symlinked to another one resumes fine.
pub(crate) fn first_pass(
    cwd: &str,
    project_dir: &str,
    cwd_is_dir: bool,
    same_folder: bool,
) -> FirstPass {
    if cwd.is_empty() {
        return FirstPass::Settled(LibStatus::Ok);
    }
    if cwd_is_dir && (same_folder || names_match(cwd, project_dir)) {
        return FirstPass::Settled(LibStatus::Ok);
    }
    if cwd_is_dir {
        FirstPass::CwdExists
    } else {
        FirstPass::CwdGone
    }
}

/// The final status from the first pass, the search for the project
/// folder name, and whether that project folder holds memory files.
pub(crate) fn settle(first: FirstPass, found: &Found, memory: bool) -> LibStatus {
    match (first, found) {
        (FirstPass::Settled(s), _) => s,
        (_, Found::At(p)) => LibStatus::Moved(p.clone()),
        (_, Found::Incomplete) => LibStatus::Unknown,
        (FirstPass::CwdExists, Found::Nothing) => LibStatus::Misplaced,
        (FirstPass::CwdGone, Found::Nothing) => LibStatus::Orphan { memory },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(
        cwd: &str,
        dir: &str,
        is_dir: bool,
        same: bool,
        found: Found,
        mem: bool,
    ) -> LibStatus {
        settle(first_pass(cwd, dir, is_dir, same), &found, mem)
    }

    #[test]
    fn ok_when_the_folder_matches_or_nothing_was_recorded() {
        let at = Found::At(PathBuf::from("/elsewhere"));
        assert_eq!(
            status("", "-x", false, false, at.clone(), false),
            LibStatus::Ok
        );
        assert_eq!(
            status("/a/b", "-a-b", true, false, at.clone(), false),
            LibStatus::Ok
        );
        // `-repo-archive--eng-android` is a symlink to
        // `-home-kenan--eng-android` on this machine: same folder, resumes.
        assert_eq!(
            status(
                "/repo/archive/.eng/android",
                "-home-kenan--eng-android",
                true,
                true,
                at,
                false
            ),
            LibStatus::Ok
        );
    }

    #[test]
    fn moved_when_another_folder_encodes_to_the_project_name() {
        let to = PathBuf::from("/r/new-name");
        // Old folder gone, renamed one found.
        assert_eq!(
            status(
                "/r/old",
                "-r-new-name",
                false,
                false,
                Found::At(to.clone()),
                true
            ),
            LibStatus::Moved(to.clone())
        );
        // The recorded folder exists too (resumed from elsewhere): still
        // resumes from the folder the transcript belongs to.
        assert_eq!(
            status(
                "/r/old",
                "-r-new-name",
                true,
                false,
                Found::At(to.clone()),
                false
            ),
            LibStatus::Moved(to)
        );
        // A deleted folder whose name still matches is gone, not moved.
        assert_eq!(
            status("/r/gone", "-r-gone", false, false, Found::Nothing, false),
            LibStatus::Orphan { memory: false }
        );
    }

    #[test]
    fn misplaced_orphan_and_unknown() {
        assert_eq!(
            status(
                "/r/here",
                "-old-machine-here",
                true,
                false,
                Found::Nothing,
                false
            ),
            LibStatus::Misplaced
        );
        assert_eq!(
            status(
                "/r/gone",
                "-old-machine-gone",
                false,
                false,
                Found::Nothing,
                true
            ),
            LibStatus::Orphan { memory: true }
        );
        assert_eq!(
            status("/r/gone", "-old", false, false, Found::Incomplete, true),
            LibStatus::Unknown
        );
        assert_eq!(
            status("/r/here", "-old", true, false, Found::Incomplete, false),
            LibStatus::Unknown
        );
    }

    #[test]
    fn reasons_and_glyphs() {
        assert!(LibStatus::Ok.reason("/a").is_empty());
        assert!(LibStatus::Ok.icon().is_none());
        assert!(
            LibStatus::Orphan { memory: true }
                .reason("/a")
                .contains("memory files")
        );
        assert!(LibStatus::Misplaced.reason("/nowhere/x").contains("Repair"));
        assert!(
            LibStatus::Moved(PathBuf::from("/b"))
                .reason("/a")
                .contains("resumes from /b")
        );
    }
}
