//! Find processes an earlier mshell started with [`crate::outliving_shell`]
//! again after a shell restart, without ever mistaking someone else's
//! process for one of ours.
//!
//! A process counts as ours only when all of these hold (read from
//! `/proc/<pid>`):
//! - its real uid is our own uid (`status`, `Uid:`);
//! - it runs in a transient scope we named, exactly
//!   `app-mshell-<label>-<pid>-<seq>.scope` (`cgroup`, the unified `0::` line);
//! - the caller's argv check passes (`cmdline`).
//!
//! A program the user started by hand from a terminal has no such scope, so
//! it is never adopted. The kernel start time (`stat`, field 22) pins the
//! pid: an [`Identity`] stays valid only for that very process, so a pid
//! reused after it exits is never signalled.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

/// The procfs root used in production.
pub const PROC: &str = "/proc";

/// A process pinned by pid + kernel start time (clock ticks since boot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity {
    pub pid: u32,
    pub start_ticks: u64,
}

/// What `/proc/<pid>` says about one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    /// Real uid.
    pub uid: u32,
    pub start_ticks: u64,
    pub argv: Vec<OsString>,
    /// Leaf of the unified cgroup path (e.g. `app-mshell-scrcpy-12-0.scope`).
    pub unit: Option<String>,
}

impl ProcInfo {
    pub fn identity(&self) -> Identity {
        Identity {
            pid: self.pid,
            start_ticks: self.start_ticks,
        }
    }

    /// Runs in a scope [`crate::outliving_shell`] created with `label`.
    pub fn in_mshell_scope(&self, label: &str) -> bool {
        self.unit
            .as_deref()
            .and_then(mshell_scope_label)
            .is_some_and(|l| l == label)
    }

    /// File name of `argv[0]` (`/usr/bin/scrcpy` → `scrcpy`).
    pub fn program_name(&self) -> Option<&std::ffi::OsStr> {
        let first = self.argv.first()?;
        Path::new(first).file_name()
    }
}

/// Kernel start time from `/proc/<pid>/stat` (field 22). `comm` (field 2)
/// may contain spaces and parentheses, so fields are counted after the
/// last `)`.
pub fn parse_start_ticks(stat: &str) -> Option<u64> {
    let (_, rest) = stat.rsplit_once(')')?;
    // rest = " S ppid …": field 3 is the first token, so field 22 is the 20th.
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// Real uid from `/proc/<pid>/status`.
pub fn parse_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Leaf unit of the unified (`0::`) line of `/proc/<pid>/cgroup`.
pub fn parse_unit(cgroup: &str) -> Option<String> {
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    let leaf = path.trim().rsplit('/').next()?;
    (leaf.ends_with(".scope") || leaf.ends_with(".service")).then(|| leaf.to_string())
}

/// `label` of a unit named exactly `app-mshell-<label>-<pid>-<seq>.scope`
/// (see [`crate::outliving_shell`]); `None` for any other unit.
pub fn mshell_scope_label(unit: &str) -> Option<&str> {
    let body = unit.strip_prefix("app-mshell-")?.strip_suffix(".scope")?;
    let mut parts = body.rsplitn(3, '-');
    let seq = parts.next()?;
    let pid = parts.next()?;
    let label = parts.next()?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(seq) && digits(pid) && !label.is_empty()).then_some(label)
}

/// Read one process from `<proc_root>/<pid>`; `None` once it is gone or
/// unreadable.
pub fn read_in(proc_root: &Path, pid: u32) -> Option<ProcInfo> {
    let dir = proc_root.join(pid.to_string());
    let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
    let status = std::fs::read_to_string(dir.join("status")).ok()?;
    let cmdline = std::fs::read(dir.join("cmdline")).ok()?;
    let cgroup = std::fs::read_to_string(dir.join("cgroup")).unwrap_or_default();
    let argv = cmdline
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| OsString::from_vec(a.to_vec()))
        .collect();
    Some(ProcInfo {
        pid,
        uid: parse_uid(&status)?,
        start_ticks: parse_start_ticks(&stat)?,
        argv,
        unit: parse_unit(&cgroup),
    })
}

/// Our own real uid.
pub fn own_uid() -> Option<u32> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .as_deref()
        .and_then(parse_uid)
}

/// Every process of `uid` running in an `app-mshell-<label>-….scope` whose
/// argv passes `argv_ok`.
pub fn find_in(
    proc_root: &Path,
    uid: u32,
    label: &str,
    argv_ok: impl Fn(&[OsString]) -> bool,
) -> Vec<ProcInfo> {
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let mut found: Vec<ProcInfo> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| read_in(proc_root, pid))
        .filter(|p| p.uid == uid && p.in_mshell_scope(label) && argv_ok(&p.argv))
        .collect();
    found.sort_by_key(|p| p.pid);
    found
}

/// [`find_in`] over the real `/proc` for our own uid.
pub fn find_own(label: &str, argv_ok: impl Fn(&[OsString]) -> bool) -> Vec<ProcInfo> {
    match own_uid() {
        Some(uid) => find_in(Path::new(PROC), uid, label, argv_ok),
        None => Vec::new(),
    }
}

/// `id` still names the same live process, owned by `uid`.
pub fn alive_in(proc_root: &Path, id: Identity, uid: u32) -> bool {
    read_in(proc_root, id.pid).is_some_and(|p| p.start_ticks == id.start_ticks && p.uid == uid)
}

/// [`alive_in`] over the real `/proc` for our own uid.
pub fn alive(id: Identity) -> bool {
    own_uid().is_some_and(|uid| alive_in(Path::new(PROC), id, uid))
}

/// SIGTERM `id` if it is still that same process of ours. Returns whether
/// the signal was sent.
pub fn terminate(id: Identity) -> bool {
    if id.pid <= 1 || !alive(id) {
        return false;
    }
    // `kill` (procps/util-linux) rather than an unsafe libc call; the pid
    // was just verified by uid and start time.
    crate::SpawnReaped::spawn_reaped(
        std::process::Command::new("kill")
            .arg("-TERM")
            .arg(id.pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // A real `/proc/<pid>/stat` line, with an awkward comm.
    const STAT: &str = "4242 (we ird) (x)) S 1 4242 4242 0 -1 4194560 2398 0 0 0 7 3 0 0 20 0 1 0 987654 12345678 1500 18446744073709551615 1 1 0 0 0 0 0 4096 0 0 0 0 17 3 0 0 0 0 0";

    #[test]
    fn start_time_is_field_22_even_with_parens_in_comm() {
        assert_eq!(parse_start_ticks(STAT), Some(987_654));
        assert_eq!(parse_start_ticks("1 (init) S"), None);
        assert_eq!(parse_start_ticks(""), None);
    }

    #[test]
    fn uid_and_unit_parse_from_status_and_cgroup() {
        assert_eq!(
            parse_uid("Name:\tx\nUid:\t1000\t1000\t1000\t1000\n"),
            Some(1000)
        );
        assert_eq!(parse_uid("Name:\tx\n"), None);
        let cg = "1:net_cls:/\n0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-mshell-scrcpy-77-3.scope\n";
        assert_eq!(
            parse_unit(cg).as_deref(),
            Some("app-mshell-scrcpy-77-3.scope")
        );
        assert_eq!(
            parse_unit(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/mshell.service\n"
            )
            .as_deref(),
            Some("mshell.service")
        );
        assert_eq!(parse_unit("0::/\n"), None);
        assert_eq!(parse_unit(""), None);
    }

    /// Only the exact scope names `outliving_shell` creates qualify.
    #[test]
    fn scope_label_is_exact() {
        assert_eq!(
            mshell_scope_label("app-mshell-scrcpy-77-3.scope"),
            Some("scrcpy")
        );
        assert_eq!(
            mshell_scope_label("app-mshell-claude-resume-x-42-3.scope"),
            Some("claude-resume-x")
        );
        let unit = crate::unit_name("rnnoise", 42, 0) + ".scope";
        assert_eq!(mshell_scope_label(&unit), Some("rnnoise"));
        for foreign in [
            "app-mshell-scrcpy.scope",
            "app-mshell-scrcpy-77.scope",
            "app-mshell-scrcpy-x-3.scope",
            "app-mshell--77-3.scope",
            "app-mshell-scrcpy-77-3.service",
            "app-gnome-scrcpy-77-3.scope",
            "app-org.genymobile.scrcpy-1234.scope",
            "tmux-spawn-459a7e06.scope",
        ] {
            assert_eq!(mshell_scope_label(foreign), None, "{foreign}");
        }
    }

    /// A fake procfs: one dir per pid with stat / status / cmdline / cgroup.
    struct FakeProc(tempfile::TempDir);

    impl FakeProc {
        fn new() -> Self {
            Self(tempfile::tempdir().expect("tempdir"))
        }

        fn root(&self) -> PathBuf {
            self.0.path().to_path_buf()
        }

        fn add(&self, pid: u32, uid: u32, start: u64, argv: &[&str], unit: &str) {
            let dir = self.0.path().join(pid.to_string());
            std::fs::create_dir_all(&dir).expect("mkdir");
            let stat =
                format!("{pid} (x) S 1 {pid} {pid} 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {start} 1 1");
            std::fs::write(dir.join("stat"), stat).expect("stat");
            std::fs::write(
                dir.join("status"),
                format!("Name:\tx\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
            )
            .expect("status");
            let mut cmdline = argv.join("\0");
            cmdline.push('\0');
            std::fs::write(dir.join("cmdline"), cmdline).expect("cmdline");
            std::fs::write(
                dir.join("cgroup"),
                format!("0::/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/{unit}\n"),
            )
            .expect("cgroup");
        }

        fn remove(&self, pid: u32) {
            std::fs::remove_dir_all(self.0.path().join(pid.to_string())).expect("rm");
        }
    }

    fn is_scrcpy(argv: &[OsString]) -> bool {
        argv.first()
            .is_some_and(|a| Path::new(a).file_name() == Some("scrcpy".as_ref()))
    }

    #[test]
    fn finds_only_our_uid_in_our_exact_scope_with_the_right_argv() {
        let p = FakeProc::new();
        p.add(
            10,
            1000,
            500,
            &["scrcpy", "--serial=A"],
            "app-mshell-scrcpy-9-0.scope",
        );
        // Someone else's scrcpy in an identically named scope.
        p.add(
            11,
            1001,
            501,
            &["scrcpy", "--serial=B"],
            "app-mshell-scrcpy-9-1.scope",
        );
        // The user's own scrcpy from a terminal: no mshell scope.
        p.add(
            12,
            1000,
            502,
            &["scrcpy", "--serial=C"],
            "tmux-spawn-1.scope",
        );
        // Our scope, but scrcpy's adb helper.
        p.add(
            13,
            1000,
            503,
            &["adb", "-s", "A", "forward"],
            "app-mshell-scrcpy-9-0.scope",
        );
        // Another mshell scope label.
        p.add(
            14,
            1000,
            504,
            &["scrcpy", "--serial=D"],
            "app-mshell-rnnoise-9-2.scope",
        );
        std::fs::create_dir_all(p.root().join("self")).expect("non-pid dir");
        let found = find_in(&p.root(), 1000, "scrcpy", is_scrcpy);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, 10);
        assert_eq!(
            found[0].argv,
            vec![OsString::from("scrcpy"), "--serial=A".into()]
        );
        assert_eq!(
            found[0].identity(),
            Identity {
                pid: 10,
                start_ticks: 500
            }
        );
    }

    #[test]
    fn identity_dies_with_the_process_and_never_matches_a_reused_pid() {
        let p = FakeProc::new();
        p.add(10, 1000, 500, &["scrcpy"], "app-mshell-scrcpy-9-0.scope");
        let id = Identity {
            pid: 10,
            start_ticks: 500,
        };
        assert!(alive_in(&p.root(), id, 1000));
        assert!(!alive_in(&p.root(), id, 1001), "other uid");
        p.remove(10);
        assert!(!alive_in(&p.root(), id, 1000), "gone");
        // Same pid, later start time: a different process.
        p.add(10, 1000, 900, &["scrcpy"], "app-mshell-scrcpy-9-0.scope");
        assert!(!alive_in(&p.root(), id, 1000), "reused pid");
    }

    /// Against the real procfs: a child of ours is ours by uid and start
    /// time, and stops being alive once it exited and was reaped.
    #[test]
    fn real_child_identity_round_trip() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        // `spawn` can return before the child's cmdline shows `sleep`. With
        // posix_spawn (CLONE_VFORK) the parent wakes in `exec_mmap`, just
        // *before* the child switches to its new mm, so a read then still
        // sees the shared parent argv (this test binary); with fork + the
        // close-on-exec pipe it wakes before the new argv is mapped (empty
        // cmdline). Only `sleep` itself proves the exec finished. Start
        // ticks never change across exec, so production, which records
        // them right after spawn, is not affected.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let info = loop {
            let info = read_in(Path::new(PROC), pid).expect("readable");
            if info.program_name() == Some("sleep".as_ref()) || std::time::Instant::now() > deadline
            {
                break info;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(Some(info.uid), own_uid());
        assert_eq!(info.program_name(), Some("sleep".as_ref()));
        let id = info.identity();
        assert!(alive(id));
        assert!(!alive(Identity {
            pid,
            start_ticks: id.start_ticks + 1
        }));
        child.kill().expect("kill");
        child.wait().expect("wait");
        assert!(!alive(id));
        assert!(!terminate(id), "never signals a dead identity");
    }
}
