pub(crate) mod cpu_dashboard_menu_widget;
pub(crate) mod drives;
pub(crate) mod drives_tab;
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

/// `pkexec kill -s <sig> -- <pid>` for a process owned by another user
/// (RULES: root goes through polkit, never sudo).
pub(crate) async fn pkexec_kill(pid: i32, sig: &'static str) -> Result<(), String> {
    let out = tokio::process::Command::new("pkexec")
        .args(["kill", "-s", sig, "--", &pid.to_string()])
        .output()
        .await
        .map_err(|e| format!("pkexec unavailable: {e}"))?;
    pkexec_outcome(out.status.code(), &String::from_utf8_lossy(&out.stderr))
}

#[cfg(test)]
mod tests {
    use super::pkexec_outcome;

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
