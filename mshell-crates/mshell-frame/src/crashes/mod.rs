//! Crash watcher core — systemd-coredump crashes of the user's own
//! processes, plus this boot's journal errors. Port of the noctalia
//! `crashes` plugin (minus its AI-agent diagnosis), used by the Crashes
//! menu and the crash-notification watcher.
//!
//! * `coredumpctl list --json=short` → [`parse_coredumps`] → [`own_crashes`]
//!   (newest first, only our uid: a system daemon dumping core is a
//!   sysadmin's problem, not a desktop notification).
//! * `journalctl -b -p <level> -o json` → [`parse_journal`].
//! * The watcher ([`ensure_watcher`]) stats `/var/lib/systemd/coredump`
//!   every few seconds and only runs `coredumpctl` when the directory
//!   changed. New crashes since the persisted high-water mark raise a
//!   critical notification unless the program is muted
//!   (`crashes.muted`). The first run only records the mark, so starting
//!   the shell never replays old crashes.
//!
//! Parsers are pure and tested against output captured on the owner's
//! machine (`testdata/`). Missing `coredumpctl` / `journalctl` degrade to
//! "not available" in the menu and an idle watcher.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, CrashesConfigStoreFields};
use reactive_graph::prelude::GetUntracked;
use serde_json::Value;

/// Where systemd-coredump stores dumps (world-readable directory).
const COREDUMP_DIR: &str = "/var/lib/systemd/coredump";
/// How often the watcher stats [`COREDUMP_DIR`].
const WATCH_TICK: Duration = Duration::from_secs(5);
/// Rows kept from `coredumpctl` (newest) — the menu shows these.
pub(crate) const MAX_CRASHES: usize = 50;
/// Journal entries fetched for the Errors tab.
pub(crate) const MAX_JOURNAL: usize = 80;

/// One core dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Crash {
    /// Microseconds since the epoch.
    pub(crate) time_us: u64,
    pub(crate) pid: u32,
    pub(crate) uid: u32,
    pub(crate) sig: i32,
    pub(crate) exe: String,
    /// The core file still exists (not rotated away).
    pub(crate) core_present: bool,
}

impl Crash {
    /// The program name: the executable's file name.
    pub(crate) fn program(&self) -> &str {
        self.exe.rsplit('/').next().unwrap_or(&self.exe)
    }
}

/// `SIGSEGV`-style name for a signal number (number as text if unknown).
pub(crate) fn signal_name(sig: i32) -> String {
    let name = match sig {
        3 => "SIGQUIT",
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        11 => "SIGSEGV",
        24 => "SIGXCPU",
        25 => "SIGXFSZ",
        31 => "SIGSYS",
        _ => return format!("signal {sig}"),
    };
    name.to_string()
}

/// Parse `coredumpctl list --json=short` output. Rows missing a time or
/// an executable are skipped; garbage gives an empty list.
pub(crate) fn parse_coredumps(json: &str) -> Vec<Crash> {
    let Ok(Value::Array(rows)) = serde_json::from_str::<Value>(json.trim()) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|r| {
            let num = |k: &str| r.get(k).and_then(Value::as_u64);
            let exe = r.get("exe").and_then(Value::as_str)?.to_string();
            Some(Crash {
                time_us: num("time")?,
                pid: num("pid").unwrap_or(0) as u32,
                uid: num("uid").map(|u| u as u32).unwrap_or(u32::MAX),
                sig: r.get("sig").and_then(Value::as_i64).unwrap_or(0) as i32,
                exe,
                core_present: r.get("corefile").and_then(Value::as_str) == Some("present"),
            })
        })
        .collect()
}

/// Only `uid`'s crashes, newest first, capped at [`MAX_CRASHES`].
pub(crate) fn own_crashes(mut all: Vec<Crash>, uid: u32) -> Vec<Crash> {
    all.retain(|c| c.uid == uid);
    all.sort_by_key(|c| std::cmp::Reverse(c.time_us));
    all.truncate(MAX_CRASHES);
    all
}

/// Whether `program` is on the mute list (case-insensitive, exact name).
pub(crate) fn is_muted(muted: &[String], program: &str) -> bool {
    muted.iter().any(|m| m.trim().eq_ignore_ascii_case(program))
}

/// What the watcher should announce for a fresh crash list.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Announce<'a> {
    /// New, unmuted crashes (newest first).
    pub(crate) crashes: Vec<&'a Crash>,
    /// The new high-water mark to persist.
    pub(crate) seen_us: u64,
}

/// Decide what to announce. `seen_us == None` (first run, nothing
/// persisted) announces nothing and just records the newest time.
pub(crate) fn to_announce<'a>(
    own: &'a [Crash],
    seen_us: Option<u64>,
    muted: &[String],
) -> Announce<'a> {
    let newest = own.iter().map(|c| c.time_us).max();
    let Some(seen) = seen_us else {
        return Announce {
            crashes: Vec::new(),
            seen_us: newest.unwrap_or(0),
        };
    };
    Announce {
        crashes: own
            .iter()
            .filter(|c| c.time_us > seen && !is_muted(muted, c.program()))
            .collect(),
        seen_us: newest.map_or(seen, |n| n.max(seen)),
    }
}

/// One journal entry for the Errors tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JournalEntry {
    pub(crate) time_us: u64,
    /// syslog priority 0 (emerg) … 7 (debug).
    pub(crate) priority: u8,
    /// `SYSLOG_IDENTIFIER`, else `_COMM`, else "?".
    pub(crate) ident: String,
    pub(crate) message: String,
}

/// Journal priority names, most severe first (journalctl `-p` values).
pub(crate) const PRIORITIES: &[&str] = &["emerg", "alert", "crit", "err", "warning"];

/// Normalise a configured priority to a journalctl `-p` value.
pub(crate) fn journal_priority(cfg: &str) -> &'static str {
    let want = cfg.trim().to_ascii_lowercase();
    PRIORITIES
        .iter()
        .copied()
        .find(|p| *p == want)
        .unwrap_or("err")
}

/// Parse `journalctl -o json` (one object per line), newest first.
/// `MESSAGE` may be a byte array (journald does that for non-UTF-8 or
/// control characters): decoded lossily with control bytes dropped.
pub(crate) fn parse_journal(text: &str) -> Vec<JournalEntry> {
    let mut out: Vec<JournalEntry> = text
        .lines()
        .filter_map(|line| {
            let v: Value = serde_json::from_str(line).ok()?;
            let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
            let message = match v.get("MESSAGE")? {
                Value::String(m) => m.clone(),
                Value::Array(bytes) => {
                    let raw: Vec<u8> = bytes
                        .iter()
                        .filter_map(|b| b.as_u64().map(|b| b as u8))
                        .collect();
                    String::from_utf8_lossy(&raw).to_string()
                }
                _ => return None,
            };
            let message = strip_control(&message);
            Some(JournalEntry {
                time_us: s("__REALTIME_TIMESTAMP")?.parse().ok()?,
                priority: s("PRIORITY").and_then(|p| p.parse().ok()).unwrap_or(3),
                ident: s("SYSLOG_IDENTIFIER")
                    .or_else(|| s("_COMM"))
                    .unwrap_or_else(|| "?".to_string()),
                message,
            })
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.time_us));
    out
}

/// Drop control characters and ANSI colour sequences from a log line.
fn strip_control(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // ESC [ … final-byte (CSI); skip the whole sequence.
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            continue;
        }
        if c == '\t' || c == '\n' {
            out.push(' ');
        } else if !c.is_control() {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Our uid (owner of `/proc/self`), `None` if /proc is unreadable.
pub(crate) fn my_uid() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// Result of a fetch: the tool may be missing or fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fetch<T> {
    Ok(T),
    /// The binary isn't installed.
    Missing(&'static str),
    Failed(String),
}

/// The user's crashes, newest first.
pub(crate) async fn fetch_crashes() -> Fetch<Vec<Crash>> {
    let out = tokio::process::Command::new("coredumpctl")
        .args(["list", "--json=short", "--no-pager"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await;
    match out {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Fetch::Missing("coredumpctl"),
        Err(e) => Fetch::Failed(e.to_string()),
        // `coredumpctl list` exits 1 with no output when there are no dumps.
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            let uid = my_uid().unwrap_or(u32::MAX);
            Fetch::Ok(own_crashes(parse_coredumps(&text), uid))
        }
    }
}

/// This boot's journal entries at `priority` and above, newest first.
pub(crate) async fn fetch_journal(priority: &str) -> Fetch<Vec<JournalEntry>> {
    let n = MAX_JOURNAL.to_string();
    let out = tokio::process::Command::new("journalctl")
        .args(["-b", "-p", priority, "-n", &n, "-o", "json", "--no-pager"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await;
    match out {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Fetch::Missing("journalctl"),
        Err(e) => Fetch::Failed(e.to_string()),
        Ok(o) => Fetch::Ok(parse_journal(&String::from_utf8_lossy(&o.stdout))),
    }
}

/// Local "Mon 4 Nov 14:07" for a microsecond epoch time.
pub(crate) fn format_time(time_us: u64) -> String {
    relm4::gtk::glib::DateTime::from_unix_local((time_us / 1_000_000) as i64)
        .and_then(|d| d.format("%a %e %b %H:%M"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

// ── Watcher ─────────────────────────────────────────────────────────────

static WATCHER: AtomicBool = AtomicBool::new(false);

fn seen_path() -> PathBuf {
    relm4::gtk::glib::user_cache_dir()
        .join("mshell")
        .join("crashes-seen")
}

fn read_seen() -> Option<u64> {
    std::fs::read_to_string(seen_path())
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn write_seen(us: u64) {
    let path = seen_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, us.to_string()) {
        tracing::warn!("crashes: cannot write {}: {e}", path.display());
    }
}

fn notify_enabled() -> bool {
    config_manager().config().crashes().notify().get_untracked()
}

fn muted() -> Vec<String> {
    config_manager().config().crashes().muted().get_untracked()
}

/// Start the crash-notification watcher once (idempotent). It idles
/// (one `stat` per tick) while `crashes.notify` is off.
pub(crate) fn ensure_watcher() {
    if WATCHER.swap(true, Ordering::SeqCst) {
        return;
    }
    relm4::spawn_local(async move {
        let mut last_mtime: Option<SystemTime> = None;
        loop {
            if notify_enabled() {
                let mtime = std::fs::metadata(COREDUMP_DIR)
                    .and_then(|m| m.modified())
                    .ok();
                if mtime.is_some() && mtime != last_mtime {
                    last_mtime = mtime;
                    // tokio::process needs the tokio runtime: run the fetch
                    // there, keep config + state on the main loop.
                    if let Ok(Fetch::Ok(own)) = relm4::spawn(fetch_crashes()).await {
                        let seen = read_seen();
                        let muted = muted();
                        let a = to_announce(&own, seen, &muted);
                        for c in &a.crashes {
                            announce(c);
                        }
                        if Some(a.seen_us) != seen {
                            write_seen(a.seen_us);
                        }
                    }
                }
            } else {
                // Re-arm so turning it back on compares against the
                // persisted mark (crashes while off are announced then).
                last_mtime = None;
            }
            relm4::gtk::glib::timeout_future(WATCH_TICK).await;
        }
    });
}

/// Raise a critical desktop notification for one crash.
fn announce(c: &Crash) {
    let summary = format!("{} crashed", c.program());
    let body = format!(
        "{} · PID {} · {}{}",
        signal_name(c.sig),
        c.pid,
        format_time(c.time_us),
        if c.core_present {
            ""
        } else {
            " · no core file"
        }
    );
    relm4::spawn_blocking(move || {
        let _ = notify_rust::Notification::new()
            .appname("Crashes")
            .summary(&summary)
            .body(&body)
            .icon("dialog-error-symbolic")
            .urgency(notify_rust::Urgency::Critical)
            .show();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const COREDUMPS: &str = include_str!("testdata/coredumpctl.json");
    const JOURNAL: &str = include_str!("testdata/journal.json");

    #[test]
    fn parses_real_coredumpctl_output() {
        let all = parse_coredumps(COREDUMPS);
        assert_eq!(all.len(), 7);
        let lld = all.iter().find(|c| c.sig == 7).unwrap();
        assert_eq!(lld.program(), "rust-lld");
        assert_eq!(lld.pid, 1642539);
        assert_eq!(lld.uid, 1000);
        assert!(lld.core_present);
        assert_eq!(signal_name(lld.sig), "SIGBUS");
        assert_eq!(all[0].program(), "python3.14");
        assert_eq!(signal_name(6), "SIGABRT");
        assert_eq!(signal_name(99), "signal 99");
    }

    #[test]
    fn coredump_garbage_and_empty_output_give_no_crashes() {
        // `coredumpctl list` prints nothing (exit 1) when there are no dumps.
        assert!(parse_coredumps("").is_empty());
        assert!(parse_coredumps("No coredumps found.").is_empty());
        assert!(parse_coredumps("{}").is_empty());
        // A row without an exe or time is skipped, the rest kept.
        let v = parse_coredumps(
            r#"[{"time":1,"pid":2,"uid":0,"sig":11,"corefile":"missing"},
                {"time":5,"pid":3,"uid":0,"sig":11,"corefile":"missing","exe":"/bin/x"}]"#,
        );
        assert_eq!(v.len(), 1);
        assert!(!v[0].core_present);
    }

    #[test]
    fn own_crashes_filters_uid_and_sorts_newest_first() {
        let mut all = parse_coredumps(COREDUMPS);
        all.push(Crash {
            time_us: u64::MAX,
            pid: 1,
            uid: 0,
            sig: 11,
            exe: "/usr/lib/systemd/systemd-udevd".into(),
            core_present: true,
        });
        let own = own_crashes(all, 1000);
        assert_eq!(own.len(), 7, "root's crash is not ours");
        assert!(own.windows(2).all(|w| w[0].time_us >= w[1].time_us));
        assert_eq!(own[0].time_us, 1793790480247274);
    }

    #[test]
    fn first_run_records_the_mark_without_announcing() {
        let own = own_crashes(parse_coredumps(COREDUMPS), 1000);
        let a = to_announce(&own, None, &[]);
        assert!(a.crashes.is_empty());
        assert_eq!(a.seen_us, 1793790480247274);
        // Nothing at all: mark 0.
        assert_eq!(to_announce(&[], None, &[]).seen_us, 0);
    }

    #[test]
    fn announces_only_newer_unmuted_crashes() {
        let own = own_crashes(parse_coredumps(COREDUMPS), 1000);
        // Mark just before the rust-lld crash: two newer (lld + python).
        let a = to_announce(&own, Some(1793786217258203), &[]);
        let names: Vec<_> = a.crashes.iter().map(|c| c.program()).collect();
        assert_eq!(names, vec!["python3.14", "rust-lld"]);
        assert_eq!(a.seen_us, 1793790480247274);
        // Muting python (any case) leaves only rust-lld.
        let a = to_announce(&own, Some(1793786217258203), &["Python3.14".into()]);
        let names: Vec<_> = a.crashes.iter().map(|c| c.program()).collect();
        assert_eq!(names, vec!["rust-lld"]);
        // Already seen everything: nothing, mark unchanged.
        let a = to_announce(&own, Some(1793790480247274), &[]);
        assert!(a.crashes.is_empty());
        assert_eq!(a.seen_us, 1793790480247274);
        // A dump rotated away (list shrank) never lowers the mark.
        assert_eq!(to_announce(&[], Some(42), &[]).seen_us, 42);
    }

    #[test]
    fn parses_real_journal_output_newest_first() {
        let j = parse_journal(JOURNAL);
        assert_eq!(j.len(), 6);
        // The byte-array MESSAGE line is the newest; ANSI colour stripped.
        assert_eq!(j[0].ident, "weird");
        assert_eq!(j[0].priority, 2);
        assert_eq!(j[0].message, "bad colour");
        let mlogind = j.iter().find(|e| e.ident == "mlogind").unwrap();
        assert_eq!(
            mlogind.message,
            "gkr-pam: unable to locate daemon control file"
        );
        assert_eq!(mlogind.priority, 3);
        assert!(j.windows(2).all(|w| w[0].time_us >= w[1].time_us));
    }

    #[test]
    fn journal_tolerates_garbage_lines_and_maps_priorities() {
        let j = parse_journal("not json\n{\"MESSAGE\":\"x\"}\n\n");
        assert!(j.is_empty(), "no timestamp → skipped");
        assert_eq!(journal_priority("warning"), "warning");
        assert_eq!(journal_priority(" CRIT "), "crit");
        assert_eq!(journal_priority("bogus"), "err");
    }

    #[test]
    fn mute_matches_program_name_exactly() {
        let m = vec!["firefox".to_string()];
        assert!(is_muted(&m, "Firefox"));
        assert!(!is_muted(&m, "firefox-bin"));
        assert!(!is_muted(&[], "x"));
    }
}
