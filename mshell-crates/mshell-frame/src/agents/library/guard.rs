//! Path guards for everything the Sessions page changes on disk.
//!
//! Every path is built from an account's config dir plus names taken
//! from the scan, then checked here: the account root is canonicalized,
//! each name must be one plain path component, and nothing below the root
//! may be a symlink (a symlink could point anywhere). Transcript ids are
//! UUIDs ([`super::meta::is_uuid`]).

use std::path::{Path, PathBuf};

/// One plain path component: not empty, `.` or `..`, no `/`, no control
/// characters.
pub(crate) fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.chars().any(char::is_control)
}

/// The canonical `<config_dir>/<sub>` (`projects`, `session-env`,
/// `file-history`). The config dir itself may be a symlink; the
/// subfolder must be a real directory.
pub(crate) fn account_dir(config_dir: &Path, sub: &str) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(config_dir)
        .map_err(|_| format!("{} is gone", config_dir.display()))?;
    let p = root.join(sub);
    match std::fs::symlink_metadata(&p) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(p),
        Ok(_) => Err(format!("{} is not a plain folder", p.display())),
        Err(_) => Err(format!("{} is missing", p.display())),
    }
}

/// `root/name`, a real directory (not a symlink) directly in `root`.
pub(crate) fn real_dir_in(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !plain_name(name) {
        return Err("Refused an unsafe folder name".into());
    }
    let p = root.join(name);
    match std::fs::symlink_metadata(&p) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(p),
        Ok(_) => Err(format!("{} is not a plain folder", p.display())),
        Err(_) => Err(format!("{} is gone", p.display())),
    }
}

/// A session's transcript, checked: `(projects root, project folder,
/// transcript)`.
pub(crate) fn transcript(
    config_dir: &Path,
    project_dir: &str,
    id: &str,
) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    if !super::meta::is_uuid(id) {
        return Err("Refused: not a session id".into());
    }
    let projects = account_dir(config_dir, "projects")?;
    let folder = real_dir_in(&projects, project_dir)?;
    let file = folder.join(format!("{id}.jsonl"));
    match std::fs::symlink_metadata(&file) {
        Ok(m) if m.is_file() => {}
        Ok(_) => return Err("The transcript is not a plain file".into()),
        Err(_) => return Err("The transcript is gone".into()),
    }
    // Belt and braces: the resolved path must still be under the root.
    let real = std::fs::canonicalize(&file).map_err(|e| e.to_string())?;
    if !real.starts_with(&projects) || real != file {
        return Err("Refused a path outside the Claude projects folder".into());
    }
    Ok((projects, folder, file))
}

/// Remove `path` (a direct child of a checked folder): a symlink loses
/// the link only, a directory goes with its contents (`remove_dir_all`
/// does not follow symlinks inside it), a file is unlinked.
pub(crate) fn remove_entry(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() || m.is_file() => {
            std::fs::remove_file(path).map(|()| true)
        }
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path).map(|()| true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// `rename(2)` that never replaces an existing `to`
/// (`RENAME_NOREPLACE`). On a filesystem without it a file falls back to
/// link + unlink (`link` refuses an existing target too); a directory is
/// refused.
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| {
        CString::new(p.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
    };
    let (src, dst) = (c(from)?, c(to)?);
    // SAFETY: both pointers are valid NUL-terminated strings that outlive
    // the call; renameat2 only reads them.
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            src.as_ptr(),
            libc::AT_FDCWD,
            dst.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    let unsupported = matches!(err.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS));
    if unsupported && std::fs::symlink_metadata(from).is_ok_and(|m| m.is_file()) {
        std::fs::hard_link(from, to)?;
        return std::fs::remove_file(from);
    }
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "cae70d77-c84d-45b9-b9ef-3a39caf81fcb";

    #[test]
    fn names_are_single_components() {
        assert!(plain_name("-home-kenan--cachy"));
        for bad in ["", ".", "..", "a/b", "../x", "a\nb"] {
            assert!(!plain_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn transcript_refuses_symlinks_and_escapes() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join(".claude");
        let proj = cfg.join("projects/-w");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join(format!("{ID}.jsonl")), "{}\n").unwrap();
        let (_, folder, file) = transcript(&cfg, "-w", ID).unwrap();
        assert_eq!(folder, std::fs::canonicalize(&proj).unwrap());
        assert!(file.ends_with(format!("{ID}.jsonl")));
        assert!(transcript(&cfg, "..", ID).is_err());
        assert!(transcript(&cfg, "-w", "../../x").is_err());
        assert!(transcript(&cfg, "-missing", ID).is_err());
        // A symlinked project folder or transcript is refused.
        let outside = d.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join(format!("{ID}.jsonl")), "{}\n").unwrap();
        std::os::unix::fs::symlink(&outside, cfg.join("projects/-link")).unwrap();
        assert!(transcript(&cfg, "-link", ID).is_err());
        let p2 = cfg.join("projects/-x");
        std::fs::create_dir_all(&p2).unwrap();
        std::os::unix::fs::symlink(
            outside.join(format!("{ID}.jsonl")),
            p2.join(format!("{ID}.jsonl")),
        )
        .unwrap();
        assert!(transcript(&cfg, "-x", ID).is_err());
        // A symlinked config dir itself is fine (canonicalized).
        let alias = d.path().join("alias");
        std::os::unix::fs::symlink(&cfg, &alias).unwrap();
        assert!(transcript(&alias, "-w", ID).is_ok());
    }

    #[test]
    fn remove_entry_unlinks_a_symlink_not_its_target() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("target");
        std::fs::create_dir_all(target.join("inner")).unwrap();
        std::fs::write(target.join("keep"), "x").unwrap();
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(remove_entry(&link).unwrap());
        assert!(target.join("keep").exists());
        assert!(remove_entry(&target).unwrap());
        assert!(!target.exists());
        assert!(!remove_entry(&target).unwrap());
    }

    #[test]
    fn rename_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        assert!(rename_noreplace(&a, &b).is_err());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b");
        std::fs::remove_file(&b).unwrap();
        rename_noreplace(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "a");
    }
}
