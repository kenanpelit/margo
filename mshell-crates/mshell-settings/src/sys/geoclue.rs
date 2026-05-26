use std::process::Stdio;
use tokio::process::Command;

/// (installed, enabled) — `enabled` is false when the unit is masked or disabled.
pub async fn status() -> (bool, bool) {
    let listed = Command::new("systemctl")
        .env("LC_ALL", "C")
        .args(["list-unit-files", "geoclue.service"])
        .stdin(Stdio::null())
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("geoclue.service"))
        .unwrap_or(false);
    if !listed {
        return (false, false);
    }
    let state = Command::new("systemctl")
        .env("LC_ALL", "C")
        .args(["is-enabled", "geoclue.service"])
        .stdin(Stdio::null())
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    let enabled = state != "masked" && state != "disabled";
    (true, enabled)
}

/// pkexec systemctl unmask (enable) / mask (disable) geoclue.service.
/// Authenticates through the running mshell-polkit agent. Err(stderr) on failure/denial.
pub async fn set_enabled(on: bool) -> Result<(), String> {
    let verb = if on { "unmask" } else { "mask" };
    let o = Command::new("pkexec")
        .args(["systemctl", verb, "geoclue.service"])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("failed to spawn pkexec: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_owned())
    }
}
