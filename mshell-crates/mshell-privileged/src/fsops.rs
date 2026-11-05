//! File-system side of the helper. Paths that passed [`crate::validate`]
//! are opened one directory at a time with `O_NOFOLLOW`, so neither the
//! target nor any directory on the way can be a symlink, and every write
//! happens relative to that directory fd: a temp file created
//! `O_EXCL`, given its mode/owner, synced, then renamed into place.
//! Nothing here follows a link or creates a file outside the validated
//! directory.

use crate::Error;
use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// A merged file larger than this is refused (the panel reviews config
/// files, not data).
pub const MAX_CONTENT: u64 = 16 * 1024 * 1024;

/// Where the logical paths (`/etc/…`, `/var/cache/pacman/pkg/…`,
/// `/proc/acpi/ibm/fan`, `/usr/bin/pacman`) live. `/` for every real
/// run; a fixture tree only for an unprivileged test run
/// ([`crate::run::sysroot_from_env`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sysroot(PathBuf);

impl Sysroot {
    pub fn real() -> Sysroot {
        Sysroot(PathBuf::from("/"))
    }

    /// A test tree (canonicalized, so its own ancestors are no concern).
    pub fn fixture(dir: &Path) -> Result<Sysroot, Error> {
        dir.canonicalize()
            .map(Sysroot)
            .map_err(|e| Error::Io(format!("sysroot {}: {e}", dir.display())))
    }

    pub fn is_real(&self) -> bool {
        self.0 == Path::new("/")
    }

    /// The physical path of a logical absolute path.
    pub fn phys(&self, logical: &Path) -> PathBuf {
        self.0.join(logical.strip_prefix("/").unwrap_or(logical))
    }
}

fn cstr(name: &OsStr) -> Result<CString, Error> {
    CString::new(name.as_bytes()).map_err(|_| Error::Invalid("NUL byte in a path".into()))
}

fn errno() -> std::io::Error {
    std::io::Error::last_os_error()
}

/// `openat(dir, name, flags | O_CLOEXEC | O_NOFOLLOW, mode)`.
fn openat(dir: &OwnedFd, name: &OsStr, flags: i32, mode: u32) -> std::io::Result<OwnedFd> {
    let c = cstr(name).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: valid dirfd and NUL-terminated name; the returned fd is
    // owned by the OwnedFd below and nowhere else.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            c.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(errno());
    }
    // SAFETY: `fd` was just returned by openat and is not shared.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The type of `name` in `dir` without following it, `None` when absent.
fn lstat_at(dir: &OwnedFd, name: &OsStr) -> Result<Option<libc::mode_t>, Error> {
    let c = cstr(name)?;
    // SAFETY: zeroed `stat` is a valid out-parameter for fstatat.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: valid dirfd, NUL-terminated name, `st` is writable.
    let r = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            c.as_ptr(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if r == 0 {
        return Ok(Some(st.st_mode & libc::S_IFMT));
    }
    let e = errno();
    if e.raw_os_error() == Some(libc::ENOENT) {
        Ok(None)
    } else {
        Err(Error::Io(format!("{}: {e}", name.to_string_lossy())))
    }
}

fn type_name(t: libc::mode_t) -> &'static str {
    match t {
        libc::S_IFLNK => "a symlink",
        libc::S_IFDIR => "a directory",
        libc::S_IFIFO => "a FIFO",
        libc::S_IFSOCK => "a socket",
        libc::S_IFCHR | libc::S_IFBLK => "a device",
        _ => "not a regular file",
    }
}

/// A file opened from a validated directory: the dir fd plus the name.
pub struct InDir {
    pub dir: OwnedFd,
    pub name: OsString,
    /// The logical path, for messages.
    pub shown: PathBuf,
}

/// Open the directory holding `logical` (which must be below `top`),
/// one component at a time from `top`, never following a symlink.
pub fn open_parent(root: &Sysroot, top: &str, logical: &Path) -> Result<InDir, Error> {
    let shown = logical.to_path_buf();
    let rel = logical
        .strip_prefix(top)
        .map_err(|_| Error::Invalid(format!("{} is not below {top}", shown.display())))?;
    let mut parts: Vec<&OsStr> = rel.iter().collect();
    let name = parts
        .pop()
        .ok_or_else(|| Error::Invalid(format!("{} has no file name", shown.display())))?
        .to_os_string();
    let refuse = |what: &Path, e: std::io::Error| match e.raw_os_error() {
        Some(libc::ELOOP) | Some(libc::ENOTDIR) => Error::Refused(format!(
            "refusing {}: a symlink or not a directory on the way",
            what.display()
        )),
        _ => Error::Io(format!("{}: {e}", what.display())),
    };
    // `top` itself (/etc, /var/cache/pacman/pkg) is opened from `/` the
    // same way, so a symlinked /etc is refused too.
    let base = root.phys(Path::new("/"));
    let c = cstr(base.as_os_str())?;
    // SAFETY: NUL-terminated path; the fd goes straight into an OwnedFd.
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(Error::Io(format!("{}: {}", base.display(), errno())));
    }
    // SAFETY: just opened, not shared.
    let mut dir = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut walked = PathBuf::from("/");
    let top_parts: Vec<&OsStr> = Path::new(top).iter().skip(1).collect();
    for part in top_parts.into_iter().chain(parts) {
        walked.push(part);
        dir = openat(&dir, part, libc::O_RDONLY | libc::O_DIRECTORY, 0)
            .map_err(|e| refuse(&walked, e))?;
    }
    Ok(InDir { dir, name, shown })
}

/// `name` in `d` must be a regular file (not a symlink, FIFO, device,
/// directory). `Ok(false)` when it doesn't exist.
fn regular_or_absent(d: &InDir, name: &OsStr, shown: &Path) -> Result<bool, Error> {
    match lstat_at(&d.dir, name)? {
        None => Ok(false),
        Some(libc::S_IFREG) => Ok(true),
        Some(t) => Err(Error::Refused(format!(
            "refusing {}: {}",
            shown.display(),
            type_name(t)
        ))),
    }
}

/// Require an existing regular file at `logical` below `top`.
pub fn require_regular(root: &Sysroot, top: &str, logical: &Path) -> Result<InDir, Error> {
    let d = open_parent(root, top, logical)?;
    if !regular_or_absent(&d, &d.name.clone(), logical)? {
        return Err(Error::Refused(format!(
            "{} does not exist",
            logical.display()
        )));
    }
    Ok(d)
}

fn read_capped(mut r: impl Read, what: &Path) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    (&mut r)
        .take(MAX_CONTENT + 1)
        .read_to_end(&mut buf)
        .map_err(|e| Error::Io(format!("reading {}: {e}", what.display())))?;
    if buf.len() as u64 > MAX_CONTENT {
        return Err(Error::Invalid(format!(
            "{} is larger than 16 MiB",
            what.display()
        )));
    }
    Ok(buf)
}

/// Open a regular file in `d` read-only, re-checking the type on the
/// open fd (the name could have been swapped after the lstat).
fn open_regular(d: &InDir, name: &OsStr, shown: &Path) -> Result<File, Error> {
    let fd = openat(&d.dir, name, libc::O_RDONLY | libc::O_NONBLOCK, 0).map_err(|e| {
        if e.raw_os_error() == Some(libc::ELOOP) {
            Error::Refused(format!("refusing {}: a symlink", shown.display()))
        } else {
            Error::Io(format!("{}: {e}", shown.display()))
        }
    })?;
    let f = File::from(fd);
    let m = f
        .metadata()
        .map_err(|e| Error::Io(format!("{}: {e}", shown.display())))?;
    if !m.file_type().is_file() {
        return Err(Error::Refused(format!(
            "refusing {}: not a regular file",
            shown.display()
        )));
    }
    Ok(f)
}

/// Mode bits, owner and times a written file gets.
struct Attrs {
    mode: u32,
    uid: u32,
    gid: u32,
    times: Option<std::fs::FileTimes>,
}

/// Write `bytes` to a fresh temp file in `d` with `attrs`, sync it and
/// rename it over `target`. The temp file is removed on any error.
fn replace(d: &InDir, target: &OsStr, bytes: &[u8], attrs: &Attrs) -> Result<(), Error> {
    let mut n = 0u32;
    let (tmp, fd) = loop {
        let mut name = OsString::from(".");
        name.push(target);
        name.push(format!(".mshell-tmp.{}.{n}", std::process::id()));
        match openat(
            &d.dir,
            &name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        ) {
            Ok(fd) => break (name, fd),
            Err(e) if e.raw_os_error() == Some(libc::EEXIST) && n < 100 => n += 1,
            Err(e) => {
                return Err(Error::Io(format!(
                    "temp file in {}: {e}",
                    d.shown.display()
                )));
            }
        }
    };
    let mut f = File::from(fd);
    let res = (|| -> std::io::Result<()> {
        f.write_all(bytes)?;
        std::os::unix::fs::fchown(&f, Some(attrs.uid), Some(attrs.gid))?;
        f.set_permissions(std::fs::Permissions::from_mode(attrs.mode))?;
        if let Some(t) = attrs.times {
            f.set_times(t)?;
        }
        f.sync_all()?;
        rename_at(&d.dir, &tmp, target)
    })();
    if let Err(e) = res {
        let _ = unlink_at(&d.dir, &tmp);
        return Err(Error::Io(format!(
            "writing {}: {e}",
            d.shown.with_file_name(target).display()
        )));
    }
    Ok(())
}

fn rename_at(dir: &OwnedFd, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
    let (f, t) = (CString::new(from.as_bytes())?, CString::new(to.as_bytes())?);
    // SAFETY: valid dirfd and NUL-terminated names.
    let r = unsafe { libc::renameat(dir.as_raw_fd(), f.as_ptr(), dir.as_raw_fd(), t.as_ptr()) };
    if r == 0 { Ok(()) } else { Err(errno()) }
}

fn unlink_at(dir: &OwnedFd, name: &OsStr) -> std::io::Result<()> {
    let c = CString::new(name.as_bytes())?;
    // SAFETY: valid dirfd and NUL-terminated name; removes the entry
    // itself, never what a symlink points to.
    let r = unsafe { libc::unlinkat(dir.as_raw_fd(), c.as_ptr(), 0) };
    if r == 0 { Ok(()) } else { Err(errno()) }
}

fn sync_dir(dir: &OwnedFd) {
    // SAFETY: valid fd. A failed directory fsync is not worth failing
    // an install that already renamed into place.
    let _ = unsafe { libc::fsync(dir.as_raw_fd()) };
}

/// Where a pacnew install takes its new content from.
pub enum Source<'a> {
    /// The review file itself.
    Review,
    /// A merge read from this stream (stdin).
    Stream(&'a mut dyn Read),
}

/// Replace `live` with the new content (the review file's or a merge),
/// keeping the live file's mode/owner and a `<live>.mshell-bak` copy
/// (mode, owner and times preserved), then remove `review`. A new live
/// file (restoring a `.pacsave`) is `0644` owned by the caller (root).
pub fn pacnew_install(
    root: &Sysroot,
    live: &Path,
    review: &Path,
    src: Source<'_>,
) -> Result<(), Error> {
    let d = open_parent(root, crate::validate::ETC, live)?;
    let review_name = review
        .file_name()
        .ok_or_else(|| Error::Invalid("review file has no name".into()))?
        .to_os_string();
    if review.parent() != live.parent() {
        return Err(Error::Invalid(
            "live and review file are in different directories".into(),
        ));
    }
    if !regular_or_absent(&d, &review_name, review)? {
        return Err(Error::Refused(format!(
            "{} does not exist",
            review.display()
        )));
    }
    let live_name = d.name.clone();
    let live_exists = regular_or_absent(&d, &live_name, live)?;
    let content = match src {
        Source::Review => read_capped(open_regular(&d, &review_name, review)?, review)?,
        Source::Stream(r) => read_capped(r, Path::new("the merged text"))?,
    };
    let attrs = if live_exists {
        let old = open_regular(&d, &live_name, live)?;
        let m = old
            .metadata()
            .map_err(|e| Error::Io(format!("{}: {e}", live.display())))?;
        let times = std::fs::FileTimes::new()
            .set_accessed(m.accessed().unwrap_or(std::time::UNIX_EPOCH))
            .set_modified(m.modified().unwrap_or(std::time::UNIX_EPOCH));
        let attrs = Attrs {
            mode: m.mode() & 0o7777,
            uid: m.uid(),
            gid: m.gid(),
            times: Some(times),
        };
        let backup = read_capped(old, live)?;
        let mut bak = live_name.clone();
        bak.push(".mshell-bak");
        replace(&d, &bak, &backup, &attrs)?;
        Attrs {
            times: None,
            ..attrs
        }
    } else {
        // SAFETY: getters with no arguments.
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        Attrs {
            mode: 0o644,
            uid,
            gid,
            times: None,
        }
    };
    replace(&d, &live_name, &content, &attrs)?;
    unlink_at(&d.dir, &review_name)
        .map_err(|e| Error::Io(format!("removing {}: {e}", review.display())))?;
    sync_dir(&d.dir);
    Ok(())
}

/// Delete a review file (a regular file; a symlink is refused).
pub fn pacnew_remove(root: &Sysroot, review: &Path) -> Result<(), Error> {
    let d = require_regular(root, crate::validate::ETC, review)?;
    unlink_at(&d.dir, &d.name)
        .map_err(|e| Error::Io(format!("removing {}: {e}", review.display())))?;
    sync_dir(&d.dir);
    Ok(())
}

/// The thinkpad_acpi control file.
pub const FAN_FILE: &str = "/proc/acpi/ibm/fan";

/// Write `level <keyword>\n` to [`FAN_FILE`]: opened write-only without
/// `O_CREAT` or following a link, so nothing else can be written.
pub fn fan_set(root: &Sysroot, level: crate::FanLevel) -> Result<(), Error> {
    let path = root.phys(Path::new(FAN_FILE));
    let c = cstr(path.as_os_str())?;
    // SAFETY: NUL-terminated path; no O_CREAT, so no mode argument.
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_WRONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        let e = errno();
        return Err(match e.raw_os_error() {
            Some(libc::ELOOP) => Error::Refused(format!("refusing {FAN_FILE}: a symlink")),
            Some(libc::ENOENT) => {
                Error::Refused(format!("{FAN_FILE} is missing (no thinkpad_acpi)"))
            }
            _ => Error::Io(format!("{FAN_FILE}: {e}")),
        });
    }
    // SAFETY: just opened, not shared.
    let mut f = File::from(unsafe { OwnedFd::from_raw_fd(fd) });
    f.write_all(format!("level {}\n", level.keyword()).as_bytes())
        .map_err(|e| Error::Io(format!("{FAN_FILE}: {e} (is fan_control=1 set?)")))
}
