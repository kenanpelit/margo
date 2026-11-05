//! `kill`: signal a process only while it is still the one the user
//! picked. The pid is pinned with a pidfd first; then, as root, its
//! `/proc/<pid>/stat` start time is compared with the one the table
//! showed and kernel threads are refused; only then does the signal go
//! through the pidfd, so a pid reused while the dialog was open is never
//! hit.

use crate::{Error, Signal};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

/// `PF_KTHREAD` in `/proc/<pid>/stat`'s flags (field 9).
const PF_KTHREAD: u64 = 0x0020_0000;

/// Start time (field 22) and flags (field 9) from a `stat` line. `comm`
/// (field 2) may hold spaces and `)`, so the fields are split after the
/// last `)`.
pub fn parse_stat(stat: &str) -> Option<(u64, u64)> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // rest[0] is field 3 (state): field n sits at n - 3.
    let flags = f.get(9 - 3)?.parse().ok()?;
    let start = f.get(22 - 3)?.parse().ok()?;
    Some((start, flags))
}

fn gone(pid: i32) -> Error {
    Error::Refused(format!("Process {pid} has already exited"))
}

/// Re-check `pid` against `start` under `proc_root` (as root). A kernel
/// thread or this helper itself is refused.
pub fn check(proc_root: &Path, pid: i32, start: u64) -> Result<(), Error> {
    if pid as u32 == std::process::id() {
        return Err(Error::Refused(
            "refusing to signal the helper itself".into(),
        ));
    }
    let stat = std::fs::read_to_string(proc_root.join(pid.to_string()).join("stat"))
        .map_err(|_| gone(pid))?;
    let (started, flags) = parse_stat(&stat).ok_or_else(|| gone(pid))?;
    if flags & PF_KTHREAD != 0 {
        return Err(Error::Refused(format!(
            "refusing to signal kernel thread {pid}"
        )));
    }
    if started != start {
        return Err(gone(pid));
    }
    Ok(())
}

fn send_error(pid: i32, e: std::io::Error) -> Error {
    match e.raw_os_error() {
        Some(libc::ESRCH) => gone(pid),
        Some(libc::EPERM) => Error::Io(format!("not permitted to signal {pid}")),
        _ => Error::Io(format!("signal {pid}: {e}")),
    }
}

/// Send `signal` to `pid` if it still started at `start`.
pub fn kill(pid: i32, start: u64, signal: Signal) -> Result<(), Error> {
    let proc_root = Path::new("/proc");
    // SAFETY: plain syscall with integer arguments.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::ENOSYS) {
            return Err(send_error(pid, e));
        }
        // Kernel < 5.3: check, then kill(2) (the old window remains).
        check(proc_root, pid, start)?;
        // SAFETY: integer arguments only.
        let r = unsafe { libc::kill(pid, signal.number()) };
        return if r == 0 {
            Ok(())
        } else {
            Err(send_error(pid, std::io::Error::last_os_error()))
        };
    }
    let fd = i32::try_from(fd).map_err(|_| Error::Io("pidfd out of range".into()))?;
    // SAFETY: `fd` was just returned by pidfd_open and is owned here only.
    let pidfd = unsafe { OwnedFd::from_raw_fd(fd) };
    // The pidfd pins this process: if it still started at `start`, the
    // signal below can't reach a newcomer reusing the number.
    check(proc_root, pid, start)?;
    // SAFETY: valid pidfd, null siginfo (as kill(2)), no flags.
    let r = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            signal.number(),
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(send_error(pid, std::io::Error::last_os_error()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_fields_survive_a_hostile_comm() {
        // comm = "a) 0 0 (b" — spaces and parens inside the name.
        let mut fields = vec!["S"; 50];
        fields[9 - 3] = "4194560";
        fields[22 - 3] = "777";
        let line = format!("42 (a) 0 0 (b) {}", fields.join(" "));
        assert_eq!(parse_stat(&line), Some((777, 4194560)));
        assert_eq!(parse_stat("42 (x"), None);
        assert_eq!(parse_stat("garbage"), None);
    }

    fn sleeper() -> (std::process::Child, i32, u64) {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id() as i32;
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("stat");
        (child, pid, parse_stat(&stat).expect("parse").0)
    }

    #[test]
    fn a_reused_pid_is_not_signalled() {
        let (mut child, pid, start) = sleeper();
        assert_eq!(kill(pid, start + 1, Signal::Term), Err(gone(pid)));
        assert!(child.try_wait().expect("wait").is_none(), "still running");
        assert_eq!(kill(pid, start, Signal::Kill), Ok(()));
        assert!(!child.wait().expect("wait").success());
        assert_eq!(kill(pid, start, Signal::Kill), Err(gone(pid)), "reaped");
    }

    #[test]
    fn kernel_threads_and_the_helper_are_refused() {
        // A fixture /proc, not the real one: inside a container's pid
        // namespace (CI) there is no kthreadd at pid 2, nor any kernel thread.
        let root = std::env::temp_dir().join(format!("mshell-procsig-{}", std::process::id()));
        std::fs::create_dir_all(root.join("2")).expect("fixture dir");
        let start = 7u64;
        let stat = |flags: u64| {
            format!("2 (kthreadd) S 0 0 0 0 -1 {flags} 0 0 0 0 0 0 0 0 20 0 1 0 {start} 0 0\n")
        };
        std::fs::write(root.join("2/stat"), stat(PF_KTHREAD | 0x40)).expect("fixture stat");
        assert_eq!(parse_stat(&stat(PF_KTHREAD)), Some((start, PF_KTHREAD)));
        assert!(
            matches!(check(&root, 2, start), Err(Error::Refused(m)) if m.contains("kernel thread"))
        );
        // The same task without PF_KTHREAD passes the check.
        std::fs::write(root.join("2/stat"), stat(0x40)).expect("fixture stat");
        assert_eq!(check(&root, 2, start), Ok(()));
        let _ = std::fs::remove_dir_all(&root);
        let me = std::process::id() as i32;
        assert!(matches!(
            check(Path::new("/proc"), me, 0),
            Err(Error::Refused(_))
        ));
    }

    #[test]
    fn a_foreign_process_is_reported_not_permitted() {
        // As an ordinary user, another user's process fails with EPERM
        // after the checks pass: the error says so instead of "gone".
        // SAFETY: getter.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let Some((pid, start)) = std::fs::read_dir("/proc")
            .expect("proc")
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
            .filter(|p| *p > 2)
            .find_map(|p| {
                // Every uid (real, effective, saved, fs) foreign: kill(2)
                // is then refused for certain, so nothing is signalled.
                let status = std::fs::read_to_string(format!("/proc/{p}/status")).ok()?;
                let uids = status.lines().find_map(|l| l.strip_prefix("Uid:"))?;
                // SAFETY: getter.
                let me = unsafe { libc::getuid() }.to_string();
                let foreign = uids.split_whitespace().all(|u| u != me);
                let stat = std::fs::read_to_string(format!("/proc/{p}/stat")).ok()?;
                let (start, flags) = parse_stat(&stat)?;
                (foreign && flags & PF_KTHREAD == 0).then_some((p, start))
            })
        else {
            return;
        };
        assert_eq!(
            kill(pid, start, Signal::Term),
            Err(Error::Io(format!("not permitted to signal {pid}")))
        );
    }
}
