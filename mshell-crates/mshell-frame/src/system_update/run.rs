//! Background upgrade: the whole upgrade runs detached, escalating
//! through polkit (`pkexec`), with every byte of output in a log file
//! the panel tails for a live progress bar + last lines. When the run
//! ends a desktop notification reports the result and the pill
//! re-checks. The interactive terminal mode
//! ([`super::launch_terminal_upgrade`]) stays the default.
//!
//! The run is a `sh -c` in its own transient systemd scope (outside
//! mshell.service's cgroup, so restarting the shell doesn't kill it)
//! writing to `~/.cache/mshell/system_update/upgrade.log`, framed by
//! `::START <ts>` / `::EXIT <code>` markers. `upgrade.run` records the
//! pid + start stamp until the result is reported, so a restarted shell
//! re-attaches to an unfinished run, and reports one that ended or died
//! meanwhile, instead of losing it. Only one watcher task exists per
//! process (multi-monitor setups build one panel per output), so the
//! result is notified once.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::watch;
use tracing::warn;

/// What the background upgrade runs (resolved from PATH + config).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UpgradePlan {
    /// Run `pacman -Syu` (repo or AUR source enabled, pacman present).
    /// AUR packages are never built here: see [`aur_review_step`].
    pub(crate) system: bool,
    /// Also `flatpak update`.
    pub(crate) flatpak: bool,
}

/// One step of a background run. `name` tags its `::STEP` result
/// line and decides how the result reads ([`step_label`],
/// [`step_escalates`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) name: &'static str,
    pub(crate) cmd: String,
}

impl Step {
    pub(crate) fn new(name: &'static str, cmd: impl Into<String>) -> Self {
        Self {
            name,
            cmd: cmd.into(),
        }
    }
}

/// Join steps so each runs even when an earlier one failed (Flatpak
/// isn't held back by a failed pacman step), logging each step's own
/// status as `::STEP <name> <code>` (the result is told per step) and
/// still exiting with the last failing step's status.
pub(crate) fn chain_steps(steps: &[Step]) -> String {
    if steps.is_empty() {
        return "true".to_string();
    }
    let mut out = vec!["rc=0".to_string()];
    for s in steps {
        out.push(format!(
            "{{ {} ; }}; s=$?; echo \"::STEP {} $s\"; [ \"$s\" -eq 0 ] || rc=$s",
            s.cmd, s.name
        ));
    }
    out.push("(exit \"$rc\")".to_string());
    out.join("; ")
}

/// Steps whose command goes through pkexec: 126 / 127 there are the
/// polkit dialog dismissed / not authorized, not the command's own.
pub(crate) fn step_escalates(name: &str) -> bool {
    matches!(name, "pacman" | "rollback")
}

/// What the user calls a step in the result.
pub(crate) fn step_label(name: &str) -> &str {
    match name {
        "pacman" => "Official repos",
        "flatpak" => "Flatpak",
        "rollback" => "Rollback",
        other => other,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepOutcome {
    Done,
    Cancelled,
    NotAuthorized,
    NotFound,
    Failed(i32),
}

fn step_outcome(name: &str, code: i32) -> StepOutcome {
    match code {
        0 => StepOutcome::Done,
        126 if step_escalates(name) => StepOutcome::Cancelled,
        127 if step_escalates(name) => StepOutcome::NotAuthorized,
        127 => StepOutcome::NotFound,
        c => StepOutcome::Failed(c),
    }
}

fn outcome_text(o: StepOutcome) -> String {
    match o {
        StepOutcome::Done => "updated".into(),
        StepOutcome::Cancelled => "cancelled (authentication dismissed)".into(),
        StepOutcome::NotAuthorized => "not authorized".into(),
        StepOutcome::NotFound => "command not found".into(),
        StepOutcome::Failed(c) => format!("failed (exit {c})"),
    }
}

/// The non-interactive background command: pkexec raises the polkit
/// dialog for the helper's `upgrade` verb, which runs
/// `pacman -Syu --noconfirm --noprogressbar --color never` (every
/// default answer taken). Official repos
/// and Flatpak only: an AUR helper would build and install, as root,
/// a PKGBUILD nobody reviewed (`--skipreview` / `--answerdiff None`),
/// so pending AUR updates are left to [`aur_review_step`] in a
/// terminal. `IgnorePkg` in pacman.conf is honoured by pacman itself;
/// packages hidden in the panel are only hidden from the count
/// (holding a package back is a partial upgrade, which Arch doesn't
/// support).
pub(crate) fn background_steps(plan: &UpgradePlan) -> Vec<Step> {
    let mut steps = Vec::new();
    if plan.system {
        // The helper's fixed `upgrade` verb runs `pacman -Syu{quiet}`
        // (its own polkit action and message).
        steps.push(Step::new(
            "pacman",
            mshell_privileged::Request::Upgrade.pkexec_command(),
        ));
    }
    if plan.flatpak {
        steps.push(Step::new("flatpak", "flatpak update -y --noninteractive"));
    }
    steps
}

/// The interactive AUR-only upgrade for the panel's "Review in
/// terminal": the helper shows every PKGBUILD diff and asks before it
/// builds.
pub(crate) fn aur_review_step(helper: &str) -> String {
    format!("{helper} -Sua")
}

/// The full detached script: START marker, the chained steps, EXIT
/// marker with the chain's status.
pub(crate) fn background_script(steps: &[Step], started_at: u64) -> String {
    format!(
        "echo '::START {started_at}'; {}; echo \"::EXIT $?\"",
        chain_steps(steps)
    )
}

/// Snapshot of the log, as the panel renders it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LogState {
    /// Unix seconds from the `::START` marker.
    pub(crate) started: Option<u64>,
    /// `Some(code)` once `::EXIT` is written.
    pub(crate) exit: Option<i32>,
    /// Packages / refs processed so far.
    pub(crate) done: usize,
    /// The last non-empty lines, ANSI-stripped.
    pub(crate) tail: Vec<String>,
    /// `(step name, exit code)` from each `::STEP` line, in order.
    pub(crate) steps: Vec<(String, i32)>,
}

/// Drop ANSI escapes and keep only what a terminal would finally show
/// of a `\r`-redrawn line.
pub(crate) fn clean_line(line: &str) -> String {
    let last = line
        .trim_end_matches('\r')
        .rsplit('\r')
        .next()
        .unwrap_or("");
    let mut out = String::with_capacity(last.len());
    let mut chars = last.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // CSI: parameters until a final byte in @..~
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// One package (pacman / AUR helper) or one Flatpak ref processed.
fn is_progress_line(line: &str) -> bool {
    let l = line.trim_start();
    // pacman's optional `( 3/12) ` counter prefix.
    let l = match l.strip_prefix('(') {
        Some(rest) => match rest.split_once(") ") {
            Some((counter, tail)) if counter.contains('/') => tail,
            _ => l,
        },
        None => l,
    };
    ["upgrading ", "installing ", "reinstalling ", "downgrading "]
        .iter()
        .any(|p| l.starts_with(p))
        || l.starts_with("Updating app/")
        || l.starts_with("Updating runtime/")
        || l.starts_with("Installing app/")
        || l.starts_with("Installing runtime/")
}

/// The whole log at once.
#[cfg(test)]
pub(crate) fn parse_log(text: &str, keep: usize) -> LogState {
    let mut t = LogTail::new(keep);
    t.feed(text.as_bytes());
    t.finish()
}

/// Incremental log parser: fed the bytes appended since the last read,
/// it keeps the markers, the progress count and the last `keep` lines,
/// so a growing log is parsed once in total, not once per tick.
#[derive(Debug, Clone)]
pub(crate) struct LogTail {
    keep: usize,
    /// Bytes after the last `\n` (a line still being written).
    partial: Vec<u8>,
    started: Option<u64>,
    exit: Option<i32>,
    done: usize,
    steps: Vec<(String, i32)>,
    lines: std::collections::VecDeque<String>,
}

impl LogTail {
    pub(crate) fn new(keep: usize) -> Self {
        Self {
            keep,
            partial: Vec::new(),
            started: None,
            exit: None,
            done: 0,
            steps: Vec::new(),
            lines: std::collections::VecDeque::new(),
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        self.partial.extend_from_slice(bytes);
        let Some(last_nl) = self.partial.iter().rposition(|b| *b == b'\n') else {
            return;
        };
        let rest = self.partial.split_off(last_nl + 1);
        let complete = std::mem::replace(&mut self.partial, rest);
        for raw in complete[..last_nl].split(|b| *b == b'\n') {
            self.line(&String::from_utf8_lossy(raw));
        }
    }

    fn line(&mut self, raw: &str) {
        let line = clean_line(raw);
        if let Some(ts) = line.strip_prefix("::START ") {
            self.started = ts.trim().parse().ok();
            return;
        }
        if let Some(code) = line.strip_prefix("::EXIT ") {
            self.exit = code.trim().parse().ok().or(Some(-1));
            return;
        }
        if let Some((name, code)) = line
            .strip_prefix("::STEP ")
            .and_then(|r| r.trim().split_once(' '))
        {
            self.steps
                .push((name.to_string(), code.trim().parse().unwrap_or(-1)));
            return;
        }
        if is_progress_line(&line) {
            self.done += 1;
        }
        if !line.trim().is_empty() {
            self.lines.push_back(line);
            while self.lines.len() > self.keep {
                self.lines.pop_front();
            }
        }
    }

    /// The state so far; a line still being written shows in the tail
    /// but isn't counted until it is complete.
    pub(crate) fn snapshot(&self) -> LogState {
        let mut tail: Vec<String> = self.lines.iter().cloned().collect();
        let partial = clean_line(&String::from_utf8_lossy(&self.partial));
        if !partial.trim().is_empty() {
            tail.push(partial);
            let skip = tail.len().saturating_sub(self.keep);
            tail.drain(..skip);
        }
        LogState {
            started: self.started,
            exit: self.exit,
            done: self.done,
            tail,
            steps: self.steps.clone(),
        }
    }

    /// The final state: an unterminated last line counts as a line.
    pub(crate) fn finish(mut self) -> LogState {
        if !self.partial.is_empty() {
            let raw = std::mem::take(&mut self.partial);
            self.line(&String::from_utf8_lossy(&raw));
        }
        self.snapshot()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadNew {
    /// The bytes after `offset` (possibly none).
    Appended(Vec<u8>),
    /// The file is shorter than what was already read: a new log.
    Truncated,
}

/// Read what was appended to `path` since `offset`, advancing it.
pub(crate) fn read_new(path: &std::path::Path, offset: &mut u64) -> std::io::Result<ReadNew> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    if f.metadata()?.len() < *offset {
        *offset = 0;
        return Ok(ReadNew::Truncated);
    }
    f.seek(SeekFrom::Start(*offset))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    *offset += buf.len() as u64;
    Ok(ReadNew::Appended(buf))
}

/// Progress 0..=1 for the bar (`expected` = pending count at start).
pub(crate) fn fraction(done: usize, expected: usize) -> f64 {
    if expected == 0 {
        return 0.0;
    }
    (done as f64 / expected as f64).min(1.0)
}

/// Notification text for a finished run.
pub(crate) fn result_message(exit: i32, done: usize) -> (String, String) {
    match exit {
        0 => (
            "System updated".to_string(),
            if done == 0 {
                "Nothing needed upgrading.".to_string()
            } else {
                format!("{done} package(s) upgraded.")
            },
        ),
        // pkexec: 126 = auth dialog dismissed, 127 = not authorized.
        126 | 127 => (
            "System update cancelled".to_string(),
            "Authentication was dismissed or refused.".to_string(),
        ),
        code => (
            "System update failed".to_string(),
            format!("Exit status {code}. Open the System Updates panel for the log."),
        ),
    }
}

/// Title + body for a finished run (`exit: None` = interrupted), told
/// per step: a dismissed prompt for pacman next to a good Flatpak step
/// is "partly done", not "cancelled".
pub(crate) fn finished_message(log: &LogState) -> (String, String) {
    match log.exit {
        // A log written before ::STEP lines existed.
        Some(code) if log.steps.is_empty() => result_message(code, log.done),
        Some(_) => steps_message(&log.steps, log.done),
        None => (
            "System update interrupted".to_string(),
            "The upgrade stopped before it finished (killed, or the machine went down). \
             Open the System Updates panel for the log."
                .to_string(),
        ),
    }
}

fn steps_message(steps: &[(String, i32)], done: usize) -> (String, String) {
    let outcomes: Vec<(&str, StepOutcome)> = steps
        .iter()
        .map(|(n, c)| (n.as_str(), step_outcome(n, *c)))
        .collect();
    let ok = outcomes
        .iter()
        .filter(|(_, o)| *o == StepOutcome::Done)
        .count();
    if ok == outcomes.len() {
        return result_message(0, done);
    }
    let per_step = || {
        outcomes
            .iter()
            .map(|(n, o)| format!("{}: {}", step_label(n), outcome_text(*o)))
            .collect::<Vec<_>>()
            .join(". ")
    };
    if ok > 0 {
        return (
            "System update partly done".to_string(),
            format!("{}.", per_step()),
        );
    }
    let refused =
        |o: &StepOutcome| matches!(o, StepOutcome::Cancelled | StepOutcome::NotAuthorized);
    if outcomes.iter().all(|(_, o)| refused(o)) {
        let body = if outcomes.iter().any(|(_, o)| *o == StepOutcome::Cancelled) {
            "Authentication was dismissed."
        } else {
            "You are not authorized to run the upgrade (polkit)."
        };
        return ("System update cancelled".to_string(), body.to_string());
    }
    (
        "System update failed".to_string(),
        format!("{}. Open the System Updates panel for the log.", per_step()),
    )
}

// ── Run state (one watcher per process) ──────────────────────────

/// A recorded background run: the shell's pid and the `::START` stamp
/// of its script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunHandle {
    pub(crate) pid: u32,
    pub(crate) started_at: u64,
    pub(crate) scope: bool,
}
impl RunHandle {
    fn unit(self) -> String {
        format!("mshell-upgrade-{}.scope", self.started_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunStatus {
    Idle,
    Running { log: LogState, expected: usize },
    Finished { log: LogState, expected: usize },
}

fn channel() -> &'static (watch::Sender<RunStatus>, watch::Receiver<RunStatus>) {
    static CH: OnceLock<(watch::Sender<RunStatus>, watch::Receiver<RunStatus>)> = OnceLock::new();
    CH.get_or_init(|| watch::channel(RunStatus::Idle))
}

/// Subscribe to run status changes (panel + pill).
pub(crate) fn subscribe() -> watch::Receiver<RunStatus> {
    channel().1.clone()
}

pub(crate) fn status() -> RunStatus {
    channel().1.borrow().clone()
}

fn run_dir() -> PathBuf {
    margo_fs::xdg::cache_home()
        .join("mshell")
        .join("system_update")
}

pub(crate) fn log_path() -> PathBuf {
    run_dir().join("upgrade.log")
}

/// `<pid> <started_at>` of the run not yet reported to the user. It
/// is removed once the result has been shown + notified, so a run that
/// ended (or died) while the shell was down is still reported.
fn handle_path() -> PathBuf {
    run_dir().join("upgrade.run")
}

/// The pid-only file older shells wrote (no start stamp, so a recycled
/// pid looked like a live run forever).
fn legacy_pid_path() -> PathBuf {
    run_dir().join("upgrade.pid")
}

fn expected_path() -> PathBuf {
    run_dir().join("upgrade.expected")
}

pub(crate) fn parse_handle(s: &str) -> Option<RunHandle> {
    let mut it = s.split_whitespace();
    let pid = it.next()?.parse().ok()?;
    let started_at = it.next()?.parse().ok()?;
    let scope = match it.next() {
        Some("scope") => true,
        Some("process") | None => false,
        _ => return None,
    };
    it.next().is_none().then_some(RunHandle {
        pid,
        started_at,
        scope,
    })
}

pub(crate) fn format_handle(h: RunHandle) -> String {
    format!(
        "{} {} {}",
        h.pid,
        h.started_at,
        if h.scope { "scope" } else { "process" }
    )
}

fn read_handle() -> Option<RunHandle> {
    parse_handle(&std::fs::read_to_string(handle_path()).ok()?)
}

/// `/proc/<pid>/cmdline` belongs to the run that started at
/// `started_at`: the script carries its own `::START <ts>` marker, so a
/// pid the kernel handed to another process never matches.
pub(crate) fn cmdline_is_ours(cmdline: &[u8], started_at: u64) -> bool {
    let marker = format!("'::START {started_at}'");
    cmdline
        .windows(marker.len())
        .any(|w| w == marker.as_bytes())
}

fn run_alive(h: RunHandle) -> bool {
    std::fs::read(format!("/proc/{}/cmdline", h.pid))
        .is_ok_and(|c| cmdline_is_ours(&c, h.started_at))
}

/// A systemd run is tracked by its independent unit, never a PID.
/// The PID/cmdline identity is only used on machines without a user manager.
async fn tracked_alive(h: RunHandle) -> bool {
    if !h.scope {
        return run_alive(h);
    }
    tokio::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", &h.unit()])
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Where a recorded run stands, from its log and whether its shell is
/// still alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunPhase {
    Running,
    /// `::EXIT <code>` was written.
    Exited(i32),
    /// The process is gone without an `::EXIT` (killed, crashed, the
    /// machine went down).
    Interrupted,
}

pub(crate) fn run_phase(log: &LogState, alive: bool) -> RunPhase {
    match (log.exit, alive) {
        (Some(c), _) => RunPhase::Exited(c),
        (None, true) => RunPhase::Running,
        (None, false) => RunPhase::Interrupted,
    }
}

/// The command that runs the detached script. With a systemd user
/// manager it goes into its own transient scope: `systemd-run --scope`
/// execs the script in place (same pid) in a cgroup of its own, so
/// stopping or restarting mshell.service (KillMode=control-group)
/// doesn't take pacman down with it. A process group alone doesn't
/// leave the cgroup.
pub(crate) fn upgrade_argv(script: &str, unit: &str, scope: bool) -> Vec<String> {
    let mut argv: Vec<String> = Vec::new();
    if scope {
        argv.extend(["systemd-run", "--user", "--scope", "--collect", "--quiet"].map(String::from));
        argv.push(format!("--unit={unit}"));
        argv.push("--".into());
    }
    argv.extend(["sh".to_string(), "-c".to_string(), script.to_string()]);
    argv
}

/// A systemd user manager we can ask for a scope.
fn user_manager_available() -> bool {
    margo_fs::path::on_path("systemd-run")
        && margo_fs::xdg::runtime_dir().is_some_and(|d| d.join("systemd/private").exists())
}

/// Start a detached background run. Refuses while one is running.
pub(crate) fn start(steps: Vec<Step>, expected: usize) -> Result<(), String> {
    if matches!(status(), RunStatus::Running { .. }) {
        return Err("an upgrade is already running".into());
    }
    if steps.is_empty() {
        return Err("nothing to upgrade (no backend / all sources off)".into());
    }
    std::fs::create_dir_all(run_dir()).map_err(|e| e.to_string())?;
    let log = std::fs::File::create(log_path()).map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let started_at = super::now_secs();
    let script = background_script(&steps, started_at);
    let scope = user_manager_available();
    let argv = upgrade_argv(&script, &format!("mshell-upgrade-{started_at}"), scope);
    use mshell_process::SpawnReaped;
    use std::os::unix::process::CommandExt;
    let pid = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(err)
        // A Ctrl-C in whatever launched mshell doesn't reach pacman.
        // (Surviving an mshell restart is the scope's job.)
        .process_group(0)
        .spawn_reaped()
        .map_err(|e| format!("spawn: {e}"))?;
    let _ = std::fs::write(
        handle_path(),
        format_handle(RunHandle {
            pid,
            started_at,
            scope,
        }),
    );
    let _ = std::fs::remove_file(legacy_pid_path());
    let _ = std::fs::write(expected_path(), expected.to_string());
    let _ = channel().0.send(RunStatus::Running {
        log: LogState::default(),
        expected,
    });
    ensure_watcher();
    wake().notify_one();
    Ok(())
}

/// The panel's "Dismiss" on a finished run.
pub(crate) fn dismiss() {
    if matches!(status(), RunStatus::Finished { .. }) {
        let _ = channel().0.send(RunStatus::Idle);
    }
}

/// Wakes the parked watcher when [`start`] records a run.
fn wake() -> &'static tokio::sync::Notify {
    static WAKE: OnceLock<tokio::sync::Notify> = OnceLock::new();
    WAKE.get_or_init(tokio::sync::Notify::new)
}

/// A run younger than this whose pid doesn't (yet) carry the script's
/// cmdline is still starting: spawn() can return before the child has
/// exec'd, and systemd-run talks to the manager before it execs sh.
const STARTUP_GRACE_SECS: u64 = 10;

/// Whether a run without `::EXIT` is still going: its process carries
/// the script's cmdline, or it was started only moments ago.
pub(crate) fn still_running(process_ours: bool, started_at: u64, now: u64) -> bool {
    process_ours || now.saturating_sub(started_at) < STARTUP_GRACE_SECS
}

/// Spawn the single watcher (idempotent). It checks once for a run
/// left by a previous shell (following it, or reporting one that ended
/// or died meanwhile), then parks until [`start`] wakes it: nothing is
/// read while no run is recorded. A followed run's log is read
/// incrementally, once a second.
pub(crate) fn ensure_watcher() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::fs::remove_file(legacy_pid_path());
    relm4::spawn(async move {
        let mut reported = None;
        loop {
            let h = idle_until_run(read_handle, reported, wake()).await;
            follow_run(h).await;
            reported = Some(h);
        }
    });
}

/// Wait for a recorded run other than `skip` (one already reported
/// whose record couldn't be removed): one check, then sleep on `wake`
/// until [`start`] records another.
pub(crate) async fn idle_until_run(
    read: impl Fn() -> Option<RunHandle>,
    skip: Option<RunHandle>,
    wake: &tokio::sync::Notify,
) -> RunHandle {
    loop {
        if let Some(h) = read().filter(|h| Some(*h) != skip) {
            return h;
        }
        wake.notified().await;
    }
}

/// Follow one run until it ends, then report it once (status,
/// notification, a re-check for every pill) and clear its record.
async fn follow_run(h: RunHandle) {
    let expected = std::fs::read_to_string(expected_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let mut tail = LogTail::new(14);
    let mut offset = 0;
    loop {
        // Liveness first, then read: a run that ends in between has
        // written all of its log by the time we read it.
        let alive = still_running(tracked_alive(h).await, h.started_at, super::now_secs());
        match read_new(&log_path(), &mut offset) {
            Ok(ReadNew::Appended(bytes)) => tail.feed(&bytes),
            Ok(ReadNew::Truncated) => {
                tail = LogTail::new(14);
                continue;
            }
            Err(e) => warn!(error = %e, "system_update: reading the upgrade log failed"),
        }
        let log = tail.snapshot();
        let phase = run_phase(&log, alive);
        if phase == RunPhase::Running {
            let next = RunStatus::Running { log, expected };
            if status() != next {
                let _ = channel().0.send(next);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let mut log = tail.finish();
        log.exit = match phase {
            RunPhase::Exited(c) => Some(c),
            _ => None,
        };
        let (title, body) = finished_message(&log);
        notify(&title, &body, log.exit == Some(0));
        let _ = std::fs::remove_file(handle_path());
        let _ = channel().0.send(RunStatus::Finished { log, expected });
        // One re-check for every pill/panel on every monitor.
        let report = super::probe_with_audit(super::ProbeConfig::from_config()).await;
        super::publish_report(report);
        return;
    }
}

fn notify(title: &str, body: &str, ok: bool) {
    use mshell_process::SpawnReaped;
    let icon = if ok {
        "software-update-available-symbolic"
    } else {
        "software-update-urgent-symbolic"
    };
    if let Err(e) = std::process::Command::new("notify-send")
        .args(["-a", "System Updates", "-i", icon, title, body])
        .spawn_reaped()
    {
        warn!(error = %e, "system_update: notify-send failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_lives_in_its_own_scope_outside_mshell_service() {
        // KillMode=control-group on mshell.service killed a run that only
        // had its own process group; a transient scope is its own cgroup.
        let argv = upgrade_argv("echo hi", "mshell-upgrade-42", true);
        assert_eq!(
            argv,
            [
                "systemd-run",
                "--user",
                "--scope",
                "--collect",
                "--quiet",
                "--unit=mshell-upgrade-42",
                "--",
                "sh",
                "-c",
                "echo hi"
            ]
        );
        assert_eq!(upgrade_argv("echo hi", "u", false), ["sh", "-c", "echo hi"]);
    }

    #[test]
    fn scope_identity_survives_pid_reuse() {
        let h = RunHandle {
            pid: 12,
            started_at: 42,
            scope: true,
        };
        assert_eq!(parse_handle(&format_handle(h)), Some(h));
        assert_eq!(h.unit(), "mshell-upgrade-42.scope");
    }

    #[test]
    fn a_recycled_pid_is_not_our_run() {
        let script = background_script(&[Step::new("pacman", "true")], 42);
        let mut ours = b"sh\0-c\0".to_vec();
        ours.extend_from_slice(script.as_bytes());
        ours.push(0);
        assert!(cmdline_is_ours(&ours, 42));
        // Same script text, an older run's start stamp.
        assert!(!cmdline_is_ours(&ours, 41));
        // The pid now belongs to something else entirely.
        assert!(!cmdline_is_ours(b"firefox\0--new-window\0", 42));
        assert!(!cmdline_is_ours(b"", 42));
    }

    #[test]
    fn handle_file_round_trips_and_ignores_the_old_pid_only_format() {
        let h = RunHandle {
            pid: 1234,
            started_at: 42,
            scope: false,
        };
        assert_eq!(parse_handle(&format_handle(h)), Some(h));
        assert_eq!(parse_handle("1234\n"), None);
        assert_eq!(parse_handle("junk"), None);
    }

    #[test]
    fn a_live_run_is_recognised_by_its_own_cmdline() {
        let script = background_script(&[Step::new("pacman", "sleep 5")], 77);
        let mut child = std::process::Command::new("sh")
            .args(["-c", &script])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let h = RunHandle {
            pid: child.id(),
            started_at: 77,
            scope: false,
        };
        // spawn() can return before the child has exec'd sh (its
        // cmdline is still ours then): give it a moment.
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while !run_alive(h) && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(run_alive(h));
        assert!(!run_alive(RunHandle {
            started_at: 76,
            ..h
        }));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!run_alive(h));
    }

    #[test]
    fn a_run_that_died_without_exit_is_interrupted() {
        let started = parse_log("::START 5\nupgrading a\n", 10);
        assert_eq!(run_phase(&started, true), RunPhase::Running);
        assert_eq!(run_phase(&started, false), RunPhase::Interrupted);
        let done = parse_log("::START 5\n::EXIT 0\n", 10);
        assert_eq!(run_phase(&done, false), RunPhase::Exited(0));
        // EXIT is the last thing the script prints; it wins even if
        // the shell is still being reaped.
        assert_eq!(run_phase(&done, true), RunPhase::Exited(0));
    }

    #[test]
    fn background_never_builds_aur_packages() {
        let cmds = |plan: &UpgradePlan| -> Vec<String> {
            background_steps(plan).into_iter().map(|s| s.cmd).collect()
        };
        // An unattended AUR build (paru --skipreview / yay --answerdiff
        // None) installs a PKGBUILD nobody looked at as root. Background
        // mode upgrades the repos (+ Flatpak) only, whatever helper is
        // installed; AUR updates go to the terminal review.
        let mut plan = UpgradePlan {
            system: true,
            flatpak: true,
        };
        // pacman runs as the helper's fixed `upgrade` verb (its own
        // polkit action), not a bare `pkexec pacman` with any argv.
        let upgrade = format!("pkexec '{}' 'upgrade'", mshell_privileged::HELPER);
        assert_eq!(
            cmds(&plan),
            [upgrade.as_str(), "flatpak update -y --noninteractive"]
        );
        assert_eq!(
            background_steps(&plan)
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>(),
            ["pacman", "flatpak"]
        );
        for s in cmds(&plan) {
            for bad in [
                "paru",
                "yay",
                "--skipreview",
                "--answerdiff",
                "--answeredit",
            ] {
                assert!(!s.contains(bad), "{s}");
            }
        }
        plan.flatpak = false;
        assert_eq!(cmds(&plan), [upgrade]);
        plan.system = false;
        assert!(background_steps(&plan).is_empty());
    }

    #[test]
    fn aur_review_runs_the_helper_interactively() {
        assert_eq!(aur_review_step("paru"), "paru -Sua");
        assert_eq!(aur_review_step("yay"), "yay -Sua");
        assert!(!aur_review_step("paru").contains("--noconfirm"));
    }

    fn run_script(steps: &[Step]) -> LogState {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(background_script(steps, 7))
            .output()
            .unwrap();
        parse_log(&String::from_utf8_lossy(&out.stdout), 10)
    }

    #[test]
    fn chained_script_really_exits_with_the_failing_status() {
        let log = run_script(&[
            Step::new("pacman", "false"),
            Step::new("flatpak", "echo ok"),
        ]);
        assert_eq!(log.started, Some(7));
        assert_eq!(log.exit, Some(1));
        assert_eq!(log.tail, ["ok"]);
        assert_eq!(
            log.steps,
            [("pacman".to_string(), 1), ("flatpak".to_string(), 0)]
        );
        assert_eq!(run_script(&[]).exit, Some(0));
        assert_eq!(
            run_script(&[Step::new("rollback", "(exit 3)")]).steps,
            [("rollback".to_string(), 3)]
        );
    }

    #[test]
    fn a_dismissed_prompt_with_a_good_flatpak_step_is_partly_done() {
        // pkexec exits 126 when the dialog is dismissed. The chain used
        // to carry only that code: "cancelled" although Flatpak updated.
        let log = run_script(&[
            Step::new("pacman", "(exit 126)"),
            Step::new("flatpak", "echo Updating app/org.x/x86_64/stable"),
        ]);
        let (title, body) = finished_message(&log);
        assert_eq!(title, "System update partly done");
        assert!(body.contains("Official repos: cancelled"), "{body}");
        assert!(body.contains("Flatpak: updated"), "{body}");
    }

    #[test]
    fn results_read_per_step() {
        let log = |steps: &[(&str, i32)], exit: i32| LogState {
            exit: Some(exit),
            done: 2,
            steps: steps.iter().map(|(n, c)| (n.to_string(), *c)).collect(),
            ..Default::default()
        };
        assert_eq!(
            finished_message(&log(&[("pacman", 0), ("flatpak", 0)], 0)).0,
            "System updated"
        );
        let (t, b) = finished_message(&log(&[("pacman", 126)], 126));
        assert_eq!(t, "System update cancelled");
        assert!(b.contains("dismissed"), "{b}");
        assert!(
            finished_message(&log(&[("pacman", 127)], 127))
                .1
                .contains("not authorized")
        );
        // 127 from a step without pkexec is sh's "command not found".
        let (t, b) = finished_message(&log(&[("flatpak", 127)], 127));
        assert_eq!(t, "System update failed");
        assert!(b.contains("not found"), "{b}");
        let (t, b) = finished_message(&log(&[("pacman", 1), ("flatpak", 0)], 1));
        assert_eq!(t, "System update partly done");
        assert!(b.contains("Official repos: failed (exit 1)"), "{b}");
        // A log from before ::STEP lines falls back to the exit code.
        assert_eq!(finished_message(&log(&[], 0)).0, "System updated");
        assert_eq!(
            finished_message(&log(&[], 126)).0,
            "System update cancelled"
        );
    }

    #[test]
    fn log_parsing_counts_packages_and_strips_ansi() {
        // pacman --noprogressbar / paru output shape, plus a pty-style
        // redrawn line and a flatpak ref.
        let log = "::START 100\n\
            :: Synchronizing package databases...\n\
            \x1b[1;34m::\x1b[0m Starting full system upgrade...\n\
            (1/2) upgrading electron43                 \n\
            \r 50%\r(2/2) upgrading firefox\n\
            installing foo...\n\
            Updating app/org.mozilla.firefox/x86_64/stable\n\
            \n\
            ::EXIT 0\n";
        let s = parse_log(log, 3);
        assert_eq!(s.started, Some(100));
        assert_eq!(s.exit, Some(0));
        assert_eq!(s.done, 4);
        assert_eq!(
            s.tail,
            [
                "(2/2) upgrading firefox",
                "installing foo...",
                "Updating app/org.mozilla.firefox/x86_64/stable"
            ]
        );
        assert_eq!(clean_line("\x1b[1;34m::\x1b[0m Starting"), ":: Starting");
    }

    #[test]
    fn the_log_parses_the_same_fed_in_any_chunks() {
        let log = "::START 100\n:: Synchronizing\n(1/3) upgrading a\n\
                   \r 50%\r(2/3) upgrading b\nUpdating app/x/y/z\n\
                   ünïcode line\n(3/3) upgrading c\n::EXIT 0\npartial";
        let whole = parse_log(log, 3);
        let bytes = log.as_bytes();
        for cut in 0..=bytes.len() {
            let mut t = LogTail::new(3);
            t.feed(&bytes[..cut]);
            t.feed(&bytes[cut..]);
            assert_eq!(t.finish(), whole, "split at {cut}");
        }
        // A line still being written shows in the live tail but is not
        // counted until it is complete.
        let mut t = LogTail::new(5);
        t.feed(b"::START 1\n(1/1) upgrad");
        let snap = t.snapshot();
        assert_eq!(snap.done, 0);
        assert_eq!(snap.tail, ["(1/1) upgrad"]);
        t.feed(b"ing a\n");
        assert_eq!(t.snapshot().done, 1);
    }

    #[test]
    fn only_the_new_bytes_are_read() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("upgrade.log");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(b"one\n").unwrap();
        let mut offset = 0;
        assert_eq!(
            read_new(&p, &mut offset).unwrap(),
            ReadNew::Appended(b"one\n".to_vec())
        );
        assert_eq!(
            read_new(&p, &mut offset).unwrap(),
            ReadNew::Appended(Vec::new())
        );
        f.write_all(b"two\n").unwrap();
        assert_eq!(
            read_new(&p, &mut offset).unwrap(),
            ReadNew::Appended(b"two\n".to_vec())
        );
        assert_eq!(offset, 8);
        std::fs::write(&p, b"x").unwrap();
        assert_eq!(read_new(&p, &mut offset).unwrap(), ReadNew::Truncated);
    }

    #[test]
    fn a_run_still_starting_is_not_interrupted() {
        // Right after spawn the pid can still show mshell's cmdline.
        assert!(still_running(false, 100, 101));
        assert!(!still_running(false, 100, 100 + STARTUP_GRACE_SECS));
        assert!(still_running(true, 100, 10_000));
    }

    #[tokio::test]
    async fn an_idle_watcher_parks_instead_of_polling() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let reads = Arc::new(AtomicUsize::new(0));
        let handle = Arc::new(std::sync::Mutex::new(None::<RunHandle>));
        let wake = Arc::new(tokio::sync::Notify::new());
        let task = {
            let (reads, handle, wake) = (reads.clone(), handle.clone(), wake.clone());
            tokio::spawn(async move {
                idle_until_run(
                    || {
                        reads.fetch_add(1, Ordering::SeqCst);
                        *handle.lock().unwrap()
                    },
                    None,
                    &wake,
                )
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(reads.load(Ordering::SeqCst), 1, "one check, then parked");
        let h = RunHandle {
            pid: 1,
            started_at: 2,
            scope: false,
        };
        *handle.lock().unwrap() = Some(h);
        wake.notify_one();
        assert_eq!(task.await.unwrap(), h);
        assert_eq!(reads.load(Ordering::SeqCst), 2);

        // A run already reported (its record couldn't be removed) is
        // not followed again.
        let wake = tokio::sync::Notify::new();
        let parked = tokio::time::timeout(
            Duration::from_millis(200),
            idle_until_run(|| Some(h), Some(h), &wake),
        )
        .await;
        assert!(parked.is_err());
    }

    #[test]
    fn unfinished_log_has_no_exit() {
        let s = parse_log("::START 5\nupgrading a\n", 10);
        assert_eq!(s.exit, None);
        assert_eq!(s.done, 1);
        assert_eq!(parse_log("::EXIT junk", 1).exit, Some(-1));
    }

    #[test]
    fn progress_fraction_is_clamped() {
        assert_eq!(fraction(0, 0), 0.0);
        assert_eq!(fraction(1, 4), 0.25);
        assert_eq!(fraction(9, 4), 1.0);
    }

    #[test]
    fn result_messages() {
        assert_eq!(result_message(0, 3).0, "System updated");
        assert_eq!(result_message(126, 0).0, "System update cancelled");
        assert!(result_message(1, 0).1.contains("Exit status 1"));
    }
}
