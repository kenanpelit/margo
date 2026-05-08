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

/// Push WAYLAND_DISPLAY / DISPLAY / XDG_* into systemd user
/// environment + dbus activation environment, so user-level units
/// (noctalia.service, transient `uwsm app` units, anything else
/// systemd --user spawns) can find our compositor socket and the
/// XWayland display number.
///
/// Why this matters in practice: `uwsm app -a kitty -- kitty` runs
/// kitty as a transient systemd-user unit, which inherits its env
/// from systemd-user environment AT THE MOMENT THE UNIT STARTS.
/// If WAYLAND_DISPLAY isn't there, the kitty (and anything mpv
/// launched from inside it) sees no Wayland socket; mpv probes
/// fall through to X11, then to DRM, which can fight margo for
/// DRM master and crash the session.
///
/// We log which vars actually got pushed (key=value) so a user
/// hitting "mpv falls back to DRM" can verify via `journalctl
/// --user -u margo | grep import_session` whether the push
/// succeeded — silent failure was the previous bug, when
/// `systemctl` exited non-zero (no user manager / dbus broken /
/// permission denied) the import would no-op without any trace.
pub fn import_session_environment(extra: &[&str]) {
    let mut vars = vec!["WAYLAND_DISPLAY", "DISPLAY", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE"];
    vars.extend_from_slice(extra);
    // De-duplicate while preserving order — `extra` may overlap
    // with the defaults (e.g. caller passed "DISPLAY" explicitly
    // because it just became known) and a duplicate arg to
    // systemctl import-environment is a hard error in some
    // versions.
    let mut seen = std::collections::HashSet::new();
    vars.retain(|v| seen.insert(*v));

    let to_push: Vec<(&str, String)> = vars
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (*k, v)))
        .collect();
    if to_push.is_empty() {
        tracing::warn!("import_session_environment: no variables set in process env, skipping");
        return;
    }
    let pushed_keys: Vec<&str> = to_push.iter().map(|(k, _)| *k).collect();
    let pushed_log: Vec<String> = to_push
        .iter()
        .map(|(k, v)| {
            // Truncate long values (a ridiculously-long XCURSOR_THEME
            // shouldn't blow up the log line).
            let v = if v.len() > 64 { format!("{}…", &v[..63]) } else { v.clone() };
            format!("{k}={v}")
        })
        .collect();

    // systemd user manager: imports each NAME from the caller's env
    // into the manager's env block. `inherit_*` and stderr capture
    // so we can include the failure reason in the log.
    let sysd = Command::new("systemctl")
        .arg("--user")
        .arg("import-environment")
        .args(&pushed_keys)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match sysd {
        Ok(out) if out.status.success() => {
            tracing::info!(
                "import_session_environment: systemctl --user OK ({})",
                pushed_log.join(" "),
            );
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            tracing::warn!(
                "import_session_environment: systemctl --user failed (status={}): {}",
                out.status,
                stderr.trim(),
            );
        }
        Err(e) => {
            tracing::warn!("import_session_environment: systemctl --user spawn failed: {e}");
        }
    }

    // dbus activation environment (xdg-desktop-portal & co.) — accepts
    // KEY=VALUE pairs rather than naked names. Independent failure mode
    // from systemd; if dbus daemon isn't running or `dbus-update-…`
    // isn't installed, we just skip without taking the systemd half
    // down.
    let env_args: Vec<String> = to_push.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let dbus = Command::new("dbus-update-activation-environment")
        .arg("--systemd")
        .args(&env_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match dbus {
        Ok(out) if out.status.success() => {
            tracing::info!("import_session_environment: dbus-update-activation OK");
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            tracing::warn!(
                "import_session_environment: dbus-update-activation failed (status={}): {}",
                out.status,
                stderr.trim(),
            );
        }
        Err(e) => {
            // Common: dbus-update-activation-environment not
            // installed (rare today, was the case on minimal
            // arch installs). Demote to debug.
            tracing::debug!(
                "import_session_environment: dbus-update-activation skipped: {e}",
            );
        }
    }
}

/// Returns the current time in milliseconds (monotonic-ish via UNIX epoch).
pub fn now_ms() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_millis() + (d.as_secs() as u32).wrapping_mul(1000))
        .unwrap_or(0)
}

/// Current CLOCK_MONOTONIC time as a `Duration`. Mirrors niri's
/// `crate::utils::get_monotonic_time` so the ported screencasting
/// code keeps its timestamps in the same domain wp_presentation
/// uses elsewhere in margo.
pub fn get_monotonic_time() -> std::time::Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime takes a CLOCK_MONOTONIC clock id and a
    // valid `timespec` pointer; both invariants are held here.
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    std::time::Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
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
