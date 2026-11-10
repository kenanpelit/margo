pub(crate) mod cpu_dashboard_menu_widget;
pub(crate) mod drive_chart;
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
pub(crate) mod selftest;
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

/// The pkexec argv signalling `pid` as root: the helper's fixed `kill`
/// verb, which (as root) pins the pid with a pidfd and re-checks its
/// start time (stat field 22) before signalling, so a pid reused while
/// the polkit dialog was open is never hit.
pub(crate) fn kill_argv(pid: i32, start: u64, sig: &str) -> Result<Vec<String>, String> {
    let signal = mshell_privileged::Signal::parse(std::ffi::OsStr::new(sig))
        .map_err(|e| e.message().to_string())?;
    let req = mshell_privileged::Request::Kill { pid, start, signal }
        .validated()
        .map_err(|e| e.message().to_string())?;
    Ok(mshell_privileged::lossy(&req.argv()))
}

/// Signal a process owned by another user as root through pkexec
/// (RULES: root goes through polkit, never sudo), checked against its
/// start time first.
pub(crate) async fn pkexec_kill(pid: i32, start: u64, sig: &'static str) -> Result<(), String> {
    let argv = kill_argv(pid, start, sig)?;
    if !mshell_privileged::installed() {
        return Err(mshell_privileged::MISSING.to_string());
    }
    let out = tokio::process::Command::new("pkexec")
        .args(&argv)
        .output()
        .await
        .map_err(|e| format!("pkexec unavailable: {e}"))?;
    pkexec_outcome(out.status.code(), &String::from_utf8_lossy(&out.stderr))
}

#[cfg(test)]
mod tests {
    use super::{kill_argv, pkexec_outcome};

    #[test]
    fn root_kill_goes_through_the_helper_with_a_fixed_argv() {
        // The start-time re-check now runs as root inside the helper
        // (mshell-privileged's own tests signal a real child with it).
        assert_eq!(
            kill_argv(4242, 777, "TERM").expect("argv"),
            [mshell_privileged::HELPER, "kill", "4242", "777", "TERM"]
        );
        assert_eq!(kill_argv(4242, 777, "KILL").expect("argv")[4], "KILL");
        // Pause / resume of another user's process ride the same verb.
        assert_eq!(kill_argv(4242, 777, "STOP").expect("argv")[4], "STOP");
        assert_eq!(kill_argv(4242, 777, "CONT").expect("argv")[4], "CONT");
        assert!(kill_argv(4242, 777, "HUP").is_err());
        assert!(kill_argv(4242, 777, "TSTP").is_err());
        assert!(kill_argv(1, 777, "TERM").is_err(), "never init");
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
