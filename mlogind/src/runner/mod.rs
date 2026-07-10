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
use std::process::{Child, Command};

use log::{error, info, warn};
use mlogind_proto::{Conn, Event, FdTransport, Request};
use pam::Authenticator;

use crate::auth::{self, utmpx::add_utmpx_entry, UserInfo};
use crate::config::Config;
use crate::info_caching::set_cache;
use crate::post_login::{
    self,
    env_variables::{
        remove_xdg, set_basic_variables, set_display, set_seat_vars, set_session_params,
        set_session_vars, set_xdg_common_paths,
    },
    PostLoginEnvironment,
};

mod converse;
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

    /// Does this host need a seat provider started for it?
    pub fn needs_seatd(self) -> bool {
        matches!(self, Self::Gui | Self::Cage)
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
    // SAFETY: `rfd` is a live SOCK_SEQPACKET socket owned by this scope, and it
    // outlives the `Conn` — `rfd` is dropped at the end of this function, which
    // only runs after `serve` returns.
    let mut conn = Conn::new(unsafe { FdTransport::new(rfd.as_raw_fd()) });

    let code = match spawn_greeter(config, host, gfd) {
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

/// The greeter process and where its output went, if this host has one.
struct Greeter {
    child: Child,
    log: PathBuf,
}

fn spawn_greeter(
    config: &Config,
    host: Host,
    gfd: OwnedFd,
) -> Result<Option<Greeter>, Box<dyn Error>> {
    if host == Host::Tty {
        // The greeter is our parent; it kept its own copy of the socket.
        drop(gfd);
        return Ok(None);
    }

    use std::os::unix::fs::PermissionsExt;

    // Root has no XDG_RUNTIME_DIR. Give the greeter compositor a private 0700
    // tmpfs dir. It no longer holds a credential file — that is the point of A1
    // — but cage and margo still want somewhere to put their sockets.
    let runtime_dir = PathBuf::from("/run/mlogind");
    std::fs::create_dir_all(&runtime_dir)?;
    std::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(0o700))?;

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

            let greeter_conf = runtime_dir.join("greeter.conf");
            crate::write_greeter_conf(&greeter_conf)?;
            let mgreet_log = runtime_dir.join("mgreet.log");

            // However mgreet exits, quit margo so this host returns. The `;`
            // runs the quit even if mgreet crashed.
            let startup = format!(
                "{mgreet} 2>{log}; {mctl} dispatch quit",
                mgreet = mgreet.display(),
                log = mgreet_log.display(),
                mctl = mctl.display(),
            );

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
                .env("MLOGIND_CACHE_PATH", &config.cache_path);
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

    cmd.env("XDG_RUNTIME_DIR", &runtime_dir)
        .env("MLOGIND_SOCK_FD", &sock_fd)
        // libseat: logind (no session) → fails; force seatd, the only backend
        // available to a session-less root process here.
        .env("LIBSEAT_BACKEND", "seatd");
    // Match the greeter keyboard to the machine's console layout.
    for (key, val) in crate::vconsole_xkb_env() {
        cmd.env(key, val);
    }
    // Capture the host's own output — it inherits the greeter's VT otherwise,
    // where a later redraw wipes any error it printed.
    if let Ok(out) = std::fs::File::create(&log) {
        if let Ok(err) = out.try_clone() {
            cmd.stdout(out).stderr(err);
        }
    }

    info!("runner: launching the {host:?} greeter host");
    let child = cmd.spawn()?;

    // Our copy must go, or the greeter exiting never reads as EOF.
    drop(gfd);

    Ok(Some(Greeter { child, log }))
}

/// Serialise the resolved power controls so mgreet renders the same F-key
/// footer and runs the same commands as the TUI greeter: one
/// `key<TAB>hint<TAB>cmd` line per entry.
fn power_env(config: &Config) -> String {
    config
        .power_controls
        .base_entries
        .0
        .iter()
        .chain(config.power_controls.entries.0.iter())
        .filter(|p| !p.key.is_empty() && !p.hint.is_empty())
        .map(|p| format!("{}\t{}\t{}", p.key, p.hint, p.cmd))
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
                Reaped::Abnormal(status) => warn!(
                    "runner: {host:?} greeter host exited abnormally ({status}) after the login was captured; continuing"
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
    Abnormal(std::process::ExitStatus),
}

fn reap(greeter: &mut Greeter) -> Reaped {
    match greeter.child.wait() {
        Ok(status) if status.success() => Reaped::Clean,
        Ok(status) => {
            tail_log(&greeter.log);
            Reaped::Abnormal(status)
        }
        Err(err) => {
            error!("runner: could not wait for the greeter host: {err}");
            Reaped::Clean
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
    match greeter.child.wait() {
        Ok(status) if status.success() => {
            info!("runner: greeter produced no login");
            EXIT_NO_LOGIN
        }
        Ok(status) => {
            error!("runner: greeter host exited abnormally ({status}) with no login");
            tail_log(&greeter.log);
            EXIT_HOST_UNAVAILABLE
        }
        Err(err) => {
            error!("runner: could not wait for the greeter host: {err}");
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
fn start_session(
    config: &Config,
    auth: &mut Authenticator<'_, GreeterConv<'_, FdTransport>>,
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

    info!("runner: waiting for the session to end");
    spawned.wait();
    info!("runner: session ended");

    drop(utmpx_session);
    // `auth` drops in the caller → pam_close_session + setcred(DELETE) + pam_end.
    EXIT_SESSION_ENDED
}
