//! `.pacnew` / `.pacsave` / `.pacorig` review.
//!
//! pacman writes `<file>.pacnew` when a package ships a new default
//! for a config file the user edited, warns once and never mentions it
//! again. The panel lists them, shows the diff against the live file —
//! a 3-way `diff3 -m` merge when the previous default (the *base*) can
//! be pulled out of a cached package with `bsdtar` — and offers merge,
//! replace and delete. Writes go through one `pkexec` call that keeps
//! the live file's mode and owner and leaves a `.mshell-bak` copy.

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

/// The argv (after `pkexec`) for an action. Paths are passed as
/// positional args to a fixed `sh -c` script, never spliced into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    /// Replace the live file with `src` (the .pacnew / .pacsave itself
    /// or a merged temp file), keeping the live file's mode/owner,
    /// backing it up as `<live>.mshell-bak`, then remove the review
    /// file. With `src == review` this is "take new" / "restore".
    Install {
        src: PathBuf,
        live: PathBuf,
        review: PathBuf,
    },
    /// Delete the review file (keep the live config).
    Remove { review: PathBuf },
}

const INSTALL_SH: &str = r#"set -e
src="$1"; live="$2"; review="$3"
if [ -e "$live" ]; then
  cp -a -- "$live" "$live.mshell-bak"
  mode=$(stat -c %a -- "$live"); own=$(stat -c %u:%g -- "$live")
else
  mode=644; own=0:0
fi
install -m "$mode" -o "${own%%:*}" -g "${own##*:}" -T -- "$src" "$live"
rm -f -- "$review""#;

pub(crate) fn op_argv(op: &Op) -> Vec<String> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    match op {
        Op::Install { src, live, review } => vec![
            "sh".into(),
            "-c".into(),
            INSTALL_SH.into(),
            "sh".into(),
            s(src),
            s(live),
            s(review),
        ],
        Op::Remove { review } => vec!["rm".into(), "-f".into(), "--".into(), s(review)],
    }
}

/// Every review path must stay under the search root and be one of
/// ours — the panel never asks root to touch anything else.
pub(crate) fn op_is_safe(op: &Op) -> bool {
    let under =
        |p: &Path| p.starts_with(SEARCH_ROOT) && !p.components().any(|c| c.as_os_str() == "..");
    match op {
        Op::Install { live, review, .. } => {
            under(live) && under(review) && classify(review).is_some_and(|c| c.live == *live)
        }
        Op::Remove { review } => under(review) && classify(review).is_some(),
    }
}

/// Run an op through pkexec (one password prompt).
pub(crate) async fn apply(op: Op) -> Result<(), String> {
    if !op_is_safe(&op) {
        return Err("refusing a path outside /etc".into());
    }
    let status = tokio::process::Command::new("pkexec")
        .args(op_argv(&op))
        .status()
        .await
        .map_err(|e| format!("pkexec: {e}"))?;
    if !status.success() {
        return Err(match status.code() {
            Some(126) | Some(127) => "authentication dismissed".into(),
            Some(c) => format!("exit {c}"),
            None => "killed".into(),
        });
    }
    Ok(())
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
}

/// Build the review (blocking: runs pacman/bsdtar/diff; call from a
/// worker thread).
pub(crate) fn review_blocking(file: &ConfigFile) -> Review {
    use std::process::Command;
    let mut r = Review::default();
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
    let dir = std::env::temp_dir().join(format!("mshell-pacnew-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let base_path = dir.join("base");
    if std::fs::write(&base_path, &candidates[i]).is_err() {
        return r;
    }
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

/// Write a merged result to a user-owned temp file for `Op::Install`.
pub(crate) fn stage_merged(text: &str) -> Result<PathBuf, String> {
    let dir = margo_fs::xdg::cache_home()
        .join("mshell")
        .join("system_update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join("merged.conf");
    std::fs::write(&p, text).map_err(|e| e.to_string())?;
    Ok(p)
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
            src: "/home/u/.cache/merged".into(),
            live: "/etc/locale.gen".into(),
            review: "/etc/locale.gen.pacnew".into(),
        };
        assert!(op_is_safe(&ok));
        let argv = op_argv(&ok);
        assert_eq!(&argv[..2], ["sh", "-c"]);
        assert_eq!(
            &argv[4..],
            [
                "/home/u/.cache/merged",
                "/etc/locale.gen",
                "/etc/locale.gen.pacnew"
            ]
        );

        assert!(!op_is_safe(&Op::Remove {
            review: "/etc/../root/x.pacnew".into()
        }));
        assert!(!op_is_safe(&Op::Remove {
            review: "/etc/passwd".into()
        }));
        assert!(!op_is_safe(&Op::Install {
            src: "/tmp/x".into(),
            live: "/etc/shadow".into(),
            review: "/etc/locale.gen.pacnew".into(),
        }));
        assert_eq!(
            op_argv(&Op::Remove {
                review: "/etc/a.pacnew".into()
            }),
            ["rm", "-f", "--", "/etc/a.pacnew"]
        );
    }

    #[test]
    fn install_script_keeps_mode_and_backs_up() {
        // Exercise the real script without root on a temp tree (owner =
        // us, so `install -o/-g` with our own ids succeeds).
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("f.conf");
        let review = dir.path().join("f.conf.pacnew");
        std::fs::write(&live, "mine\n").unwrap();
        std::fs::write(&review, "theirs\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&live, std::fs::Permissions::from_mode(0o600)).unwrap();
        let st = std::process::Command::new("sh")
            .args(["-c", INSTALL_SH, "sh"])
            .arg(&review)
            .arg(&live)
            .arg(&review)
            .status()
            .unwrap();
        assert!(st.success());
        assert_eq!(std::fs::read_to_string(&live).unwrap(), "theirs\n");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.conf.mshell-bak")).unwrap(),
            "mine\n"
        );
        let mode = std::fs::metadata(&live).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!review.exists(), "the .pacnew is consumed");
    }
}
