#![allow(dead_code)]
use std::ffi::OsStr;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

/// Environment for children that only becomes known once margo is
/// already multi-threaded (XWayland's `DISPLAY`, the matching
/// `XCURSOR_*`). Mutating the process environment at that point races C
/// code on other threads (PipeWire, Mesa, glibc's `localtime`) that calls
/// `getenv` without Rust's env lock, so these live here and are applied
/// to every spawned [`Command`] instead.
static CHILD_ENV: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

fn child_env() -> std::sync::MutexGuard<'static, Vec<(String, String)>> {
    CHILD_ENV.lock().unwrap_or_else(|e| e.into_inner())
}

/// Set `key` for every child spawned from now on.
pub fn set_child_env(key: &str, value: impl Into<String>) {
    let value = value.into();
    let mut env = child_env();
    match env.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => env.push((key.to_owned(), value)),
    }
}

/// `key` as a spawned child would see it: the child-env overlay first,
/// then margo's own process environment.
pub fn child_env_var(key: &str) -> Option<String> {
    if let Some((_, v)) = child_env().iter().find(|(k, _)| k == key) {
        return Some(v.clone());
    }
    std::env::var(key).ok()
}

/// Apply the child-env overlay to `cmd`.
pub fn apply_child_env(cmd: &mut Command) -> &mut Command {
    for (k, v) in child_env().iter() {
        cmd.env(k, v);
    }
    cmd
}

/// Spawn a program asynchronously (mirrors C `spawn`).
///
/// Goes through `setsid --fork` so the command runs in its own session
/// and the short-lived setsid shim exits immediately — the real
/// process then reparents to init, which reaps it. Without this, every
/// short-lived keybind spawn (mlock via the `alt+l` bind, screenshot
/// tools, …) piled up as a `<defunct>` zombie under margo, because the
/// old code dropped the `Child` without ever `wait()`ing. (A global
/// SIGCHLD reaper would race the explicit `wait()`s margo does for
/// grim / xwayland, so detach-and-reparent is the safe fix.)
pub fn spawn<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut it = args.into_iter();
    let program = it
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty spawn args"))?;
    let cmd = Command::new("setsid");
    spawn_detached(cmd, |c| {
        c.arg("--fork").arg(program).args(it);
    })
}

/// Spawn a shell command via `sh -c` (detached + reaped, see [`spawn`]).
pub fn spawn_shell(cmd: &str) -> Result<()> {
    let command = Command::new("setsid");
    spawn_detached(command, |c| {
        c.arg("--fork").arg("sh").arg("-c").arg(cmd);
    })
}

/// Finish configuring `cmd` (null stdio + the caller's args via `build`),
/// launch it, and reap the short-lived `setsid` shim on a detached
/// thread so it can't linger as a zombie either.
fn spawn_detached(mut cmd: Command, build: impl FnOnce(&mut Command)) -> Result<()> {
    build(&mut cmd);
    apply_child_env(&mut cmd);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
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
    let mut vars = vec![
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_TYPE",
    ];
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
        .filter_map(|k| child_env_var(k).map(|v| (*k, v)))
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
            let v = if v.len() > 64 {
                format!("{}…", &v[..63])
            } else {
                v.clone()
            };
            format!("{k}={v}")
        })
        .collect();

    // systemd user manager: imports each NAME from the caller's env
    // into the manager's env block. `inherit_*` and stderr capture
    // so we can include the failure reason in the log.
    let sysd = apply_child_env(&mut Command::new("systemctl"))
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
    let dbus = apply_child_env(&mut Command::new("dbus-update-activation-environment"))
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
            tracing::debug!("import_session_environment: dbus-update-activation skipped: {e}",);
        }
    }
}

/// Returns the current time in milliseconds (monotonic-ish via UNIX epoch).
///
/// Deliberately a wrapping 32-bit ms counter (u32::MAX ≈ 49.7 days) — same
/// convention as GTK/X11 event timestamps. Every caller only ever compares
/// two nearby `now_ms()` values (double-click windows, debounce, animation
/// deltas), so wraparound is fine as long as the truncation itself can't
/// panic. `Duration::as_millis()` is `u128`; the `as u32` cast truncates
/// (never overflow-panics, unlike `+`/`*`) — this used to compute
/// `subsec_millis() + (secs as u32).wrapping_mul(1000)`, where the `+`
/// wasn't wrapping and did panic once the wrapped product landed within
/// 999 of u32::MAX (rare, hence surviving ~a day of uptime before crashing
/// the whole compositor).
pub fn now_ms() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
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
    px >= rx as f64 && py >= ry as f64 && px < (rx + rw) as f64 && py < (ry + rh) as f64
}

/// Longest string a single-string Wayland event can carry: the 4096-byte
/// message cap minus the 8-byte header, 4-byte length prefix and the NUL.
pub const MAX_WIRE_STRING: usize = 4083;

/// Clamp a client-controlled string (window title / app_id) to
/// [`MAX_WIRE_STRING`] on a UTF-8 boundary before it goes on the wire.
/// wayland-backend kills the receiving client when an event write fails,
/// so an oversized title (an XWayland window can set an unbounded one)
/// would otherwise disconnect taskbars and other foreign-toplevel clients.
pub fn clamp_wire_str(s: &str) -> &str {
    if s.len() <= MAX_WIRE_STRING {
        return s;
    }
    let mut end = MAX_WIRE_STRING;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod clamp_wire_str_tests {
    use super::*;

    #[test]
    fn short_strings_pass_through() {
        assert_eq!(clamp_wire_str("kitty"), "kitty");
        let exact = "a".repeat(MAX_WIRE_STRING);
        assert_eq!(clamp_wire_str(&exact), exact);
    }

    #[test]
    fn long_strings_are_cut_to_the_limit() {
        let long = "a".repeat(MAX_WIRE_STRING + 500);
        assert_eq!(clamp_wire_str(&long).len(), MAX_WIRE_STRING);
    }

    #[test]
    fn cut_never_splits_a_multibyte_char() {
        // 'ş' is 2 bytes; an odd limit lands mid-character.
        let long = "ş".repeat(MAX_WIRE_STRING);
        let out = clamp_wire_str(&long);
        assert!(out.len() <= MAX_WIRE_STRING);
        assert!(out.chars().all(|c| c == 'ş'));
    }
}

#[cfg(test)]
mod child_env_tests {
    use super::*;

    #[test]
    fn overlay_wins_and_reaches_spawned_commands() {
        set_child_env("MARGO_CHILD_ENV_TEST", "one");
        set_child_env("MARGO_CHILD_ENV_TEST", "two");
        assert_eq!(
            child_env_var("MARGO_CHILD_ENV_TEST").as_deref(),
            Some("two")
        );
        let mut cmd = Command::new("true");
        apply_child_env(&mut cmd);
        assert!(
            cmd.get_envs()
                .any(|(k, v)| k == "MARGO_CHILD_ENV_TEST" && v == Some(OsStr::new("two")))
        );
    }
}
