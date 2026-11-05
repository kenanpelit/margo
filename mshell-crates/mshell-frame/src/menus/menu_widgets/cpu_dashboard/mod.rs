pub(crate) mod cpu_dashboard_menu_widget;
pub(crate) mod drives;
pub(crate) mod drives_tab;
pub(crate) mod fan;
pub(crate) mod fan_section;
pub(crate) mod gpu;
pub(crate) mod gpu_section;
pub(crate) mod keyed_rows;
pub(crate) mod ports;
pub(crate) mod ports_tab;
pub(crate) mod processes_tab;
pub(crate) mod procs;
pub(crate) mod ui;

/// Map a finished `pkexec` run to the user-facing outcome. 126 = the
/// dialog was dismissed / authorization refused, 127 = authentication
/// failed (pkexec(1)); any other non-zero is the wrapped tool's own error.
pub(crate) fn pkexec_outcome(code: Option<i32>, stderr: &str) -> Result<(), String> {
    match code {
        Some(0) => Ok(()),
        Some(126) => Err("Authorization was cancelled".to_string()),
        Some(127) => Err("Authorization failed".to_string()),
        Some(c) => {
            let msg = stderr.lines().last().unwrap_or("").trim();
            if msg.is_empty() {
                Err(format!("Command failed (exit {c})"))
            } else {
                Err(msg.to_string())
            }
        }
        None => Err("Command was killed by a signal".to_string()),
    }
}

/// The root side of [`pkexec_kill`] (`sh -c SCRIPT sh <pid> <start>
/// <sig>`): signal `pid` only if it still started at `start` (stat field
/// 22), so a pid reused while the polkit dialog was open is never hit.
/// `comm` (field 2) may hold spaces and `)`, so the fields are split after
/// the last `) `; field 22 is the 20th from there.
const KILL_SCRIPT: &str = r#"pid=$1 start=$2 sig=$3
stat=$(cat "/proc/$pid/stat" 2>/dev/null) || { echo "Process $pid has already exited" >&2; exit 3; }
rest=${stat##*) }
set -f
set -- $rest
[ "${20}" = "$start" ] || { echo "Process $pid has already exited" >&2; exit 3; }
exec kill -s "$sig" -- "$pid""#;

/// `kill -s <sig> -- <pid>` as root through pkexec for a process owned by
/// another user (RULES: root goes through polkit, never sudo), checked
/// against its start time first.
pub(crate) async fn pkexec_kill(pid: i32, start: u64, sig: &'static str) -> Result<(), String> {
    let out = tokio::process::Command::new("pkexec")
        .args(["/bin/sh", "-c", KILL_SCRIPT, "sh"])
        .args([pid.to_string(), start.to_string(), sig.to_string()])
        .output()
        .await
        .map_err(|e| format!("pkexec unavailable: {e}"))?;
    pkexec_outcome(out.status.code(), &String::from_utf8_lossy(&out.stderr))
}

#[cfg(test)]
mod tests {
    use super::{KILL_SCRIPT, pkexec_outcome};
    use std::path::Path;

    /// Run the root script as ourselves (no pkexec) against a child.
    fn run_script(pid: u32, start: u64) -> std::process::Output {
        std::process::Command::new("/bin/sh")
            .args(["-c", KILL_SCRIPT, "sh"])
            .args([pid.to_string(), start.to_string(), "TERM".to_string()])
            .output()
            .expect("sh")
    }

    #[test]
    fn root_kill_script_checks_the_start_time() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        let start = super::procs::proc_start(Path::new("/proc"), pid as i32).expect("start");

        let reused = run_script(pid, start + 1);
        assert_eq!(reused.status.code(), Some(3), "a reused pid is refused");
        assert_eq!(
            pkexec_outcome(
                reused.status.code(),
                &String::from_utf8_lossy(&reused.stderr)
            ),
            Err(format!("Process {pid} has already exited"))
        );
        assert!(child.try_wait().expect("wait").is_none(), "not signalled");

        let ok = run_script(pid, start);
        assert_eq!(ok.status.code(), Some(0));
        assert!(!child.wait().expect("wait").success(), "terminated");
        assert_eq!(run_script(pid, start).status.code(), Some(3), "gone now");
    }

    #[test]
    fn pkexec_exit_codes() {
        assert_eq!(pkexec_outcome(Some(0), ""), Ok(()));
        assert_eq!(
            pkexec_outcome(Some(126), ""),
            Err("Authorization was cancelled".to_string())
        );
        assert_eq!(
            pkexec_outcome(Some(127), ""),
            Err("Authorization failed".to_string())
        );
        assert_eq!(
            pkexec_outcome(Some(1), "kill: (99): No such process\n"),
            Err("kill: (99): No such process".to_string())
        );
        assert_eq!(
            pkexec_outcome(Some(2), ""),
            Err("Command failed (exit 2)".to_string())
        );
        assert!(pkexec_outcome(None, "").is_err());
    }
}
