//! The session runner: one forked process that owns a login from the first
//! prompt to the last `pam_close_session`.
//!
//! The daemon never touches PAM. That is not tidiness — `pam_open_session`
//! decides the calling process's cgroup, writes `/proc/self/loginuid` when
//! `pam_loginuid` is in the stack, and hands `pam_systemd` the *caller's* PID as
//! the logind session leader. All three must belong to a process that dies with
//! the session. So the daemon forks a runner, the runner does everything, and
//! when it exits the daemon forks a fresh one: no PAM handle, no environment
//! and no fd survives from one login to the next. This is atrium's central
//! invariant (`daemon/session/session_runner.c`) and we get it for free by
//! never running PAM in the parent.
//!
//! The greeter is the runner's child, not the daemon's. That ordering is load
//! bearing: the session compositor must not open DRM until the greeter
//! compositor has exited and released it.

use std::error::Error;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::Command;

use log::{error, info, warn};
use mlogind_proto::{Conn, Event, FdTransport, Request};
use pam::Authenticator;

use crate::auth::{self, UserInfo, utmpx::add_utmpx_entry};
use crate::config::{Config, PowerControl};
use crate::info_caching::set_cache;
use crate::post_login::{
    self, PostLoginEnvironment,
    env_variables::{
        remove_xdg, set_basic_variables, set_display, set_seat_vars, set_session_params,
        set_session_vars, set_xdg_common_paths,
    },
};

mod converse;
mod greeter_session;
pub mod session_active;

use converse::{Abort, GreeterConv};

/// The session ran and ended. The daemon shows the greeter again.
pub const EXIT_SESSION_ENDED: i32 = 0;
/// The greeter quit without a login. The daemon shuts down.
pub const EXIT_NO_LOGIN: i32 = 10;
/// The greeter host could not run at all. The daemon falls down the ladder.
pub const EXIT_HOST_UNAVAILABLE: i32 = 11;
/// Authentication succeeded but the session did not start. The daemon re-greets.
pub const EXIT_SESSION_FAILED: i32 = 12;

/// What the login form is being drawn by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// `margo` + `mgreet`: a GTK login card on every connected output.
    Gui,
    /// `cage` + `foot` + `mlogind --greet`: the TUI form, one output.
    Cage,
    /// The TUI form drawn by the daemon itself on the bare VT. Here the greeter
    /// is our *parent*, so there is no child to spawn or reap.
    Tty,
}

impl Host {
    /// Can this host run at all? Checked in the daemon, before the fork, so a
    /// missing binary falls down the ladder instead of costing a fork per loop.
    pub fn preflight(self) -> Result<(), Box<dyn Error>> {
        let needed: &[&str] = match self {
            // mgreet tears margo down with `mctl dispatch quit`; require mctl up
            // front rather than launching a greeter we could never shut down.
            Self::Gui => &["margo", "mgreet", "mctl"],
            Self::Cage => &["cage", "foot"],
            Self::Tty => &[],
        };
        for bin in needed {
            crate::which(bin).ok_or_else(|| format!("`{bin}` not found in PATH"))?;
        }
        Ok(())
    }
}

/// A `SOCK_SEQPACKET` pair: `(runner end, greeter end)`.
///
/// `SOCK_SEQPACKET` because the kernel then guarantees one `recv` returns
/// exactly one frame — no reassembly state machine in the login path.
pub fn socketpair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as RawFd; 2];
    // SAFETY: `fds` is a valid two-element array for the kernel to fill.
    let rc = unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both fds were just created by the kernel and are owned by us.
    unsafe { Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}

use std::os::fd::FromRawFd;

/// Let the greeter inherit this fd across `exec`.
///
/// `mlogind` is single-threaded, so nothing can `exec` between the `fcntl` and
/// `Command::spawn`, and no `pre_exec` closure is needed.
fn clear_cloexec(fd: RawFd) -> io::Result<()> {
    // SAFETY: `fd` is an open descriptor we own.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The body of the forked child. Never returns.
///
/// `gfd` is the greeter's end of the socket. For [`Host::Gui`] and
/// [`Host::Cage`] it is handed to the greeter process and then dropped here, so
/// the greeter exiting produces a clean EOF. For [`Host::Tty`] the parent kept
/// it and ours is dropped immediately.
pub fn run(config: &Config, host: Host, rfd: OwnedFd, gfd: OwnedFd) -> ! {
    // The daemon blocks its fate signals for its signalfd loop, and the mask
    // is inherited across fork AND exec — without this reset the user's own
    // session would run with SIGTERM blocked, a desktop no `systemctl` could
    // ever stop. First thing, before any child can be spawned.
    crate::daemon::reset_signal_mask();

    // SAFETY: `rfd` is a live SOCK_SEQPACKET socket owned by this scope, and it
    // outlives the `Conn` — `rfd` is dropped at the end of this function, which
    // only runs after `serve` returns.
    let runner_fd = rfd.as_raw_fd();
    let mut conn = Conn::new(unsafe { FdTransport::new(runner_fd) });

    let code = match spawn_greeter(config, host, gfd, runner_fd) {
        Ok(greeter) => serve(config, host, &mut conn, greeter),
        Err(err) => {
            error!("runner: cannot start the {host:?} greeter host: {err}");
            EXIT_HOST_UNAVAILABLE
        }
    };

    drop(conn);
    drop(rfd);
    std::process::exit(code)
}

/// The body of the forked autologin child. Never returns.
///
/// No greeter, no socketpair: the PAM stack's auth phase is `pam_permit`
/// (nobody is at the keyboard to answer anything), and the `PasswordConv`
/// handler exists only to answer the `pam_get_user` prompt with the
/// configured name. Everything from `open_session` on is the exact
/// interactive path — an autologin session is a first-class logind session.
pub fn run_autologin(config: &Config) -> ! {
    // See `run`: the daemon's blocked mask must not reach the session.
    crate::daemon::reset_signal_mask();
    std::process::exit(autologin(config))
}

fn autologin(config: &Config) -> i32 {
    let knob = &config.autologin;
    let Some(post_login_env) = resolve_session(config, &knob.session) else {
        error!(
            "runner: autologin session '{}' does not exist",
            knob.session
        );
        return EXIT_SESSION_FAILED;
    };
    let user_info = match auth::lookup(&knob.user) {
        Ok(info) => info,
        Err(err) => {
            error!("runner: autologin: {err}");
            return EXIT_SESSION_FAILED;
        }
    };
    let mut auth = match Authenticator::with_password(&knob.pam_service) {
        Ok(auth) => auth,
        Err(err) => {
            error!(
                "runner: cannot open PAM service '{}': {err}",
                knob.pam_service
            );
            return EXIT_SESSION_FAILED;
        }
    };
    // The password is never read — the stack permits — but the handler still
    // answers `pam_get_user` with this name.
    auth.get_handler().set_credentials(knob.user.as_str(), "");
    if let Err(err) = auth.authenticate() {
        // A locked or expired account lands here too; the greeter takes over.
        error!(
            "runner: autologin authentication failed for '{}': {err}",
            knob.user
        );
        return EXIT_SESSION_FAILED;
    }
    info!(
        "runner: autologin as '{}' into '{}'",
        knob.user, knob.session
    );
    // The last-login cache is deliberately not written: it pre-fills the
    // *greeter*, whose next appearance means someone just logged out on
    // purpose — config-driven logins are not "the last thing a person typed".
    start_session(config, &mut auth, &post_login_env, &knob.user, &user_info)
}

/// The greeter-session process and where its host's output went.
///
/// A forked pid rather than a `std::process::Child`: between the runner and
/// `margo` there is now a process that opens the greeter's logind session, drops
/// privilege, and holds the PAM handle for as long as the greeter lives.
struct Greeter {
    pid: libc::pid_t,
    log: PathBuf,
}

fn spawn_greeter(
    config: &Config,
    host: Host,
    gfd: OwnedFd,
    runner_fd: RawFd,
) -> Result<Option<Greeter>, Box<dyn Error>> {
    if host == Host::Tty {
        // The greeter is our parent; it kept its own copy of the socket.
        drop(gfd);
        return Ok(None);
    }

    use std::os::unix::fs::PermissionsExt;

    // A world-readable state dir: the greeter is unprivileged now and margo has
    // to read `greeter.conf`. Nothing secret has lived here since A1 removed the
    // credential hand-off. The greeter's own runtime dir comes from pam_systemd
    // (/run/user/<uid>) and is not this.
    let runtime_dir = PathBuf::from("/run/mlogind");
    std::fs::create_dir_all(&runtime_dir)?;
    std::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(0o755))?;

    // atrium's `CREDENTIALS_FD` idiom: the socket rides across `exec` on a
    // known number, announced in the environment.
    clear_cloexec(gfd.as_raw_fd())?;
    let sock_fd = gfd.as_raw_fd().to_string();

    // `Host::Tty` returned above, so this is Gui-or-Cage. An `if` rather than a
    // `match` with an `unreachable!` arm: the login gate takes no panics.
    let (mut cmd, log) = if host == Host::Gui {
        {
            let margo = crate::which("margo").ok_or("`margo` not found in PATH")?;
            let mgreet = crate::which("mgreet").ok_or("`mgreet` not found in PATH")?;
            let mctl = crate::which("mctl").ok_or("`mctl` not found in PATH")?;
            // The on-screen keyboard is a decoration, never a gate: a missing
            // mkeys drops the OSK, not the host (preflight stays untouched).
            let mkeys = config.display.osk.then(|| crate::which("mkeys")).flatten();

            let greeter_conf = runtime_dir.join("greeter.conf");
            let background = crate::write_greeter_conf(&greeter_conf, config)?;

            let startup = greeter_startup(&mgreet, &mctl, mkeys.as_deref());

            let mut cmd = Command::new(&margo);
            cmd.arg("--config")
                .arg(&greeter_conf)
                // Pure-Wayland greeter — never bring up an X server as root.
                .arg("--no-xwayland")
                .arg("--startup-command")
                .arg(&startup)
                // Power controls (F-key footer) mirrored from the TUI greeter.
                .env("MLOGIND_POWER", power_env(config))
                // Read-only: mgreet pre-fills the last user + session. The
                // *write* is the runner's job now.
                .env("MLOGIND_CACHE_PATH", &config.cache_path)
                // Seconds before the greeter blanks itself; 0 disables.
                .env(
                    "MLOGIND_BLANK_SECS",
                    config.display.blank_timeout.to_string(),
                );
            // The `background_dir` pick, so the login card paints the same
            // image the compositor wallpaper shows behind it.
            if let Some(background) = background {
                cmd.env("MLOGIND_BACKGROUND", background);
            }
            // Admin CSS layered over the palette. Checked here, as root: the
            // greeter should not learn a path it can never read.
            let css = &config.display.greeter_css;
            if !css.is_empty() {
                if Path::new(css).is_file() {
                    cmd.env("MLOGIND_CSS", css);
                } else {
                    warn!("runner: greeter_css '{css}' is not a readable file; skipping it");
                }
            }
            (cmd, runtime_dir.join("margo-greeter.log"))
        }
    } else {
        {
            let cage = crate::which("cage").ok_or("`cage` not found in PATH")?;
            let foot = crate::which("foot").ok_or("`foot` not found in PATH")?;
            let self_exe = std::env::current_exe()?;

            let mut cmd = Command::new(&cage);
            cmd.arg("-m") // output mode: "last" confines the greeter to one monitor
                .arg(&config.display.output_mode)
                .arg("-s") // allow VT switching → escape hatch stays open
                .arg("--")
                .arg(&foot)
                .arg(&self_exe)
                .arg("--greet");
            (cmd, runtime_dir.join("cage.log"))
        }
    };

    // XDG_RUNTIME_DIR is deliberately NOT set: pam_systemd gives the greeter
    // session its own (/run/user/<uid>), and a root-owned path in its way would
    // only break it. Nor is LIBSEAT_BACKEND: the greeter has a real logind
    // session now, so libseat finds its logind backend by itself. Those two lines
    // were the entire reason `seatd` had to be started by hand.
    cmd.env("MLOGIND_SOCK_FD", &sock_fd);
    // Match the greeter keyboard to the machine's console layout.
    for (key, val) in crate::vconsole_xkb_env() {
        cmd.env(key, val);
    }
    // Capture the host's own output — it inherits the greeter's VT otherwise,
    // where a later redraw wipes any error it printed.
    if let Ok(out) = std::fs::File::create(&log)
        && let Ok(err) = out.try_clone()
    {
        cmd.stdout(out).stderr(err);
    }

    info!("runner: launching the {host:?} greeter host");

    // SAFETY: mlogind is single-threaded.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error().into());
    }
    if pid == 0 {
        // A fork is not an exec, so CLOEXEC did nothing for our inherited copy
        // of the runner's end. Close it, or the greeter exiting never reads as
        // EOF and `serve` waits forever.
        // SAFETY: this child never touches that descriptor again.
        unsafe { libc::close(runner_fd) };
        greeter_session::run(config, cmd);
    }

    // Our copy of the greeter's end must go too, for the same reason.
    drop(gfd);

    Ok(Some(Greeter { pid, log }))
}

/// The shell line the greeter compositor runs as its startup command.
///
/// However mgreet exits, quit margo so this host returns — the `;` runs the
/// quit even if mgreet crashed. mgreet's output flows through margo's stderr
/// into the log the runner opens, as root, and passes down as an inherited fd.
///
/// With an OSK, mkeys floats over the card for touch login and is killed the
/// moment mgreet is done — margo's quit would reap it anyway, but not before
/// it repainted over a session that is trying to start.
fn greeter_startup(mgreet: &Path, mctl: &Path, mkeys: Option<&Path>) -> String {
    let mgreet = mgreet.display();
    let mctl = mctl.display();
    match mkeys {
        Some(mkeys) => format!(
            "{mkeys} & OSK=$!; {mgreet}; kill $OSK 2>/dev/null; {mctl} dispatch quit",
            mkeys = mkeys.display(),
        ),
        None => format!("{mgreet}; {mctl} dispatch quit"),
    }
}

/// Every configured power action, base entries first.
///
/// This ordering *is* the wire format: `Request::Power` carries an index into
/// this list, and both greeters build their footer from the same config in the
/// same order. Filtering here — dropping an entry with a blank key, say — would
/// silently shift every index after it.
fn power_entries(config: &Config) -> Vec<&PowerControl> {
    config
        .power_controls
        .base_entries
        .0
        .iter()
        .chain(config.power_controls.entries.0.iter())
        .collect()
}

/// Run the configured power action at `index`, as root, and always answer.
///
/// The greeter sends an index, never a command: it is unprivileged, we are not,
/// and letting it name what we run would hand back in one line exactly the
/// privilege it just gave up.
///
/// The reply matters. Most power actions never return — the machine is going
/// down — but `suspend` does, and a greeter that blocks for an answer (the TUI)
/// must not hang when one does.
fn run_power(config: &Config, index: u32, conn: &mut Conn<FdTransport>) {
    let entries = power_entries(config);
    let Some(entry) = usize::try_from(index).ok().and_then(|i| entries.get(i)) else {
        // Both sides read the same config in the same order, so this means they
        // disagree about it. Worth saying out loud rather than running entry 0.
        error!("runner: greeter asked for power action {index}, which does not exist");
        let _ = conn.send_event(&Event::Error {
            text: "Unknown power action".to_string(),
        });
        return;
    };

    info!(
        "runner: running power action '{}': {}",
        entry.hint, entry.cmd
    );
    let output = Command::new(&config.system_shell)
        .arg("-c")
        .arg(&entry.cmd)
        .output();

    let event = match output {
        Ok(out) if out.status.success() => Event::Info {
            text: format!("{}…", entry.hint),
        },
        Ok(out) => {
            error!(
                "runner: power action '{}' exited {}: {}",
                entry.hint,
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
            Event::Error {
                text: format!("Failed to {}", entry.hint),
            }
        }
        Err(err) => {
            error!("runner: could not run power action '{}': {err}", entry.hint);
            Event::Error {
                text: format!("Failed to {}", entry.hint),
            }
        }
    };
    let _ = conn.send_event(&event);
}

/// Serialise the power controls mgreet renders in its F-key footer: one
/// `index<TAB>key<TAB>hint` line per entry.
///
/// The command is deliberately absent. mgreet is unprivileged and could not run
/// it anyway; shipping root commands into an unprivileged process's environment
/// buys nothing. The index is explicit rather than implied by line order,
/// because blank entries are skipped here but still occupy a slot in
/// [`power_entries`], which is what `Request::Power` indexes into.
fn power_env(config: &Config) -> String {
    power_entries(config)
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.key.is_empty() && !p.hint.is_empty())
        .map(|(i, p)| format!("{i}\t{}\t{}", p.key, p.hint))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run conversations until one succeeds, the greeter leaves, or the host dies.
fn serve(
    config: &Config,
    host: Host,
    conn: &mut Conn<FdTransport>,
    mut greeter: Option<Greeter>,
) -> i32 {
    // A `Begin` the greeter sent out of turn, carried across the retry.
    let mut pending: Option<(String, String)> = None;

    loop {
        let (username, session_name) = match pending.take() {
            Some(begin) => begin,
            None => match conn.recv_request() {
                Ok(Some(Request::Begin { user, session })) => (user, session),
                // The greeter is unprivileged; shutting the machine down is ours.
                Ok(Some(Request::Power { index })) => {
                    run_power(config, index, conn);
                    continue;
                }
                // A Cancel with no conversation in flight, or a stray Response.
                Ok(Some(_)) => continue,
                Ok(None) => return no_login(greeter.as_mut()),
                Err(err) => {
                    error!("runner: protocol error waiting for a login: {err}");
                    return no_login(greeter.as_mut());
                }
            },
        };

        let Some(post_login_env) = resolve_session(config, &session_name) else {
            error!("runner: greeter chose unknown session '{session_name}'");
            let _ = conn.send_event(&Event::Failure {
                reason: format!("Unknown session '{session_name}'"),
            });
            continue;
        };

        // A fresh `pam_start` per attempt. Retrying `authenticate()` on a handle
        // whose `acct_mgmt` already failed is not well defined; atrium likewise
        // starts a new PAM context for every try.
        let conv = GreeterConv::new(conn, username.clone());
        let mut auth = match Authenticator::with_handler(&config.pam_service, conv) {
            Ok(auth) => auth,
            Err(err) => {
                error!(
                    "runner: cannot open PAM service '{}': {err}",
                    config.pam_service
                );
                return EXIT_SESSION_FAILED;
            }
        };

        // `authenticate()` runs pam_authenticate and pam_acct_mgmt. Every prompt
        // it raises becomes a round trip to the greeter.
        if let Err(pam_err) = auth.authenticate() {
            match auth.get_handler().abort() {
                Some(Abort::Eof | Abort::Broken) => {
                    drop(auth); // pam_end
                    return no_login(greeter.as_mut());
                }
                Some(Abort::Cancelled) => {
                    // The greeter withdrew. Nothing to report back to it.
                    pending = auth.get_handler().take_pending_begin();
                }
                None => {
                    // Log the real reason; tell the greeter only that it failed.
                    // Whether the account exists is not the greeter's business.
                    info!("runner: authentication failed for '{username}': {pam_err}");
                    auth.get_handler().send_failure("Invalid login credentials");
                }
            }
            continue; // `auth` drops here → pam_end
        }

        info!("runner: authentication succeeded for '{username}'");
        auth.get_handler().send_success();

        // Look the account up before `open_session`, whose `initialize_environment`
        // would otherwise panic on a user that vanished between the two.
        let user_info = match auth::lookup(&username) {
            Ok(info) => info,
            Err(err) => {
                error!("runner: {err}");
                return EXIT_SESSION_FAILED;
            }
        };

        // The greeter must be off the screen before we take DRM.
        match greeter.as_mut() {
            Some(g) => match reap(g) {
                // The greeter compositor can die tearing down its outputs long
                // after the login was captured. Honour the login regardless: the
                // kernel drops DRM master on exit either way.
                Reaped::Abnormal(code) => warn!(
                    "runner: {host:?} greeter host exited abnormally ({code}) after the login was captured; continuing"
                ),
                Reaped::Clean => {}
            },
            // No child to reap: the TTY greeter is our parent. It closes the
            // socket once it has left the alternate screen.
            None => auth.get_handler().wait_for_eof(),
        }

        // Remember the login. The greeter used to write this file; it has no
        // business writing to /var/cache as root, and under A2 it will not be able to.
        set_cache(Some(&session_name), Some(&username), config);

        return start_session(config, &mut auth, &post_login_env, &username, &user_info);
    }
}

/// Resolve the session name the greeter chose. Both sides derive their list
/// from `get_envs` in the same order, so a name that round-trips resolves here.
fn resolve_session(config: &Config, name: &str) -> Option<PostLoginEnvironment> {
    post_login::get_envs(config)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, env)| env)
}

enum Reaped {
    Clean,
    Abnormal(i32),
}

fn reap(greeter: &mut Greeter) -> Reaped {
    match crate::wait_for(greeter.pid) {
        0 => Reaped::Clean,
        code => {
            tail_log(&greeter.log);
            Reaped::Abnormal(code)
        }
    }
}

/// The greeter produced no login. Distinguish "the user quit" from "the host is
/// broken" the way the old orchestrator did: by the host's exit status.
fn no_login(greeter: Option<&mut Greeter>) -> i32 {
    let Some(greeter) = greeter else {
        info!("runner: greeter produced no login");
        return EXIT_NO_LOGIN;
    };
    match crate::wait_for(greeter.pid) {
        0 => {
            info!("runner: greeter produced no login");
            EXIT_NO_LOGIN
        }
        code => {
            error!("runner: greeter host exited abnormally ({code}) with no login");
            tail_log(&greeter.log);
            EXIT_HOST_UNAVAILABLE
        }
    }
}

fn tail_log(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tail: Vec<&str> = text.lines().rev().take(12).collect();
    for line in tail.into_iter().rev() {
        error!("{name}: {line}");
    }
}

/// Open the PAM session in *this* process, then run the user's compositor as a
/// child of it.
///
/// `open_session` must run here and nowhere else: it decides our cgroup, writes
/// our `loginuid`, and makes us the logind session leader. Keeping the runner
/// alive for the whole session is what lets logind tear the session down the
/// moment we exit.
/// Refresh what the greeters render from the user's desktop: mgreet's blurred
/// backdrop and matugen CSS, and the TUI's palette.
///
/// Best-effort by construction. A decoration that could not be prepared is never
/// a reason to change how a login ends, so nothing here reaches the exit code.
fn refresh_greeter_theme(user_info: &UserInfo) {
    match crate::theme_sync::sync(user_info) {
        Ok(written) if written.is_empty() => {
            info!("runner: nothing to sync into the greeter's theme");
        }
        Ok(written) => info!("runner: refreshed {} greeter asset(s)", written.len()),
        Err(err) => warn!("runner: could not refresh the greeter's theme: {err}"),
    }
}

fn start_session<C: pam::Converse>(
    config: &Config,
    auth: &mut Authenticator<'_, C>,
    post_login_env: &PostLoginEnvironment,
    username: &str,
    user_info: &UserInfo,
) -> i32 {
    // pam_systemd reads these to register the session on the right seat and VT.
    // It looks them up with `getenv_harder()`, which tries the PAM environment
    // first and falls back to the process environment — which is what these set.
    if matches!(post_login_env, PostLoginEnvironment::X { .. }) {
        set_display(&config.x11.x11_display);
    }
    remove_xdg();
    set_session_params(post_login_env);
    set_seat_vars(config.tty);

    // The greeter session lived on this VT. `pam_close_session` started its
    // teardown when the greeter-session process exited, but logind finishes
    // asynchronously — and pam_systemd will not put a session on a VT that still
    // has one.
    session_active::wait_vt_free(u32::from(config.tty));

    if let Err(err) = auth.open_session() {
        error!("runner: failed to open a PAM session: {err}");
        return EXIT_SESSION_FAILED;
    }

    // pam_systemd populated XDG_RUNTIME_DIR and XDG_SESSION_ID; these adopt them.
    set_session_vars(user_info.uid);
    set_basic_variables(
        username,
        &user_info.home_dir,
        &user_info.shell,
        &config.initial_path,
    );
    set_xdg_common_paths(&user_info.home_dir);

    // logind grants device access asynchronously. Do not race it to the DRM node.
    session_active::wait();

    let spawned = match post_login_env.spawn(user_info, config) {
        Ok(spawned) => spawned,
        Err(err) => {
            error!("runner: failed to start the session environment: {err}");
            return EXIT_SESSION_FAILED;
        }
    };

    let utmpx_session = add_utmpx_entry(username, config.tty, spawned.pid());

    // Bake the greeter's backdrop while the session is coming up. The runner is
    // idle here until the session ends, so this costs nothing anyone can feel.
    //
    // Doing it *only* at logout was wrong: a machine that reboots rather than
    // logging out kills the runner before it gets there, so the sync would never
    // run at all and the login screen would stay flat forever. Whatever else
    // happens now, one sync has already landed.
    //
    // mshell rewrites `wallpaper.raw` shortly after it starts, so this can read
    // a half-written file. That loses safely: a header that disagrees with the
    // byte count is rejected, nothing is published, and the copy already on disk
    // stands.
    refresh_greeter_theme(user_info);

    info!("runner: waiting for the session to end");
    spawned.wait();
    info!("runner: session ended");

    // Again, with the desktop gone. This one catches a wallpaper the user
    // changed during the session, and reads a file nothing is still writing.
    refresh_greeter_theme(user_info);

    drop(utmpx_session);
    // `auth` drops in the caller → pam_close_session + setcred(DELETE) + pam_end.
    EXIT_SESSION_ENDED
}

#[cfg(test)]
mod tests {
    use super::greeter_startup;
    use std::path::Path;

    #[test]
    fn the_startup_line_quits_margo_however_mgreet_ends() {
        let line = greeter_startup(
            Path::new("/usr/bin/mgreet"),
            Path::new("/usr/bin/mctl"),
            None,
        );
        assert_eq!(line, "/usr/bin/mgreet; /usr/bin/mctl dispatch quit");
    }

    #[test]
    fn the_osk_floats_beside_the_greeter_and_dies_with_it() {
        let line = greeter_startup(
            Path::new("/usr/bin/mgreet"),
            Path::new("/usr/bin/mctl"),
            Some(Path::new("/usr/bin/mkeys")),
        );
        assert_eq!(
            line,
            "/usr/bin/mkeys & OSK=$!; /usr/bin/mgreet; kill $OSK 2>/dev/null; /usr/bin/mctl dispatch quit"
        );
    }
}
