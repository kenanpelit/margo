use libc::{SIG_DFL, SIG_IGN, SIGUSR1, signal};
use rand::Rng;

use once_cell::sync::Lazy;

use std::env;
use std::error::Error;
use std::fmt::Display;
use std::fs::remove_file;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::{thread, time};

use std::path::{Path, PathBuf};

use log::{error, info};

use crate::auth::UserInfo;
use crate::config::Config;
use crate::post_login::wait_with_log::SessionChild;

const XSTART_CHECK_INTERVAL_MILLIS: u64 = 100;

#[derive(Debug, Clone)]
pub enum XSetupError {
    DisplayEnvVar,
    HomeEnvVar,
    VTNREnvVar,
    FillingXAuth,
    InvalidUTF8Path,
    XServerStart,
    XServerTimeout,
    XServerPrematureExit,
}

impl Display for XSetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DisplayEnvVar => f.write_str("`DISPLAY` is not set"),
            Self::HomeEnvVar => f.write_str("`HOME` is not set"),
            Self::VTNREnvVar => f.write_str("`XDG_VTNR` is not set"),
            Self::FillingXAuth => f.write_str("Failed to fill `.Xauthority` file"),
            Self::InvalidUTF8Path => f.write_str("Path that is given is not valid UTF8"),
            Self::XServerStart => f.write_str("Failed to start X server binary"),
            Self::XServerTimeout => f.write_str("Timeout while waiting for X server to start"),
            Self::XServerPrematureExit => {
                f.write_str("X server exited before it signaled to accept connections")
            }
        }
    }
}

impl Error for XSetupError {}

fn mcookie() -> String {
    // TODO: Verify that this is actually safe. Maybe just use the mcookie binary?? Is that always
    // available?
    let mut rng = rand::rng();
    let cookie: u128 = rng.random();
    format!("{cookie:032x}")
}

/// Split a configured `xserver_path` / `xauth_path` into program and leading
/// arguments on whitespace (`/usr/bin/X -nolisten tcp` keeps working), with no
/// shell in between. Both used to go through `sh -c` with the display and the
/// cookie interpolated into the line, so a `display` from the config was
/// parsed as shell, as root. `None` for an empty setting.
fn split_command(configured: &str) -> Option<(String, Vec<String>)> {
    let mut words = configured.split_whitespace().map(str::to_string);
    let program = words.next()?;
    Some((program, words.collect()))
}

/// How to write the session's MIT cookie into `xauth_file`: program, argv and
/// the script fed to its stdin.
///
/// The cookie goes through stdin (`xauth -q -f <file> source -`), as startx and
/// xinit do. On the command line it was readable by every local account in
/// `/proc/<pid>/cmdline` for as long as xauth ran.
fn xauth_invocation(
    configured: &str,
    xauth_file: &str,
    display: &str,
    cookie: &str,
) -> Option<(String, Vec<String>, String)> {
    let (program, mut args) = split_command(configured)?;
    args.extend(["-q", "-f", xauth_file, "source", "-"].map(str::to_string));
    Some((program, args, format!("add {display} . {cookie}\n")))
}

/// The X server's program and argv: the configured command, the display, the
/// VT and `-auth <xauth_file>`.
///
/// Without `-auth` Xorg has no authorization records and falls back to its
/// host list, which lets every local account connect: the cookie xauth just
/// wrote was never checked. startx passes the same flag. A `-auth` already in
/// the configured command wins.
fn xserver_invocation(
    configured: &str,
    display: &str,
    doubledigit_vtnr: &str,
    xauth_file: &str,
) -> Option<(String, Vec<String>)> {
    let (program, mut args) = split_command(configured)?;
    let has_auth = args.iter().any(|a| a == "-auth");
    args.extend([display.to_string(), format!("vt{doubledigit_vtnr}")]);
    if !has_auth {
        args.extend(["-auth".to_string(), xauth_file.to_string()]);
    }
    Some((program, args))
}

static X_HAS_STARTED: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));

fn handle_sigusr1(_: i32) {
    X_HAS_STARTED.store(true, std::sync::atomic::Ordering::SeqCst);

    unsafe {
        signal(SIGUSR1, handle_sigusr1 as *const () as usize);
    }
}

pub fn setup_x(user_info: &UserInfo, config: &Config) -> Result<SessionChild, XSetupError> {
    use std::os::unix::process::CommandExt;

    info!("Start setup of X server");

    let display_value = env::var("DISPLAY").map_err(|_| XSetupError::DisplayEnvVar)?;
    let vtnr_value = env::var("XDG_VTNR").map_err(|_| XSetupError::VTNREnvVar)?;

    // Setup xauth
    let xauth_dir = PathBuf::from(env::var("HOME").map_err(|_| XSetupError::HomeEnvVar)?);
    let xauth_path = xauth_dir.join(".Xauthority");

    info!(
        "Filling `.Xauthority` file at `{xauth_path}`",
        xauth_path = xauth_path.display()
    );

    // Make sure that we are generating a new file. This is necessary sometimes, since there may be
    // a `root` permission `.Xauthority` file there.
    let _ = remove_file(&xauth_path);

    let xauth_path = xauth_path.to_str().ok_or(XSetupError::InvalidUTF8Path)?;
    let (xauth, xauth_args, xauth_script) = xauth_invocation(
        &config.x11.xauth_path,
        xauth_path,
        &display_value,
        &mcookie(),
    )
    .ok_or(XSetupError::FillingXAuth)?;
    let fill_failed = |reason: &dyn Display| {
        error!("Failed to fill Xauthority file with `xauth`. Reason: {reason}");
        XSetupError::FillingXAuth
    };
    let mut xauth_child = Command::new(xauth)
        .args(&xauth_args)
        .uid(user_info.uid)
        .gid(user_info.primary_gid)
        .stdin(Stdio::piped())
        .stdout(Stdio::null()) // TODO: Maybe this should be logged or something?
        .stderr(Stdio::null()) // TODO: Maybe this should be logged or something?
        .spawn()
        .map_err(|err| fill_failed(&err))?;
    // Write the script, then drop the pipe so xauth sees EOF.
    let written = match xauth_child.stdin.take() {
        Some(mut stdin) => stdin.write_all(xauth_script.as_bytes()),
        None => Err(std::io::Error::other("no stdin pipe")),
    };
    let status = xauth_child.wait().map_err(|err| fill_failed(&err))?;
    written.map_err(|err| fill_failed(&err))?;
    if !status.success() {
        return Err(fill_failed(&status));
    }

    // SAFETY: mlogind is single-threaded, so no other thread can be reading
    // the environment while this writes it.
    unsafe { std::env::set_var("XAUTHORITY", xauth_path) };

    let doubledigit_vtnr = if vtnr_value.len() == 1 {
        format!("0{vtnr_value}")
    } else {
        vtnr_value
    };

    // Here we explicitly ignore the first USR defined signal. Xorg looks at whether this signal
    // is ignored or not. If it is ignored, it will send that signal to the parent when it ready to
    // receive connections. This is also how xinit does it.
    //
    // After we spawn the Xorg process, we need to make sure to quickly re-enable this signal as we
    // need to listen to the signal by Xorg.
    unsafe {
        libc::signal(SIGUSR1, SIG_IGN);
    }

    let (xserver, xserver_args) = xserver_invocation(
        &config.x11.xserver_path,
        &display_value,
        &doubledigit_vtnr,
        xauth_path,
    )
    .ok_or(XSetupError::XServerStart)?;
    let mut child = Command::new(xserver);
    child.args(&xserver_args);

    let log_path = config
        .do_log
        .then_some(Path::new(&config.x11.xserver_log_path));

    let mut child = SessionChild::spawn(child, log_path).map_err(|err| {
        error!("Failed to start X server. Reason: {}", err);
        XSetupError::XServerStart
    })?;

    // See note above
    unsafe {
        libc::signal(SIGUSR1, SIG_DFL);
        signal(SIGUSR1, handle_sigusr1 as *const () as usize);
    }

    // Wait for XServer to boot-up
    let start_time = time::SystemTime::now();
    loop {
        if config.x11.xserver_timeout_secs == 0
            || start_time
                .elapsed()
                .is_ok_and(|t| t.as_secs() > config.x11.xserver_timeout_secs.into())
        {
            break;
        }

        // This will be set by the `handle_sigusr1` signal handler.
        if X_HAS_STARTED.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        if let Some(status) = child.try_wait().unwrap_or(None) {
            error!(
                "X server died before signaling it was ready to received connections. Status code: {status}."
            );

            return Err(XSetupError::XServerPrematureExit);
        }

        thread::sleep(time::Duration::from_millis(XSTART_CHECK_INTERVAL_MILLIS));
    }

    // If the value is still `false`, this means we have time-ed out and Xorg is not running.
    if !X_HAS_STARTED.load(std::sync::atomic::Ordering::SeqCst) {
        child.kill().unwrap_or_else(|err| {
            error!("Failed to kill Xorg after it timed out. Reason: {err}");
        });
        return Err(XSetupError::XServerTimeout);
    }

    if let Ok(x_server_start_time) = start_time.elapsed() {
        info!(
            "It took X server {start_ms}ms to start",
            start_ms = x_server_start_time.as_millis()
        );
    }

    X_HAS_STARTED.store(false, std::sync::atomic::Ordering::SeqCst);

    info!("X server is running");

    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::{split_command, xauth_invocation, xserver_invocation};

    #[test]
    fn the_x_server_is_told_to_check_the_cookie() {
        let (program, args) = xserver_invocation(
            "/usr/bin/X -nolisten tcp",
            ":1",
            "02",
            "/home/u/.Xauthority",
        )
        .unwrap();
        assert_eq!(program, "/usr/bin/X");
        assert_eq!(
            args,
            [
                "-nolisten",
                "tcp",
                ":1",
                "vt02",
                "-auth",
                "/home/u/.Xauthority"
            ]
        );
    }

    #[test]
    fn a_configured_auth_file_is_left_alone() {
        let (_, args) =
            xserver_invocation("/usr/bin/X -auth /etc/x.auth", ":0", "01", "/h").unwrap();
        assert_eq!(args, ["-auth", "/etc/x.auth", ":0", "vt01"]);
    }

    #[test]
    fn the_cookie_goes_to_xauth_on_stdin_never_in_argv() {
        let cookie = "0123456789abcdef0123456789abcdef";
        let (program, args, script) =
            xauth_invocation("/usr/bin/xauth", "/home/u/.Xauthority", ":1", cookie).unwrap();
        assert_eq!(program, "/usr/bin/xauth");
        assert_eq!(args, ["-q", "-f", "/home/u/.Xauthority", "source", "-"]);
        assert!(args.iter().all(|a| !a.contains(cookie)));
        assert_eq!(script, format!("add :1 . {cookie}\n"));
    }

    #[test]
    fn configured_xauth_flags_come_before_the_script() {
        let (_, args, _) = xauth_invocation("xauth -v", "/f", ":0", "c").unwrap();
        assert_eq!(args, ["-v", "-q", "-f", "/f", "source", "-"]);
        assert!(xauth_invocation("  ", "/f", ":0", "c").is_none());
    }

    #[test]
    fn a_plain_path_is_the_program_with_no_arguments() {
        assert_eq!(
            split_command("/usr/bin/X"),
            Some(("/usr/bin/X".to_string(), vec![]))
        );
    }

    #[test]
    fn configured_flags_survive_without_a_shell() {
        assert_eq!(
            split_command("/usr/bin/X  -nolisten tcp"),
            Some((
                "/usr/bin/X".to_string(),
                vec!["-nolisten".to_string(), "tcp".to_string()]
            ))
        );
        // Shell syntax is just text now: no command substitution, no `;`.
        let (_, args) = split_command("/usr/bin/X $(id);reboot").unwrap();
        assert_eq!(args, vec!["$(id);reboot".to_string()]);
    }

    #[test]
    fn an_empty_setting_is_refused() {
        assert_eq!(split_command("   "), None);
    }
}
