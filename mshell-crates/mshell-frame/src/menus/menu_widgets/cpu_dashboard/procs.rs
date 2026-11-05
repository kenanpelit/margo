//! Process table data for the CPU dashboard's Processes tab: `/proc`
//! parsers, the per-poll CPU / disk-IO delta tracker, sort, search and
//! the "group by app window" fold. All pure (or rooted at a caller-given
//! `/proc` path) so it is unit-tested against real captured samples in
//! `testdata/`; the GTK side lives in `processes_tab.rs`.

use std::collections::HashMap;
use std::path::Path;

/// `PF_KTHREAD` in `/proc/<pid>/stat` field 9 (`flags`): kernel threads
/// have no user-space memory or command line and can't be usefully killed,
/// so the table hides them.
const PF_KTHREAD: u64 = 0x0020_0000;

/// The fields the table needs from `/proc/<pid>/stat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatFields {
    pub pid: i32,
    pub comm: String,
    pub ppid: i32,
    pub kthread: bool,
    /// `utime + stime` in clock ticks.
    pub ticks: u64,
    /// Field 22: start time after boot (ticks). Pairs with the pid so a
    /// recycled pid never inherits the previous process's counters.
    pub start: u64,
}

/// Parse one `/proc/<pid>/stat` line. `comm` is the parenthesised field 2
/// and may itself contain spaces and parentheses (`(tmux: client)`,
/// `((sd-pam))`), so the split anchors on the *last* `)`.
pub(crate) fn parse_stat(line: &str) -> Option<StatFields> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    if close <= open {
        return None;
    }
    let pid = line[..open].trim().parse().ok()?;
    let comm = line[open + 1..close].to_string();
    // Fields after comm, numbered from 3 (state) onwards.
    let rest: Vec<&str> = line[close + 1..].split_whitespace().collect();
    let field = |n: usize| rest.get(n - 3).copied();
    let num = |n: usize| field(n).and_then(|v| v.parse::<u64>().ok());
    let ppid = field(4)?.parse().ok()?;
    let flags = num(9)?;
    let ticks = num(14)? + num(15)?;
    let start = num(22)?;
    Some(StatFields {
        pid,
        comm,
        ppid,
        kthread: flags & PF_KTHREAD != 0,
        ticks,
        start,
    })
}

/// `(uid, VmRSS kB, VmSwap kB)` from `/proc/<pid>/status`. Memory lines are
/// absent for zombies; they read as 0.
pub(crate) fn parse_status(s: &str) -> (u32, u64, u64) {
    let mut uid = u32::MAX;
    let mut rss = 0;
    let mut swap = 0;
    for line in s.lines() {
        let first = |rest: &str| rest.split_whitespace().next().and_then(|v| v.parse().ok());
        if let Some(r) = line.strip_prefix("Uid:") {
            uid = first(r).map(|v: u64| v as u32).unwrap_or(u32::MAX);
        } else if let Some(r) = line.strip_prefix("VmRSS:") {
            rss = first(r).unwrap_or(0);
        } else if let Some(r) = line.strip_prefix("VmSwap:") {
            swap = first(r).unwrap_or(0);
        }
    }
    (uid, rss, swap)
}

/// `(read_bytes, write_bytes)` — real storage I/O, not `rchar`/`wchar`
/// (those count page-cache hits and pipes). Only readable for processes
/// we may ptrace (our own), so this is `None` for the rest.
pub(crate) fn parse_io(s: &str) -> Option<(u64, u64)> {
    let mut read = None;
    let mut write = None;
    for line in s.lines() {
        if let Some(r) = line.strip_prefix("read_bytes:") {
            read = r.trim().parse().ok();
        } else if let Some(r) = line.strip_prefix("write_bytes:") {
            write = r.trim().parse().ok();
        }
    }
    Some((read?, write?))
}

/// NUL-separated `/proc/<pid>/cmdline` → a space-joined display string.
pub(crate) fn parse_cmdline(raw: &[u8]) -> String {
    raw.split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Sum of all fields of the aggregate `cpu` line of `/proc/stat` (jiffies
/// across all CPUs).
pub(crate) fn parse_total_jiffies(proc_stat: &str) -> Option<u64> {
    let line = proc_stat.lines().find(|l| l.starts_with("cpu "))?;
    Some(
        line.split_whitespace()
            .skip(1)
            .filter_map(|x| x.parse::<u64>().ok())
            .sum(),
    )
}

/// One process as read from `/proc` this poll (raw counters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcSample {
    pub stat: StatFields,
    pub uid: u32,
    pub cmdline: String,
    pub rss_kb: u64,
    pub swap_kb: u64,
    pub io: Option<(u64, u64)>,
}

/// One `/proc` sweep: every user-space process plus the CPU-time base.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProcSnapshot {
    pub procs: Vec<ProcSample>,
    pub total_jiffies: u64,
    pub ncpu: usize,
}

/// Read every `<root>/<pid>` (`root` = `/proc` in production, a temp tree
/// in tests). Processes that exit mid-sweep are skipped silently. Blocking:
/// call it off the GTK thread.
pub(crate) fn read_snapshot(root: &Path) -> ProcSnapshot {
    let stat = std::fs::read_to_string(root.join("stat")).unwrap_or_default();
    let total_jiffies = parse_total_jiffies(&stat).unwrap_or(0);
    let ncpu = stat
        .lines()
        .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
        .count()
        .max(1);
    let mut procs = Vec::new();
    let Ok(dir) = std::fs::read_dir(root) else {
        return ProcSnapshot {
            procs,
            total_jiffies,
            ncpu,
        };
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let dir = entry.path();
        let Some(stat) = std::fs::read_to_string(dir.join("stat"))
            .ok()
            .and_then(|s| parse_stat(&s))
        else {
            continue;
        };
        if stat.kthread {
            continue;
        }
        let (uid, rss_kb, swap_kb) = std::fs::read_to_string(dir.join("status"))
            .map(|s| parse_status(&s))
            .unwrap_or((u32::MAX, 0, 0));
        let cmdline = std::fs::read(dir.join("cmdline"))
            .map(|b| parse_cmdline(&b))
            .unwrap_or_default();
        let io = std::fs::read_to_string(dir.join("io"))
            .ok()
            .and_then(|s| parse_io(&s));
        procs.push(ProcSample {
            stat,
            uid,
            cmdline,
            rss_kb,
            swap_kb,
            io,
        });
    }
    ProcSnapshot {
        procs,
        total_jiffies,
        ncpu,
    }
}

/// A display row: rates computed against the previous sweep.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProcRow {
    pub pid: i32,
    pub ppid: i32,
    pub uid: u32,
    pub name: String,
    pub cmdline: String,
    /// Percent of ONE CPU (top/htop convention: a busy 4-thread process
    /// reads 400%).
    pub cpu: f32,
    pub rss_kb: u64,
    pub swap_kb: u64,
    /// Bytes/s read + written; `None` when `/proc/<pid>/io` is unreadable.
    pub io_bps: Option<(u64, u64)>,
    /// `/proc/<pid>/stat` start time: with the pid, names this process
    /// and no later one that reuses the pid (see [`send_signal`]).
    pub start: u64,
}

impl ProcRow {
    /// Combined read + write rate, 0 when unknown (sort key).
    pub(crate) fn io_total(&self) -> u64 {
        self.io_bps.map(|(r, w)| r + w).unwrap_or(0)
    }
}

/// Keeps the previous sweep's counters to turn them into rates.
#[derive(Debug, Default)]
pub(crate) struct ProcTracker {
    prev: HashMap<(i32, u64), (u64, Option<(u64, u64)>)>,
    prev_total: u64,
}

impl ProcTracker {
    /// Rows for `snap`; `elapsed_secs` is the wall time since the previous
    /// call. The first call (no baseline) reports 0% CPU and no IO rate.
    pub(crate) fn update(&mut self, snap: &ProcSnapshot, elapsed_secs: f64) -> Vec<ProcRow> {
        let d_total = snap.total_jiffies.saturating_sub(self.prev_total);
        let mut next = HashMap::with_capacity(snap.procs.len());
        let rows = snap
            .procs
            .iter()
            .map(|p| {
                let key = (p.stat.pid, p.stat.start);
                let prev = self.prev.get(&key);
                let cpu = match prev {
                    Some((ticks, _)) if d_total > 0 && self.prev_total > 0 => {
                        let d = p.stat.ticks.saturating_sub(*ticks);
                        (d as f64 / d_total as f64 * snap.ncpu as f64 * 100.0) as f32
                    }
                    _ => 0.0,
                };
                let io_bps = match (prev.and_then(|(_, io)| *io), p.io) {
                    (Some((r0, w0)), Some((r1, w1))) if elapsed_secs > 0.0 => Some((
                        (r1.saturating_sub(r0) as f64 / elapsed_secs) as u64,
                        (w1.saturating_sub(w0) as f64 / elapsed_secs) as u64,
                    )),
                    (_, Some(_)) => Some((0, 0)),
                    _ => None,
                };
                next.insert(key, (p.stat.ticks, p.io));
                ProcRow {
                    pid: p.stat.pid,
                    ppid: p.stat.ppid,
                    uid: p.uid,
                    name: p.stat.comm.clone(),
                    cmdline: p.cmdline.clone(),
                    cpu,
                    rss_kb: p.rss_kb,
                    swap_kb: p.swap_kb,
                    io_bps,
                    start: p.stat.start,
                }
            })
            .collect();
        self.prev = next;
        self.prev_total = snap.total_jiffies;
        rows
    }
}

/// Sort column of the process table (always descending — "what's eating
/// my machine" is the question).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ProcSort {
    #[default]
    Cpu,
    Mem,
    Swap,
    Io,
}

fn cmp_by(sort: ProcSort, a: &ProcRow, b: &ProcRow) -> std::cmp::Ordering {
    let primary = match sort {
        ProcSort::Cpu => b.cpu.total_cmp(&a.cpu),
        ProcSort::Mem => b.rss_kb.cmp(&a.rss_kb),
        ProcSort::Swap => b.swap_kb.cmp(&a.swap_kb),
        ProcSort::Io => b.io_total().cmp(&a.io_total()),
    };
    // Stable tiebreak so equal rows don't shuffle between polls.
    primary.then(a.pid.cmp(&b.pid))
}

pub(crate) fn sort_rows(rows: &mut [ProcRow], sort: ProcSort) {
    rows.sort_by(|a, b| cmp_by(sort, a, b));
}

/// Search: case-insensitive substring of the name or command line, or a
/// pid prefix when the query is all digits. Empty query matches all.
pub(crate) fn row_matches(row: &ProcRow, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    if q.bytes().all(|b| b.is_ascii_digit()) && row.pid.to_string().starts_with(q) {
        return true;
    }
    let q = q.to_lowercase();
    row.name.to_lowercase().contains(&q) || row.cmdline.to_lowercase().contains(&q)
}

/// A group of processes in the "by window" view.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProcGroup {
    /// The window's app id, or `None` for the "Other processes" bucket.
    pub app: Option<String>,
    /// Indices into the rows slice, in the slice's (sorted) order.
    pub members: Vec<usize>,
    pub cpu: f32,
    pub rss_kb: u64,
    pub swap_kb: u64,
    pub io: u64,
}

/// Walk each row's parent chain until it hits a pid that owns a compositor
/// window (`clients`: `(pid, app_id)` from margo). The process belongs to
/// that window's app (a browser's renderers, a terminal's shell and its
/// children…). Rows with no window ancestor land in the trailing
/// `app: None` group. Groups sort by the summed `sort` key; several windows
/// of one app fold into one group. Cycles and missing parents end the walk.
pub(crate) fn group_by_window(
    rows: &[ProcRow],
    clients: &[(i32, String)],
    sort: ProcSort,
) -> Vec<ProcGroup> {
    let parent: HashMap<i32, i32> = rows.iter().map(|r| (r.pid, r.ppid)).collect();
    let window: HashMap<i32, &str> = clients.iter().map(|(p, a)| (*p, a.as_str())).collect();
    let mut order: Vec<Option<String>> = Vec::new();
    let mut groups: HashMap<Option<String>, ProcGroup> = HashMap::new();
    for (i, row) in rows.iter().enumerate() {
        let mut pid = row.pid;
        let mut app = None;
        for _ in 0..64 {
            if let Some(a) = window.get(&pid) {
                app = Some((*a).to_string());
                break;
            }
            match parent.get(&pid) {
                Some(pp) if *pp > 1 && *pp != pid => pid = *pp,
                _ => break,
            }
        }
        let g = groups.entry(app.clone()).or_insert_with(|| {
            order.push(app.clone());
            ProcGroup {
                app,
                members: Vec::new(),
                cpu: 0.0,
                rss_kb: 0,
                swap_kb: 0,
                io: 0,
            }
        });
        g.members.push(i);
        g.cpu += row.cpu;
        g.rss_kb += row.rss_kb;
        g.swap_kb += row.swap_kb;
        g.io += row.io_total();
    }
    let mut out: Vec<ProcGroup> = order
        .into_iter()
        .filter_map(|k| groups.remove(&k))
        .collect();
    out.sort_by(|a, b| {
        // "Other processes" always last.
        a.app.is_none().cmp(&b.app.is_none()).then_with(|| {
            match sort {
                ProcSort::Cpu => b.cpu.total_cmp(&a.cpu),
                ProcSort::Mem => b.rss_kb.cmp(&a.rss_kb),
                ProcSort::Swap => b.swap_kb.cmp(&a.swap_kb),
                ProcSort::Io => b.io.cmp(&a.io),
            }
            .then_with(|| a.app.cmp(&b.app))
        })
    });
    out
}

/// Why a kill request is refused before any signal is sent.
pub(crate) fn kill_guard(pid: i32, self_pid: u32) -> Result<(), &'static str> {
    if pid <= 1 {
        Err("Refusing to signal init or an invalid pid")
    } else if pid as u32 == self_pid {
        Err("Refusing to signal mshell itself")
    } else {
        Ok(())
    }
}

/// Outcome of sending a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KillResult {
    Sent,
    /// EPERM: owned by another user — needs the pkexec path.
    NotPermitted,
    /// ESRCH: already gone.
    Gone,
    Failed(i32),
}

/// Map a `kill(2)` return / errno pair to the outcome the UI shows.
pub(crate) fn classify_kill(ret: i32, errno: i32) -> KillResult {
    if ret == 0 {
        KillResult::Sent
    } else if errno == libc::EPERM {
        KillResult::NotPermitted
    } else if errno == libc::ESRCH {
        KillResult::Gone
    } else {
        KillResult::Failed(errno)
    }
}

/// Start time (`/proc/<pid>/stat` field 22) of `pid` under `proc_root`;
/// `None` once it has exited.
pub(crate) fn proc_start(proc_root: &Path, pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(proc_root.join(pid.to_string()).join("stat")).ok()?;
    parse_stat(&stat).map(|s| s.start)
}

/// Signal the process the user picked: `pid` started at `start` (the
/// table's snapshot). Seconds pass between the snapshot and the confirm,
/// and the pid may have been reused by then, so the signal goes through a
/// pidfd that is checked to still be that process; a reused pid reads as
/// [`KillResult::Gone`]. Kernels without pidfds (< 5.3) check the start
/// time, then `kill(2)`.
pub(crate) fn send_signal(pid: i32, start: u64, sig: i32) -> Result<KillResult, &'static str> {
    kill_guard(pid, std::process::id())?;
    let proc_root = Path::new("/proc");
    let errno = || std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    // SAFETY: plain syscall with integer arguments; no memory is shared.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        let e = errno();
        if e != libc::ENOSYS {
            return Ok(classify_kill(-1, e));
        }
        if proc_start(proc_root, pid) != Some(start) {
            return Ok(KillResult::Gone);
        }
        // SAFETY: as above.
        let ret = unsafe { libc::kill(pid, sig) };
        return Ok(classify_kill(ret, if ret == 0 { 0 } else { errno() }));
    }
    let Ok(fd) = i32::try_from(fd) else {
        return Ok(KillResult::Failed(libc::EBADF));
    };
    // SAFETY: `fd` was just returned by pidfd_open and is owned here only.
    let fd = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    // The pidfd pins one process: if it is still the one that started at
    // `start`, the signal can't reach a newcomer that reuses the pid.
    if proc_start(proc_root, pid) != Some(start) {
        return Ok(KillResult::Gone);
    }
    let raw = std::os::fd::AsRawFd::as_raw_fd(&fd);
    // SAFETY: valid pidfd, null siginfo (as kill(2)), no flags.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            raw,
            sig,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    let ret = if ret == 0 { 0 } else { -1 };
    Ok(classify_kill(ret, if ret == 0 { 0 } else { errno() }))
}

/// Human byte size from kB: `512 kB`, `12.3 MB`, `1.4 GB`.
pub(crate) fn fmt_kb(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.1} GB", kb as f64 / (1024.0 * 1024.0))
    } else if kb >= 1024 {
        format!("{:.0} MB", kb as f64 / 1024.0)
    } else {
        format!("{kb} kB")
    }
}

/// Human byte rate: `0 B/s`, `12 kB/s`, `3.4 MB/s`.
pub(crate) fn fmt_rate(bps: u64) -> String {
    if bps >= 1024 * 1024 {
        format!("{:.1} MB/s", bps as f64 / (1024.0 * 1024.0))
    } else if bps >= 1024 {
        format!("{:.0} kB/s", bps as f64 / 1024.0)
    } else {
        format!("{bps} B/s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = include_str!("testdata/proc_pid_stat.txt");
    const STAT_ODD: &str = include_str!("testdata/proc_stat_odd.txt");
    const STATUS: &str = include_str!("testdata/proc_pid_status.txt");
    const IO: &str = include_str!("testdata/proc_pid_io.txt");
    const CPU_LINE: &str = include_str!("testdata/proc_stat_cpu.txt");

    #[test]
    fn parses_real_mshell_stat() {
        let s = parse_stat(STAT).expect("stat parses");
        assert_eq!(s.pid, 2199);
        assert_eq!(s.comm, "mshell");
        assert_eq!(s.ppid, 1678);
        assert!(!s.kthread);
        assert_eq!(s.ticks, 3625 + 2057);
        assert_eq!(s.start, 2409);
    }

    #[test]
    fn comm_with_spaces_and_parens_and_kthreads() {
        let lines: Vec<_> = STAT_ODD
            .lines()
            .map(|l| parse_stat(l).expect("parses"))
            .collect();
        assert_eq!(lines[0].comm, "tmux: client");
        assert_eq!(lines[0].ppid, 4028);
        assert_eq!(lines[1].comm, "(sd-pam)");
        assert_eq!(lines[1].ppid, 1678);
        assert!(!lines[0].kthread && !lines[1].kthread);
        assert_eq!(lines[2].comm, "kthreadd");
        assert!(lines[2].kthread, "kthreadd carries PF_KTHREAD");
        assert!(parse_stat("garbage").is_none());
        assert!(parse_stat("12 (x) S 1").is_none(), "truncated line");
    }

    #[test]
    fn parses_real_status_and_io() {
        assert_eq!(parse_status(STATUS), (1000, 222_496, 0));
        assert_eq!(parse_io(IO), Some((165_388_288, 103_669_760)));
        assert_eq!(parse_io(""), None);
        // A zombie has no Vm* lines.
        assert_eq!(parse_status("Name:\tz\nUid:\t0\t0\t0\t0\n"), (0, 0, 0));
    }

    #[test]
    fn cmdline_and_total_jiffies() {
        assert_eq!(
            parse_cmdline(b"/usr/bin/mshell\0--flag\0\0"),
            "/usr/bin/mshell --flag"
        );
        assert_eq!(parse_cmdline(b""), "");
        assert_eq!(
            parse_total_jiffies(CPU_LINE),
            Some(169168 + 167690 + 78222 + 3475413 + 9984 + 13590 + 6128)
        );
    }

    fn sample(pid: i32, ppid: i32, ticks: u64, io: Option<(u64, u64)>) -> ProcSample {
        ProcSample {
            stat: StatFields {
                pid,
                comm: format!("p{pid}"),
                ppid,
                kthread: false,
                ticks,
                start: 100,
            },
            uid: 1000,
            cmdline: String::new(),
            rss_kb: pid as u64 * 10,
            swap_kb: 0,
            io,
        }
    }

    #[test]
    fn tracker_turns_counters_into_rates() {
        let mut t = ProcTracker::default();
        let s0 = ProcSnapshot {
            procs: vec![sample(10, 1, 100, Some((0, 0))), sample(11, 1, 0, None)],
            total_jiffies: 1000,
            ncpu: 4,
        };
        let r0 = t.update(&s0, 0.0);
        assert_eq!(r0[0].cpu, 0.0, "no baseline yet");
        assert_eq!(r0[0].io_bps, Some((0, 0)));
        assert_eq!(r0[1].io_bps, None, "io unreadable");
        // 400 jiffies of machine time over 4 CPUs = 100 ticks per CPU; the
        // process used 100 ticks → one full CPU = 100%.
        let s1 = ProcSnapshot {
            procs: vec![
                sample(10, 1, 200, Some((4096, 2048))),
                sample(11, 1, 50, None),
            ],
            total_jiffies: 1400,
            ncpu: 4,
        };
        let r1 = t.update(&s1, 2.0);
        assert!((r1[0].cpu - 100.0).abs() < 0.01, "{}", r1[0].cpu);
        assert!((r1[1].cpu - 50.0).abs() < 0.01);
        assert_eq!(r1[0].io_bps, Some((2048, 1024)));
    }

    #[test]
    fn recycled_pid_does_not_inherit_counters() {
        let mut t = ProcTracker::default();
        let s0 = ProcSnapshot {
            procs: vec![sample(10, 1, 5000, None)],
            total_jiffies: 1000,
            ncpu: 1,
        };
        t.update(&s0, 0.0);
        let mut fresh = sample(10, 1, 10, None);
        fresh.stat.start = 999;
        let s1 = ProcSnapshot {
            procs: vec![fresh],
            total_jiffies: 1100,
            ncpu: 1,
        };
        assert_eq!(t.update(&s1, 2.0)[0].cpu, 0.0);
    }

    fn row(pid: i32, ppid: i32, cpu: f32, rss: u64) -> ProcRow {
        ProcRow {
            pid,
            ppid,
            uid: 1000,
            name: format!("p{pid}"),
            cmdline: format!("/usr/bin/p{pid} --x"),
            cpu,
            rss_kb: rss,
            swap_kb: rss / 2,
            io_bps: None,
            start: 0,
        }
    }

    /// A signal names a process by pid *and* start time: a pid that now
    /// belongs to another process (reused) is never signalled.
    #[test]
    fn signal_checks_the_start_time() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = i32::try_from(child.id()).expect("pid");
        let start = proc_start(Path::new("/proc"), pid).expect("start time");
        assert_eq!(
            send_signal(pid, start + 1, libc::SIGTERM),
            Ok(KillResult::Gone),
            "another process's start time: refused"
        );
        assert!(child.try_wait().expect("wait").is_none(), "still running");
        assert_eq!(send_signal(pid, start, libc::SIGTERM), Ok(KillResult::Sent));
        let status = child.wait().expect("wait");
        assert!(!status.success(), "terminated by the signal");
        assert_eq!(proc_start(Path::new("/proc"), pid), None);
    }

    #[test]
    fn sort_and_search() {
        let mut rows = vec![row(3, 1, 1.0, 900), row(2, 1, 5.0, 100), row(4, 1, 5.0, 50)];
        sort_rows(&mut rows, ProcSort::Cpu);
        assert_eq!(rows.iter().map(|r| r.pid).collect::<Vec<_>>(), [2, 4, 3]);
        sort_rows(&mut rows, ProcSort::Mem);
        assert_eq!(rows[0].pid, 3);
        sort_rows(&mut rows, ProcSort::Swap);
        assert_eq!(rows[0].pid, 3);
        assert!(row_matches(&rows[0], ""));
        assert!(row_matches(&row(1234, 1, 0.0, 0), "12"));
        assert!(row_matches(&row(7, 1, 0.0, 0), "P7"));
        assert!(row_matches(&row(7, 1, 0.0, 0), "--X"));
        assert!(!row_matches(&row(7, 1, 0.0, 0), "firefox"));
    }

    #[test]
    fn groups_descendants_under_their_window() {
        // kitty(100, window) → zsh(101) → cargo(102); chrome(200, window)
        // → renderer(201); a second kitty window (300) folds into "kitty";
        // daemon(400) has no window ancestor.
        let rows = vec![
            row(100, 1, 1.0, 10),
            row(101, 100, 0.5, 5),
            row(102, 101, 50.0, 500),
            row(200, 1, 2.0, 100),
            row(201, 200, 3.0, 200),
            row(300, 1, 0.5, 10),
            row(400, 1, 9.0, 1),
        ];
        let clients = vec![
            (100, "kitty".to_string()),
            (200, "chrome".to_string()),
            (300, "kitty".to_string()),
        ];
        let g = group_by_window(&rows, &clients, ProcSort::Cpu);
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].app.as_deref(), Some("kitty"));
        assert_eq!(g[0].members, [0, 1, 2, 5]);
        assert!((g[0].cpu - 52.0).abs() < 0.01);
        assert_eq!(g[1].app.as_deref(), Some("chrome"));
        assert_eq!(g[1].members, [3, 4]);
        assert_eq!(g[2].app, None, "unattached processes come last");
        assert_eq!(g[2].members, [6]);
        let by_mem = group_by_window(&rows, &clients, ProcSort::Mem);
        assert_eq!(by_mem[0].app.as_deref(), Some("kitty"));
        assert_eq!(by_mem[0].rss_kb, 525);
    }

    #[test]
    fn grouping_survives_parent_cycles() {
        let rows = vec![row(5, 6, 0.0, 0), row(6, 5, 0.0, 0)];
        let g = group_by_window(&rows, &[], ProcSort::Cpu);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].app, None);
    }

    #[test]
    fn kill_guard_and_errno_mapping() {
        assert!(kill_guard(1, 500).is_err());
        assert!(kill_guard(0, 500).is_err());
        assert!(kill_guard(-3, 500).is_err());
        assert!(kill_guard(500, 500).is_err());
        assert!(kill_guard(501, 500).is_ok());
        assert_eq!(classify_kill(0, 0), KillResult::Sent);
        assert_eq!(classify_kill(-1, libc::EPERM), KillResult::NotPermitted);
        assert_eq!(classify_kill(-1, libc::ESRCH), KillResult::Gone);
        assert_eq!(
            classify_kill(-1, libc::EINVAL),
            KillResult::Failed(libc::EINVAL)
        );
    }

    #[test]
    fn reads_a_proc_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(
            root.join("stat"),
            format!("{CPU_LINE}cpu0 1 2 3 4\ncpu1 1 2 3 4\n"),
        )
        .expect("write");
        let p = root.join("2199");
        std::fs::create_dir(&p).expect("mkdir");
        std::fs::write(p.join("stat"), STAT).expect("write");
        std::fs::write(p.join("status"), STATUS).expect("write");
        std::fs::write(p.join("io"), IO).expect("write");
        std::fs::write(p.join("cmdline"), b"/usr/bin/mshell\0").expect("write");
        let k = root.join("2");
        std::fs::create_dir(&k).expect("mkdir");
        std::fs::write(k.join("stat"), STAT_ODD.lines().nth(2).unwrap_or_default()).expect("write");
        std::fs::create_dir(root.join("self")).expect("mkdir");
        let snap = read_snapshot(root);
        assert_eq!(snap.ncpu, 2);
        assert_eq!(snap.procs.len(), 1, "kthread and non-pid dirs skipped");
        assert_eq!(snap.procs[0].cmdline, "/usr/bin/mshell");
        assert_eq!(snap.procs[0].rss_kb, 222_496);
        assert_eq!(snap.procs[0].io, Some((165_388_288, 103_669_760)));
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_kb(512), "512 kB");
        assert_eq!(fmt_kb(222_496), "217 MB");
        assert_eq!(fmt_kb(3 * 1024 * 1024), "3.0 GB");
        assert_eq!(fmt_rate(0), "0 B/s");
        assert_eq!(fmt_rate(4096), "4 kB/s");
        assert_eq!(fmt_rate(3 * 1024 * 1024), "3.0 MB/s");
    }
}
