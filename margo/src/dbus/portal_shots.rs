//! Where the Screenshot portal's PNGs live, and how long.
//!
//! `grim` writes each portal screenshot to a file whose `file://` URI the
//! requesting app gets back. They used to be
//! `$XDG_RUNTIME_DIR/margo-portal-shot-<millis>.png` (or `/tmp/…` without a
//! runtime dir): never deleted, so every shot (a password manager, a chat)
//! stayed in RAM-backed tmpfs for the whole session, and two requests in
//! the same millisecond shared one file, so an app granted one region could
//! read another request's full-screen shot. In `/tmp` the name was
//! guessable and grim follows symlinks.
//!
//! Now: one `0700` directory under `$XDG_RUNTIME_DIR` (no runtime dir = no
//! shot), a fresh random name per shot created with `O_EXCL | O_NOFOLLOW`
//! at `0600`, and a sweep that removes shots older than [`MAX_AGE`] (and,
//! at startup, every leftover).

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Subdirectory of `$XDG_RUNTIME_DIR` holding the shots.
const DIR_NAME: &str = "margo-portal-shots";
/// Name prefix of a shot file (also what the sweep removes).
const PREFIX: &str = "shot-";
/// The pre-4.9.3 layout: files straight in the runtime dir.
const LEGACY_PREFIX: &str = "margo-portal-shot-";

/// How long an app has to read its screenshot before the sweep removes it.
/// Apps read the URI right away; a sandboxed app reads it through the
/// document portal, which needs the file to still exist.
pub const MAX_AGE: Duration = Duration::from_secs(10 * 60);

/// The shot directory under `runtime`, created `0700` if missing. Refuses
/// one that is a symlink, not a directory, or not owned by us; tightens an
/// owned directory with looser permissions. `runtime` is
/// `$XDG_RUNTIME_DIR`; without one there is no private place to write, so
/// this is an error (never a `/tmp` fallback).
pub fn shot_dir(runtime: Option<&OsStr>) -> io::Result<PathBuf> {
    let runtime = runtime
        .filter(|r| !r.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    let runtime = Path::new(runtime);
    // XDG_RUNTIME_DIR must itself be private. Do not trust an inherited
    // relative path, symlink or shared directory as a security boundary.
    let meta = fs::symlink_metadata(runtime)?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    if !runtime.is_absolute()
        || !meta.file_type().is_dir()
        || meta.uid() != uid
        || meta.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime directory is not private",
        ));
    }
    let dir = runtime.join(DIR_NAME);
    match fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = fs::symlink_metadata(&dir)?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    if !meta.file_type().is_dir() || meta.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a directory owned by us", dir.display()),
        ));
    }
    if meta.mode() & 0o077 != 0 {
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

/// Create a new, empty shot file with a random name in `dir` and return its
/// path. `O_EXCL | O_NOFOLLOW`, mode `0600`: the name is ours alone.
pub fn reserve_shot(dir: &Path) -> io::Result<PathBuf> {
    for _ in 0..8 {
        let path = dir.join(format!("{PREFIX}{}.png", random_hex()?));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free screenshot name",
    ))
}

/// Remove shots in `dir` last modified more than `max_age` before `now`
/// (`Duration::ZERO` = all of them). Returns how many were removed.
pub fn sweep(dir: &Path, now: SystemTime, max_age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(PREFIX))
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .map(|t| now.duration_since(t).unwrap_or_default() >= max_age)
                .unwrap_or(true)
        })
        .filter(|e| fs::remove_file(e.path()).is_ok())
        .count()
}

/// At startup: every shot a previous margo left behind, in the shot
/// directory and in the old layout straight under `runtime`.
pub fn sweep_leftovers(runtime: &Path) -> usize {
    let legacy = fs::read_dir(runtime)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    name.starts_with(LEGACY_PREFIX) && name.ends_with(".png")
                })
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter(|e| fs::remove_file(e.path()).is_ok())
                .count()
        })
        .unwrap_or(0);
    legacy + sweep(&runtime.join(DIR_NAME), SystemTime::now(), Duration::ZERO)
}

fn random_hex() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        d
    }

    #[test]
    fn no_runtime_dir_means_no_shot_rather_than_tmp() {
        assert!(shot_dir(None).is_err());
        assert!(shot_dir(Some(OsStr::new(""))).is_err());
    }

    #[test]
    fn the_shot_dir_is_private_and_reused() {
        let runtime = runtime_dir();
        let dir = shot_dir(Some(runtime.path().as_os_str())).unwrap();
        assert_eq!(dir, runtime.path().join(DIR_NAME));
        let mode = fs::metadata(&dir).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // A loosened directory is tightened again.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(shot_dir(Some(runtime.path().as_os_str())).unwrap(), dir);
        assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
    }

    #[test]
    fn a_symlinked_shot_dir_is_refused() {
        let runtime = runtime_dir();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), runtime.path().join(DIR_NAME)).unwrap();
        assert!(shot_dir(Some(runtime.path().as_os_str())).is_err());
    }

    #[test]
    fn every_shot_gets_its_own_private_file() {
        let runtime = runtime_dir();
        let dir = shot_dir(Some(runtime.path().as_os_str())).unwrap();
        let a = reserve_shot(&dir).unwrap();
        let b = reserve_shot(&dir).unwrap();
        assert_ne!(a, b, "two requests must never share a file");
        for p in [&a, &b] {
            assert!(p.starts_with(&dir));
            assert_eq!(fs::metadata(p).unwrap().mode() & 0o777, 0o600);
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            assert!(name.starts_with(PREFIX) && name.ends_with(".png"), "{name}");
            // 128 random bits, not a timestamp.
            assert_eq!(name.len(), PREFIX.len() + 32 + 4, "{name}");
        }
    }

    #[test]
    fn the_sweep_removes_old_shots_only() {
        let runtime = runtime_dir();
        let dir = shot_dir(Some(runtime.path().as_os_str())).unwrap();
        let shot = reserve_shot(&dir).unwrap();
        let other = dir.join("not-a-shot.txt");
        fs::write(&other, "x").unwrap();
        let now = SystemTime::now();
        assert_eq!(sweep(&dir, now, MAX_AGE), 0, "a fresh shot was removed");
        assert!(shot.exists());
        assert_eq!(
            sweep(&dir, now + MAX_AGE + Duration::from_secs(1), MAX_AGE),
            1
        );
        assert!(!shot.exists());
        assert!(other.exists(), "the sweep only touches shot files");
    }

    #[test]
    fn startup_removes_every_leftover_including_the_old_layout() {
        let runtime = runtime_dir();
        let dir = shot_dir(Some(runtime.path().as_os_str())).unwrap();
        let fresh = reserve_shot(&dir).unwrap();
        let legacy = runtime.path().join("margo-portal-shot-1700000000000.png");
        fs::write(&legacy, "png").unwrap();
        let unrelated = runtime.path().join("margo-other.png");
        fs::write(&unrelated, "png").unwrap();
        assert_eq!(sweep_leftovers(runtime.path()), 2);
        assert!(!fresh.exists() && !legacy.exists());
        assert!(unrelated.exists());
    }
}
