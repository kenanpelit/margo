//! Privileged-action runner for the Settings pages.
//!
//! Settings is a layer-shell surface that holds a keyboard grab, so a `pkexec`
//! polkit *password* dialog spawned from here can't reliably receive keyboard
//! focus — the same failure the DNS menu hit, where the prompt appeared but
//! couldn't be typed into and the shell looked frozen. When the user has
//! passwordless sudo we therefore prefer a **silent `sudo -n`**, which needs no
//! dialog at all; only when that's unavailable do we fall back to `pkexec` (the
//! integrated mshell-polkit agent), preserving the previous behaviour rather
//! than regressing it. Either way the action returns an error, never hangs.

use std::process::Stdio;
use tokio::process::Command;

/// True when `sudo` can run without prompting (NOPASSWD rule or cached creds).
/// Probed before use so a genuine command failure is never mistaken for an
/// auth failure (which would otherwise double-run the command under `pkexec`).
async fn have_sudo_n() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run `args` with elevated privileges — silent `sudo -n` when available, else
/// `pkexec`. `Err` on failure or a dismissed prompt (never a hang).
pub async fn run(args: &[&str]) -> Result<(), String> {
    let sudo = have_sudo_n().await;
    let mut cmd = Command::new(if sudo { "sudo" } else { "pkexec" });
    if sudo {
        cmd.arg("-n");
    }
    let out = cmd
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("privileged command failed to start: {e}"))?;
    classify(out.status, &out.stderr, !sudo)
}

/// Like [`run`] but feeds `input` to the command's stdin (e.g. `chpasswd`,
/// `cat > file`) so a secret/body never lands in the process arguments.
pub async fn run_with_stdin(args: &[&str], input: &[u8]) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let sudo = have_sudo_n().await;
    let mut cmd = Command::new(if sudo { "sudo" } else { "pkexec" });
    if sudo {
        cmd.arg("-n");
    }
    let mut child = cmd
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("privileged command failed to start: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input).await;
        let _ = stdin.shutdown().await;
    }
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    classify(out.status, &out.stderr, !sudo)
}

/// Map an exit status to a user-facing result. A `pkexec` exit 126 means the
/// polkit prompt was dismissed; otherwise surface the last stderr line.
fn classify(
    status: std::process::ExitStatus,
    stderr: &[u8],
    is_pkexec: bool,
) -> Result<(), String> {
    if status.success() {
        return Ok(());
    }
    if is_pkexec && status.code() == Some(126) {
        return Err("Authorization dismissed.".into());
    }
    let err = String::from_utf8_lossy(stderr);
    let line = err.lines().last().unwrap_or("command failed").trim();
    Err(format!("Failed: {line}"))
}
