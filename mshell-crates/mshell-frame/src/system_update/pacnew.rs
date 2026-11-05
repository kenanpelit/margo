//! `.pacnew` / `.pacsave` / `.pacorig` review.
//!
//! pacman writes `<file>.pacnew` when a package ships a new default
//! for a config file the user edited, warns once and never mentions it
//! again. The panel lists them, shows the diff against the live file —
//! a 3-way `diff3 -m` merge when the previous default (the *base*) can
//! be pulled out of a cached package with `bsdtar` — and offers merge,
//! replace and delete. Writes go through one `pkexec mshell-privileged
//! pacnew-*` call that keeps the live file's mode and owner and leaves
//! a `.mshell-bak` copy.

use std::path::{Path, PathBuf};

pub(crate) const SEARCH_ROOT: &str = "/etc";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// New default from an upgrade (`<file>.pacnew`).
    Pacnew,
    /// Config kept when its package was removed (`<file>.pacsave`).
    Pacsave,
    /// Pre-existing file pacman moved aside on install (`.pacorig`).
    Pacorig,
}

impl Kind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Pacnew => "new default",
            Kind::Pacsave => "saved on removal",
            Kind::Pacorig => "original before install",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigFile {
    pub(crate) kind: Kind,
    /// The `.pacnew` / `.pacsave` itself.
    pub(crate) path: PathBuf,
    /// The live file it belongs to.
    pub(crate) live: PathBuf,
}

/// Classify a path found under /etc.
pub(crate) fn classify(path: &Path) -> Option<ConfigFile> {
    let s = path.to_str()?;
    let (kind, live) = if let Some(l) = s.strip_suffix(".pacnew") {
        (Kind::Pacnew, l)
    } else if let Some(l) = s.strip_suffix(".pacsave") {
        (Kind::Pacsave, l)
    } else if let Some(l) = s.strip_suffix(".pacorig") {
        (Kind::Pacorig, l)
    } else {
        return None;
    };
    if live.is_empty() || live.ends_with('/') {
        return None;
    }
    Some(ConfigFile {
        kind,
        path: path.to_path_buf(),
        live: PathBuf::from(live),
    })
}

/// Recursive scan (no symlink following; unreadable dirs skipped).
pub(crate) fn scan(root: &Path) -> Vec<ConfigFile> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file()
                && let Some(c) = classify(&p)
            {
                out.push(c);
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

// ── diff3 -m ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Segment {
    Common(Vec<String>),
    Conflict {
        mine: Vec<String>,
        base: Vec<String>,
        theirs: Vec<String>,
    },
}

/// Parse `diff3 -m -L mine -L base -L theirs` output. Everything one
/// side changed alone is already merged into the common text; only
/// regions both sides touched become conflicts.
pub(crate) fn parse_diff3(text: &str) -> Vec<Segment> {
    enum St {
        Common,
        Mine,
        Base,
        Theirs,
    }
    let mut out = Vec::new();
    let mut common: Vec<String> = Vec::new();
    let (mut mine, mut base, mut theirs) = (Vec::new(), Vec::new(), Vec::new());
    let mut st = St::Common;
    for line in text.lines() {
        match st {
            St::Common if line.starts_with("<<<<<<< ") || line == "<<<<<<<" => {
                if !common.is_empty() {
                    out.push(Segment::Common(std::mem::take(&mut common)));
                }
                st = St::Mine;
            }
            St::Common => common.push(line.to_string()),
            St::Mine if line.starts_with("||||||| ") || line == "|||||||" => st = St::Base,
            St::Mine | St::Base if line == "=======" => st = St::Theirs,
            St::Mine => mine.push(line.to_string()),
            St::Base => base.push(line.to_string()),
            St::Theirs if line.starts_with(">>>>>>> ") || line == ">>>>>>>" => {
                out.push(Segment::Conflict {
                    mine: std::mem::take(&mut mine),
                    base: std::mem::take(&mut base),
                    theirs: std::mem::take(&mut theirs),
                });
                st = St::Common;
            }
            St::Theirs => theirs.push(line.to_string()),
        }
    }
    if !common.is_empty() {
        out.push(Segment::Common(common));
    }
    out
}

pub(crate) fn conflicts(segments: &[Segment]) -> usize {
    segments
        .iter()
        .filter(|s| matches!(s, Segment::Conflict { .. }))
        .count()
}

/// Which side wins every conflict when merging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Prefer {
    Mine,
    Theirs,
}

/// The merged file text.
pub(crate) fn resolve(segments: &[Segment], prefer: Prefer) -> String {
    let mut out = String::new();
    for s in segments {
        let lines = match s {
            Segment::Common(l) => l,
            Segment::Conflict { mine, theirs, .. } => match prefer {
                Prefer::Mine => mine,
                Prefer::Theirs => theirs,
            },
        };
        for l in lines {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

// ── unified diff (display) ───────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffLine {
    Context,
    Removed,
    Added,
    Hunk,
}

/// Classify `diff -u` lines for colouring (file headers dropped).
pub(crate) fn parse_unified(text: &str) -> Vec<(DiffLine, String)> {
    text.lines()
        .filter(|l| !l.starts_with("--- ") && !l.starts_with("+++ "))
        .map(|l| {
            let kind = match l.chars().next() {
                Some('@') => DiffLine::Hunk,
                Some('-') => DiffLine::Removed,
                Some('+') => DiffLine::Added,
                _ => DiffLine::Context,
            };
            (kind, l.to_string())
        })
        .collect()
}

// ── base lookup ──────────────────────────────────────────────────

/// `etc/foo.conf` — the member name inside a package archive.
pub(crate) fn archive_member(live: &Path) -> Option<String> {
    live.to_str()?.strip_prefix('/').map(str::to_string)
}

/// Pick the base: among older cached copies of the file (newest first)
/// the first that differs from the `.pacnew` — that is the default the
/// user's file was last derived from. `None` when every cached copy
/// equals the new default (the cache was cleaned past the change).
pub(crate) fn pick_base(candidates: &[Vec<u8>], theirs: &[u8]) -> Option<usize> {
    candidates.iter().position(|c| c.as_slice() != theirs)
}

// ── privileged actions ───────────────────────────────────────────

/// A privileged pacnew action. Each runs as one fixed verb of the
/// root helper (`mshell-privileged`), which re-validates the paths and
/// refuses symlinks itself; [`op_is_safe`] is the same check up front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    /// Replace the live file with new content, keeping the live
    /// file's mode/owner, backing it up as `<live>.mshell-bak`, then
    /// remove the review file.
    Install {
        from: InstallFrom,
        live: PathBuf,
        review: PathBuf,
    },
    /// Delete the review file (keep the live config).
    Remove { review: PathBuf },
}

/// Where [`Op::Install`]'s new content comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InstallFrom {
    /// The review file itself: "take new" / "restore".
    Review,
    /// A merge resolved in the panel. Piped to the helper on stdin
    /// (`pacnew-merge`), never staged in a user-writable file a
    /// same-uid process could swap while the password prompt is up.
    Merged(String),
}

/// The helper request for an op (`mshell-privileged pacnew-*`).
pub(crate) fn op_request(op: &Op) -> mshell_privileged::Request {
    use mshell_privileged::Request;
    match op {
        Op::Install {
            from: InstallFrom::Review,
            live,
            review,
        } => Request::PacnewInstall {
            live: live.clone(),
            review: review.clone(),
        },
        Op::Install {
            from: InstallFrom::Merged(_),
            live,
            review,
        } => Request::PacnewMerge {
            live: live.clone(),
            review: review.clone(),
        },
        Op::Remove { review } => Request::PacnewRemove {
            review: review.clone(),
        },
    }
}

/// The argv after `pkexec`: the fixed helper, its verb, the paths.
pub(crate) fn op_argv(op: &Op) -> Vec<std::ffi::OsString> {
    op_request(op).argv()
}

/// What the op feeds the helper on stdin (the merged text).
pub(crate) fn op_stdin(op: &Op) -> Option<&[u8]> {
    match op {
        Op::Install {
            from: InstallFrom::Merged(text),
            ..
        } => Some(text.as_bytes()),
        _ => None,
    }
}

/// Every review path must stay under the search root and be one of
/// ours — the panel never asks root to touch anything else — and
/// neither the live file nor a directory on its way may be a symlink
/// (root would write through it; the script refuses too).
pub(crate) fn op_is_safe(op: &Op) -> bool {
    let root = Path::new(SEARCH_ROOT);
    let under = |p: &Path| p.starts_with(root) && !p.components().any(|c| c.as_os_str() == "..");
    let shape = match op {
        Op::Install { live, review, .. } => {
            under(live)
                && under(review)
                && classify(review).is_some_and(|c| c.live == *live)
                && no_symlink_below(root, live)
                && no_symlink_below(root, review)
        }
        Op::Remove { review } => under(review) && classify(review).is_some(),
    };
    // What the helper would refuse anyway (`.`/`//` components, …).
    shape && op_request(op).validated().is_ok()
}

/// No component of `p` below `root` is a symlink. Components that
/// don't exist yet are fine (restoring a .pacsave whose live file is
/// gone); any other error to stat one is treated as unsafe.
pub(crate) fn no_symlink_below(root: &Path, p: &Path) -> bool {
    let Ok(rest) = p.strip_prefix(root) else {
        return false;
    };
    let mut cur = root.to_path_buf();
    for c in rest.components() {
        cur.push(c);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return false,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        }
    }
    true
}

/// Run an op through pkexec (one password prompt).
pub(crate) async fn apply(op: Op) -> Result<(), String> {
    if !op_is_safe(&op) {
        return Err("refusing a path outside /etc or through a symlink".into());
    }
    if !mshell_privileged::installed() {
        return Err(mshell_privileged::MISSING.into());
    }
    let input = op_stdin(&op);
    let mut child = tokio::process::Command::new("pkexec")
        .args(op_argv(&op))
        .stdin(if input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("pkexec: {e}"))?;
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        use tokio::io::AsyncWriteExt;
        // A dismissed prompt closes the pipe early; the exit status
        // below says why, so a write error here is not reported.
        let _ = stdin.write_all(bytes).await;
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| format!("pkexec: {e}"))?;
    apply_outcome(out.status.code(), &String::from_utf8_lossy(&out.stderr))
}

/// The panel's message for a finished helper run: pkexec's 126/127
/// are the dialog, anything else is the helper's own last stderr line
/// (e.g. "refusing /etc/resolv.conf: a symlink").
pub(crate) fn apply_outcome(code: Option<i32>, stderr: &str) -> Result<(), String> {
    match code {
        Some(0) => Ok(()),
        Some(126) | Some(127) => Err("authentication dismissed".into()),
        Some(c) => Err(stderr
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map_or_else(|| format!("exit {c}"), str::to_string)),
        None => Err("killed".into()),
    }
}

/// Everything the detail view needs for one file.
#[derive(Debug, Clone, Default)]
pub(crate) struct Review {
    /// `diff -u live review` lines.
    pub(crate) unified: Vec<(DiffLine, String)>,
    /// 3-way merge when a base was found.
    pub(crate) merge: Option<Vec<Segment>>,
    /// Package version the base came from.
    pub(crate) base_from: Option<String>,
    /// Live and review file are byte-identical.
    pub(crate) identical: bool,
    /// Couldn't read one side (root-only file).
    pub(crate) unreadable: bool,
    /// The live file (or a directory on its way) is a symlink: the
    /// panel won't write through it, only Delete is offered.
    pub(crate) symlinked: bool,
}

impl Review {
    /// Merge / replace may be offered: both sides were readable and
    /// the live file isn't reached through a symlink.
    pub(crate) fn writable(&self) -> bool {
        !self.unreadable && !self.symlinked
    }
}

/// Build the review (blocking: runs pacman/bsdtar/diff; call from a
/// worker thread).
pub(crate) fn review_blocking(file: &ConfigFile) -> Review {
    use std::process::Command;
    let mut r = Review {
        symlinked: !no_symlink_below(Path::new(SEARCH_ROOT), &file.live),
        ..Review::default()
    };
    let (Ok(mine), Ok(theirs)) = (std::fs::read(&file.live), std::fs::read(&file.path)) else {
        r.unreadable = true;
        return r;
    };
    r.identical = mine == theirs;
    if let Ok(o) = Command::new("diff")
        .arg("-u")
        .arg(&file.live)
        .arg(&file.path)
        .output()
    {
        r.unified = parse_unified(&String::from_utf8_lossy(&o.stdout));
    }
    if file.kind != Kind::Pacnew || r.identical {
        return r;
    }
    // Base: owner package → its cached archives, newest first.
    let owner = Command::new("pacman")
        .arg("-Qoq")
        .arg(&file.live)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty() && !s.contains('\n'));
    let (Some(owner), Some(member)) = (owner, archive_member(&file.live)) else {
        return r;
    };
    let mut files: Vec<(String, i64)> = super::history::cache_listing()
        .into_iter()
        .filter(|(f, _)| super::history::cache_file_version(&owner, f).is_some())
        .collect();
    files.sort_by_key(|(_, m)| std::cmp::Reverse(*m));
    let mut candidates = Vec::new();
    let mut versions = Vec::new();
    for (f, _) in files.iter().take(6) {
        let path = Path::new(super::size::PKG_CACHE).join(f);
        if let Ok(o) = Command::new("bsdtar")
            .arg("-xOf")
            .arg(&path)
            .arg(&member)
            .output()
            && o.status.success()
        {
            candidates.push(o.stdout);
            versions.push(super::history::cache_file_version(&owner, f).unwrap_or_default());
        }
    }
    let Some(i) = pick_base(&candidates, &theirs) else {
        return r;
    };
    let Ok(base_path) = stage_base(&margo_fs::xdg::runtime_dir_or_tmp(), &candidates[i]) else {
        return r;
    };
    if let Ok(o) = Command::new("diff3")
        .args(["-m", "-L", "mine", "-L", "base", "-L", "theirs"])
        .arg(&file.live)
        .arg(&base_path)
        .arg(&file.path)
        .output()
        // 0 = clean merge, 1 = conflicts; 2 = trouble.
        && o.status.code().is_some_and(|c| c <= 1)
    {
        r.merge = Some(parse_diff3(&String::from_utf8_lossy(&o.stdout)));
        r.base_from = Some(format!("{owner} {}", versions[i]));
    }
    let _ = std::fs::remove_file(&base_path);
    r
}

/// Write the base copy for diff3 into `dir` (the user's 0700 runtime
/// dir in practice): a fresh name opened `O_CREAT|O_EXCL` with mode
/// 0600, so a pre-planted symlink or FIFO is never followed or opened.
/// The caller removes it.
pub(crate) fn stage_base(dir: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let mut last = None;
    for _ in 0..8 {
        let p = dir.join(format!(
            "mshell-pacnew-base-{}-{nanos:x}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&p)
        {
            Ok(mut f) => {
                if let Err(e) = f.write_all(bytes) {
                    let _ = std::fs::remove_file(&p);
                    return Err(e);
                }
                return Ok(p);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no free name")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF3: &str = include_str!("testdata/diff3-mkinitcpio.txt");
    const UNIFIED: &str = include_str!("testdata/diff-mkinitcpio.txt");

    #[test]
    fn classifies_review_files() {
        let c = classify(Path::new("/etc/pacman.d/mirrorlist.pacnew")).unwrap();
        assert_eq!(c.kind, Kind::Pacnew);
        assert_eq!(c.live, Path::new("/etc/pacman.d/mirrorlist"));
        assert_eq!(
            classify(Path::new("/etc/x.pacsave")).unwrap().kind,
            Kind::Pacsave
        );
        assert_eq!(
            classify(Path::new("/etc/x.pacorig")).unwrap().kind,
            Kind::Pacorig
        );
        assert!(classify(Path::new("/etc/x.conf")).is_none());
        assert!(classify(Path::new("/etc/.pacnew")).is_none());
    }

    #[test]
    fn scans_a_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("pam.d")).unwrap();
        std::fs::write(dir.path().join("locale.gen"), "a").unwrap();
        std::fs::write(dir.path().join("locale.gen.pacnew"), "b").unwrap();
        std::fs::write(dir.path().join("pam.d/mlogind.pacnew"), "c").unwrap();
        std::fs::write(dir.path().join("pam.d/other"), "d").unwrap();
        let found = scan(dir.path());
        assert_eq!(found.len(), 2);
        assert!(found[0].path.ends_with("locale.gen.pacnew"));
    }

    #[test]
    fn parses_real_diff3_merge() {
        // Real `diff3 -m` of this machine's /etc/mkinitcpio.conf against
        // mkinitcpio.conf.pacnew with an edited base: the HOOKS line was
        // changed on both sides → one conflict, the rest merges.
        let segs = parse_diff3(DIFF3);
        assert_eq!(conflicts(&segs), 1);
        let Some(Segment::Conflict { mine, base, theirs }) =
            segs.iter().find(|s| matches!(s, Segment::Conflict { .. }))
        else {
            panic!("no conflict")
        };
        assert_eq!(
            mine,
            &[
                "HOOKS=(base systemd autodetect microcode kms modconf block keyboard sd-vconsole plymouth filesystems)"
            ]
        );
        assert_eq!(base.len(), 1);
        assert!(theirs[0].ends_with("block filesystems fsck)"));

        let merged = resolve(&segs, Prefer::Mine);
        assert!(merged.contains("sd-vconsole plymouth filesystems)\n"));
        assert!(!merged.contains("<<<<<<<"));
        assert_eq!(merged.lines().count(), DIFF3.lines().count() - 6);
        let theirs_merge = resolve(&segs, Prefer::Theirs);
        assert!(!theirs_merge.contains("plymouth"));
    }

    #[test]
    fn diff3_without_conflicts_is_all_common() {
        let segs = parse_diff3("a\nb\n");
        assert_eq!(segs, vec![Segment::Common(vec!["a".into(), "b".into()])]);
        assert_eq!(resolve(&segs, Prefer::Theirs), "a\nb\n");
    }

    #[test]
    fn parses_real_unified_diff() {
        let lines = parse_unified(UNIFIED);
        assert_eq!(lines[0].0, DiffLine::Hunk);
        assert_eq!(
            lines
                .iter()
                .filter(|(k, _)| *k == DiffLine::Removed)
                .count(),
            2
        );
        assert_eq!(
            lines.iter().filter(|(k, _)| *k == DiffLine::Added).count(),
            2
        );
        assert!(lines.iter().all(|(_, l)| !l.starts_with("+++")));
    }

    #[test]
    fn base_is_the_newest_copy_that_differs_from_the_new_default() {
        let theirs = b"new".to_vec();
        assert_eq!(
            pick_base(&[theirs.clone(), b"old".to_vec()], &theirs),
            Some(1)
        );
        // This machine's cache: mkinitcpio 42-1 / 42.1-1 both ship the
        // .pacnew's content → no base, fall back to 2-way.
        assert_eq!(pick_base(&[theirs.clone(), theirs.clone()], &theirs), None);
        assert_eq!(
            archive_member(Path::new("/etc/mkinitcpio.conf")).unwrap(),
            "etc/mkinitcpio.conf"
        );
    }

    #[test]
    fn ops_stay_inside_etc() {
        let ok = Op::Install {
            from: InstallFrom::Review,
            live: "/etc/locale.gen".into(),
            review: "/etc/locale.gen.pacnew".into(),
        };
        assert!(op_is_safe(&ok));
        // The fixed helper, never a shell: the polkit dialog names it.
        assert_eq!(
            op_argv(&ok),
            [
                mshell_privileged::HELPER,
                "pacnew-install",
                "/etc/locale.gen",
                "/etc/locale.gen.pacnew"
            ]
        );
        assert_eq!(op_stdin(&ok), None);

        assert!(!op_is_safe(&Op::Remove {
            review: "/etc/../root/x.pacnew".into()
        }));
        assert!(!op_is_safe(&Op::Remove {
            review: "/etc/passwd".into()
        }));
        assert!(!op_is_safe(&Op::Install {
            from: InstallFrom::Review,
            live: "/etc/shadow".into(),
            review: "/etc/locale.gen.pacnew".into(),
        }));
        // /etc/localtime is a symlink on (nearly) every systemd box.
        if std::fs::symlink_metadata("/etc/localtime").is_ok_and(|m| m.file_type().is_symlink()) {
            assert!(!op_is_safe(&Op::Install {
                from: InstallFrom::Review,
                live: "/etc/localtime".into(),
                review: "/etc/localtime.pacnew".into(),
            }));
        }
        assert_eq!(
            op_argv(&Op::Remove {
                review: "/etc/a.pacnew".into()
            }),
            [mshell_privileged::HELPER, "pacnew-remove", "/etc/a.pacnew"]
        );
    }

    #[test]
    fn a_merge_reaches_root_on_stdin_not_through_a_user_file() {
        // The merged text used to be staged in ~/.cache (user-writable)
        // and root installed whatever that file held after the password
        // prompt. It is piped to the helper now (its own tests run the
        // real binary on a fixture tree).
        let op = Op::Install {
            from: InstallFrom::Merged("merged\n".into()),
            live: "/etc/locale.gen".into(),
            review: "/etc/locale.gen.pacnew".into(),
        };
        assert!(op_is_safe(&op));
        assert_eq!(
            op_argv(&op),
            [
                mshell_privileged::HELPER,
                "pacnew-merge",
                "/etc/locale.gen",
                "/etc/locale.gen.pacnew"
            ]
        );
        assert_eq!(op_stdin(&op), Some(&b"merged\n"[..]));
    }

    #[test]
    fn a_refusal_reads_as_the_helpers_reason() {
        assert_eq!(apply_outcome(Some(0), ""), Ok(()));
        assert_eq!(
            apply_outcome(Some(126), "x"),
            Err("authentication dismissed".into())
        );
        assert_eq!(
            apply_outcome(Some(3), "refusing /etc/resolv.conf: a symlink\n"),
            Err("refusing /etc/resolv.conf: a symlink".into())
        );
        assert_eq!(apply_outcome(Some(1), ""), Err("exit 1".into()));
        assert!(apply_outcome(None, "").is_err());
    }

    #[test]
    fn what_the_helper_would_refuse_never_raises_a_prompt() {
        // classify() takes any .pacnew under /etc; the helper also wants
        // plain components (no `.`/`//`), so the panel checks the same.
        assert!(!op_is_safe(&Op::Remove {
            review: "/etc/./a.pacnew".into()
        }));
        assert!(!op_is_safe(&Op::Install {
            from: InstallFrom::Review,
            live: "/etc//a".into(),
            review: "/etc//a.pacnew".into(),
        }));
    }

    #[test]
    fn the_diff3_base_never_follows_a_planted_path() {
        // The base used to go to /tmp/mshell-pacnew-<pid>/base: another
        // user could pre-create that as a symlink to one of our files
        // (overwritten with the package default) or a FIFO (diff3 hangs).
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("authorized_keys");
        std::fs::write(&victim, "ssh-ed25519 KEY\n").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join("base")).unwrap();
        let mut staged = Vec::new();
        for _ in 0..2 {
            let p = stage_base(dir.path(), b"default\n").unwrap();
            let m = std::fs::symlink_metadata(&p).unwrap();
            assert!(m.file_type().is_file());
            assert_eq!(m.permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::read(&p).unwrap(), b"default\n");
            staged.push(p);
        }
        assert_ne!(staged[0], staged[1], "a fresh name every time");
        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "ssh-ed25519 KEY\n"
        );
    }

    #[test]
    fn a_symlinked_live_file_is_delete_only() {
        let r = Review {
            symlinked: true,
            ..Review::default()
        };
        assert!(!r.writable());
        assert!(Review::default().writable());
        assert!(
            !Review {
                unreadable: true,
                ..Review::default()
            }
            .writable()
        );
    }

    #[test]
    fn symlinks_below_the_root_are_unsafe() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("pam.d")).unwrap();
        std::fs::write(root.join("pam.d/system-auth"), "x").unwrap();
        std::fs::write(root.join("real"), "x").unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("resolv.conf")).unwrap();
        std::os::unix::fs::symlink(root.join("pam.d"), root.join("pam-link")).unwrap();

        assert!(no_symlink_below(root, &root.join("pam.d/system-auth")));
        // Not there yet (a .pacsave restore): fine.
        assert!(no_symlink_below(root, &root.join("pam.d/new")));
        assert!(!no_symlink_below(root, &root.join("resolv.conf")));
        assert!(!no_symlink_below(root, &root.join("pam-link/system-auth")));
    }
}
