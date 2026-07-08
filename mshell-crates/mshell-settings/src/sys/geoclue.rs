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

/// `systemctl` unmask (enable) / mask (disable) geoclue.service, privileged.
/// Prefers silent `sudo -n`, falls back to the polkit agent — see
/// [`crate::sys::privileged`]. Err on failure/denial.
pub async fn set_enabled(on: bool) -> Result<(), String> {
    let verb = if on { "unmask" } else { "mask" };
    crate::sys::privileged::run(&["systemctl", verb, "geoclue.service"]).await
}
