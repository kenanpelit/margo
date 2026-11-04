//! Background upgrade: the whole upgrade runs detached, escalating
//! through polkit (`pkexec`), with every byte of output in a log file
//! the panel tails for a live progress bar + last lines. When the run
//! ends a desktop notification reports the result and the pill
//! re-checks. The interactive terminal mode
//! ([`super::launch_terminal_upgrade`]) stays the default.
//!
//! The run is a `sh -c` in its own process group writing to
//! `~/.cache/mshell/system_update/upgrade.log`, framed by `::START` /
//! `::EXIT <code>` markers, so a restarted shell re-attaches to an
//! unfinished run instead of losing it. Only one watcher task exists
//! per process (multi-monitor setups build one panel per output), so
//! the result is notified once.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::watch;
use tracing::warn;

/// What the upgrade step should run (resolved from PATH + config).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UpgradePlan {
    /// `paru` / `yay` when present and repo/AUR are enabled; `None` →
    /// plain pacman.
    pub(crate) aur_helper: Option<String>,
    /// Run the pacman/AUR step at all (repo or AUR source enabled).
    pub(crate) system: bool,
    /// Also `flatpak update`.
    pub(crate) flatpak: bool,
}

/// Join steps so each runs even when an earlier one failed (one AUR
/// package that won't build must not hold back Flatpak), still exiting
/// with the last failing step's status.
pub(crate) fn chain_steps(steps: &[String]) -> String {
    match steps {
        [] => "true".to_string(),
        [one] => one.clone(),
        many => {
            let mut out = vec!["rc=0".to_string()];
            for s in many {
                out.push(format!("{{ {s} ; }} || rc=$?"));
            }
            out.push("(exit \"$rc\")".to_string());
            out.join("; ")
        }
    }
}

/// The non-interactive background command: pkexec raises the polkit
/// dialog, `--noconfirm` takes every default answer, an AUR helper
/// builds as the user and escalates its install steps via pkexec.
/// `IgnorePkg` in pacman.conf is honoured by pacman itself; packages
/// hidden in the panel are only hidden from the count (holding a
/// package back is a partial upgrade, which Arch doesn't support).
pub(crate) fn background_steps(plan: &UpgradePlan) -> Vec<String> {
    let quiet = " --noconfirm --noprogressbar --color never";
    let mut steps = Vec::new();
    if plan.system {
        steps.push(match plan.aur_helper.as_deref() {
            Some("paru") => format!("paru -Syu --sudo pkexec --skipreview{quiet}"),
            Some(helper) => format!(
                "{helper} -Syu --sudo pkexec --answerdiff None --answerclean None --answeredit None{quiet}"
            ),
            None => format!("pkexec pacman -Syu{quiet}"),
        });
    }
    if plan.flatpak {
        steps.push("flatpak update -y --noninteractive".to_string());
    }
    steps
}

/// The full detached script: START marker, the chained steps, EXIT
/// marker with the chain's status.
pub(crate) fn background_script(steps: &[String], started_at: u64) -> String {
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

pub(crate) fn parse_log(text: &str, keep: usize) -> LogState {
    let mut state = LogState::default();
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = clean_line(raw);
        if let Some(ts) = line.strip_prefix("::START ") {
            state.started = ts.trim().parse().ok();
            continue;
        }
        if let Some(code) = line.strip_prefix("::EXIT ") {
            state.exit = code.trim().parse().ok().or(Some(-1));
            continue;
        }
        if is_progress_line(&line) {
            state.done += 1;
        }
        if !line.trim().is_empty() {
            lines.push(line);
        }
    }
    let skip = lines.len().saturating_sub(keep);
    state.tail = lines.split_off(skip);
    state
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

// ── Run state (one watcher per process) ──────────────────────────

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

fn pid_path() -> PathBuf {
    run_dir().join("upgrade.pid")
}

fn expected_path() -> PathBuf {
    run_dir().join("upgrade.expected")
}

fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// Start a detached background run. Refuses while one is running.
pub(crate) fn start(steps: Vec<String>, expected: usize) -> Result<(), String> {
    if matches!(status(), RunStatus::Running { .. }) {
        return Err("an upgrade is already running".into());
    }
    if steps.is_empty() {
        return Err("nothing to upgrade (no backend / all sources off)".into());
    }
    std::fs::create_dir_all(run_dir()).map_err(|e| e.to_string())?;
    let log = std::fs::File::create(log_path()).map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let script = background_script(&steps, super::now_secs());
    use mshell_process::SpawnReaped;
    use std::os::unix::process::CommandExt;
    let pid = std::process::Command::new("sh")
        .arg("-c")
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(err)
        // Own process group: survives a shell restart, and a Ctrl-C in
        // whatever launched mshell doesn't reach pacman.
        .process_group(0)
        .spawn_reaped()
        .map_err(|e| format!("spawn: {e}"))?;
    let _ = std::fs::write(pid_path(), pid.to_string());
    let _ = std::fs::write(expected_path(), expected.to_string());
    let _ = channel().0.send(RunStatus::Running {
        log: LogState::default(),
        expected,
    });
    ensure_watcher();
    Ok(())
}

/// The panel's "Dismiss" on a finished run.
pub(crate) fn dismiss() {
    if matches!(status(), RunStatus::Finished { .. }) {
        let _ = channel().0.send(RunStatus::Idle);
    }
}

/// Spawn the single watcher (idempotent). Re-attaches to a run left
/// by a previous shell instance.
pub(crate) fn ensure_watcher() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    relm4::spawn(async move {
        loop {
            let text = tokio::fs::read_to_string(log_path())
                .await
                .unwrap_or_default();
            let log = parse_log(&text, 14);
            let expected = std::fs::read_to_string(expected_path())
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            let pid: Option<u32> = std::fs::read_to_string(pid_path())
                .ok()
                .and_then(|s| s.trim().parse().ok());
            let current = status();
            let running = log.started.is_some() && log.exit.is_none() && pid.is_some_and(pid_alive);
            let next = match (&current, running, log.exit) {
                (_, true, _) => RunStatus::Running { log, expected },
                // A run we watched ended (EXIT written, or the process
                // died without one — killed / crashed → -1).
                (RunStatus::Running { .. }, false, exit) => {
                    let mut log = log;
                    log.exit = Some(exit.unwrap_or(-1));
                    let code = log.exit.unwrap_or(-1);
                    notify(code, log.done);
                    let _ = std::fs::remove_file(pid_path());
                    RunStatus::Finished { log, expected }
                }
                (other, false, _) => other.clone(),
            };
            let finished_now = matches!(current, RunStatus::Running { .. })
                && matches!(next, RunStatus::Finished { .. });
            if next != current {
                let _ = channel().0.send(next.clone());
            }
            if finished_now {
                // One re-check for every pill/panel on every monitor.
                let report = super::probe_with_audit(super::ProbeConfig::from_config()).await;
                super::publish_report(report);
            }
            let delay = if matches!(next, RunStatus::Running { .. }) {
                Duration::from_secs(1)
            } else {
                // Idle: just wait for the next start() (cheap wake).
                Duration::from_secs(5)
            };
            tokio::time::sleep(delay).await;
        }
    });
}

fn notify(code: i32, done: usize) {
    use mshell_process::SpawnReaped;
    let (title, body) = result_message(code, done);
    let icon = if code == 0 {
        "software-update-available-symbolic"
    } else {
        "software-update-urgent-symbolic"
    };
    if let Err(e) = std::process::Command::new("notify-send")
        .args(["-a", "System Updates", "-i", icon, &title, &body])
        .spawn_reaped()
    {
        warn!(error = %e, "system_update: notify-send failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_per_backend() {
        let mut plan = UpgradePlan {
            aur_helper: Some("paru".into()),
            system: true,
            flatpak: true,
        };
        assert_eq!(
            background_steps(&plan),
            [
                "paru -Syu --sudo pkexec --skipreview --noconfirm --noprogressbar --color never",
                "flatpak update -y --noninteractive"
            ]
        );
        plan.aur_helper = Some("yay".into());
        assert!(background_steps(&plan)[0].starts_with("yay -Syu --sudo pkexec --answerdiff None"));
        plan.aur_helper = None;
        plan.flatpak = false;
        assert_eq!(
            background_steps(&plan),
            ["pkexec pacman -Syu --noconfirm --noprogressbar --color never"]
        );
        plan.system = false;
        assert!(background_steps(&plan).is_empty());
    }

    #[test]
    fn chain_runs_every_step_and_keeps_the_failure() {
        assert_eq!(chain_steps(&["a".into()]), "a");
        assert_eq!(
            chain_steps(&["a".into(), "b".into()]),
            "rc=0; { a ; } || rc=$?; { b ; } || rc=$?; (exit \"$rc\")"
        );
        assert_eq!(chain_steps(&[]), "true");
    }

    #[test]
    fn chained_script_really_exits_with_the_failing_status() {
        let script = background_script(&["false".into(), "echo ok".into()], 7);
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .output()
            .unwrap();
        let log = parse_log(&String::from_utf8_lossy(&out.stdout), 10);
        assert_eq!(log.started, Some(7));
        assert_eq!(log.exit, Some(1));
        assert_eq!(log.tail, ["ok"]);
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
