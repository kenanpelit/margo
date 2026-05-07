#![allow(dead_code)]
use std::ffi::OsStr;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

/// Spawn a program asynchronously (mirrors C `spawn`).
pub fn spawn<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut it = args.into_iter();
    let program = it.next().ok_or_else(|| anyhow::anyhow!("empty spawn args"))?;
    Command::new(program)
        .args(it)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

/// Spawn a shell command via `sh -c`.
pub fn spawn_shell(cmd: &str) -> Result<()> {
    Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

/// Push WAYLAND_DISPLAY (and optionally DISPLAY for XWayland) into systemd
/// user environment + dbus activation environment, so user-level units
/// like noctalia.service launched by `systemd --user` can find our socket.
pub fn import_session_environment(extra: &[&str]) {
    let mut vars = vec!["WAYLAND_DISPLAY", "DISPLAY", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE"];
    vars.extend_from_slice(extra);

    // systemd user manager
    let _ = Command::new("systemctl")
        .arg("--user")
        .arg("import-environment")
        .args(&vars)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // dbus activation environment (for desktop portal etc.)
    let env_args: Vec<String> = vars
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| format!("{k}={v}")))
        .collect();
    if !env_args.is_empty() {
        let _ = Command::new("dbus-update-activation-environment")
            .arg("--systemd")
            .args(&env_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Returns the current time in milliseconds (monotonic-ish via UNIX epoch).
pub fn now_ms() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_millis() + (d.as_secs() as u32).wrapping_mul(1000))
        .unwrap_or(0)
}

/// Clamp an integer value.
#[inline]
pub fn clamp_i32(x: i32, min: i32, max: i32) -> i32 {
    x.clamp(min, max)
}

/// Clamp a float value.
#[inline]
pub fn clamp_f32(x: f32, min: f32, max: f32) -> f32 {
    x.clamp(min, max)
}

/// Check if a point (px, py) is inside a rectangle.
#[inline]
pub fn point_in_rect(px: f64, py: f64, rx: i32, ry: i32, rw: i32, rh: i32) -> bool {
    px >= rx as f64
        && py >= ry as f64
        && px < (rx + rw) as f64
        && py < (ry + rh) as f64
}
