//! The greeter's own logind session — and the process that drops privilege into it.
//!
//! The login screen is the most exposed surface a desktop has: a GTK stack
//! decoding whatever image the theme points at, shaping a hostname read off the
//! network. It has no business being root. A1 removed the last reason it needed
//! to be — it runs no PAM and writes no credentials — so here it stops.
//!
//! atrium asks logind for the session directly, over D-Bus, and says why:
//! `pam_systemd` ties the session to the calling process, so it would need
//! another dedicated fork. We already fork a session runner per login, so one
//! more fork is free — and taking the PAM route keeps zbus, zvariant and
//! async-io out of a login manager. SDDM does exactly this
//! (`src/helper/HelperApp.cpp`).
//!
//! ```text
//! greeter session (root)      ← THIS file. pam_open_session ⇒ logind session leader.
//!   └─ margo / cage           ← setgroups + setgid + setuid, then exec.
//! ```
//!
//! `margo` inherits this process's cgroup, so `sd_pid_get_session()` resolves for
//! it and libseat's logind backend hands it the seat's DRM node. That is what
//! retired `seatd` and `LIBSEAT_BACKEND`.

use std::ffi::{CStr, CString};
use std::process::Command;

use log::{error, info};
use pam::Authenticator;

use crate::auth::{self, UserInfo};
use crate::config::Config;

use super::session_active;

/// Exit code when the greeter's PAM session could not be opened. The runner turns
/// this into `EXIT_HOST_UNAVAILABLE` and the daemon falls down the host ladder.
pub const EXIT_NO_SESSION: i32 = 1;

/// A [`pam::Converse`] for a stack that never asks anything.
///
/// The greeter user is not a login: `/etc/pam.d/mlogind-greeter` is `pam_permit`
/// in its auth phase and exists for `pam_systemd` in its session phase. Any
/// prompt at all means the stack was edited into something it should not be, so
/// refuse rather than guess an answer.
struct SilentConv {
    username: String,
}

impl pam::Converse for SilentConv {
    fn prompt_echo(&mut self, msg: &CStr) -> Result<CString, ()> {
        error!(
            "greeter session: PAM asked '{}'; the greeter stack must not prompt",
            msg.to_string_lossy()
        );
        Err(())
    }

    fn prompt_blind(&mut self, msg: &CStr) -> Result<CString, ()> {
        self.prompt_echo(msg)
    }

    fn info(&mut self, msg: &CStr) {
        info!("greeter session: pam: {}", msg.to_string_lossy());
    }

    fn error(&mut self, msg: &CStr) {
        error!("greeter session: pam: {}", msg.to_string_lossy());
    }

    fn username(&self) -> &str {
        &self.username
    }
}

/// The body of the forked greeter-session process. Never returns.
///
/// `cmd` is the greeter host, already built (margo, or cage). We open the logind
/// session here so that *this* pid is its leader — logind tears the session down
/// the moment we exit — then drop privilege in the child and exec.
pub fn run(config: &Config, mut cmd: Command) -> ! {
    let greeter = auth::lookup_greeter(&config.greeter_user);
    // Whose session logind is asked to create. With no greeter user we still
    // want a session — that is what gives margo its DRM — just a root one.
    let session_user = greeter
        .as_ref()
        .map(|_| config.greeter_user.clone())
        .unwrap_or_else(|| "root".to_string());

    // The previous login's session may still be on this VT: `pam_close_session`
    // has run, but logind tears down asynchronously and will not put a second
    // session on a VT that still has one. Same wait the user session does before
    // taking the VT back from us.
    session_active::wait_vt_free(u32::from(config.tty));

    // pam_systemd reads these with `getenv_harder()` — PAM environment first,
    // then ours. `greeter` is the class that tells logind (and `loginctl`) this
    // is not a user session.
    //
    // SAFETY: this process is a fresh `fork()` of a single-threaded runner and
    // has spawned nothing yet, so no other thread can be reading the environment.
    unsafe {
        std::env::set_var("XDG_SEAT", "seat0");
        std::env::set_var("XDG_VTNR", config.tty.to_string());
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        std::env::set_var("XDG_SESSION_CLASS", "greeter");
    }

    let conv = SilentConv {
        username: session_user.clone(),
    };
    let mut auth = match Authenticator::with_handler(&config.greeter_pam_service, conv) {
        Ok(auth) => auth,
        Err(err) => {
            error!(
                "greeter session: cannot open PAM service '{}': {err}",
                config.greeter_pam_service
            );
            std::process::exit(EXIT_NO_SESSION);
        }
    };

    // `pam_permit`: nothing is asked, nothing is checked. `open_session` refuses
    // to run without this, and it is where `pam_systemd` lives.
    if let Err(err) = auth.authenticate() {
        error!("greeter session: PAM refused the greeter user: {err}");
        std::process::exit(EXIT_NO_SESSION);
    }
    if let Err(err) = auth.open_session() {
        error!("greeter session: pam_open_session failed: {err}");
        std::process::exit(EXIT_NO_SESSION);
    }

    // logind grants device access asynchronously. margo must not race it to the
    // DRM node — that is the "first login after boot cannot take the device"
    // flake, and it is the same wait the user session does.
    session_active::wait();

    // margo writes its log under `$XDG_STATE_HOME/margo/logs`, defaulting to
    // `$HOME/.local/state`. The greeter user's home is `/`, so that would be an
    // EACCES it silently swallows. Point it somewhere it can actually write —
    // logind just gave us exactly such a directory.
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        // SAFETY: still single-threaded; nothing has been spawned.
        unsafe { std::env::set_var("XDG_STATE_HOME", &runtime_dir) };
    }

    if let Some(user) = greeter.as_ref() {
        info!(
            "greeter session: running the greeter as '{}' (uid {})",
            config.greeter_user, user.uid
        );
        drop_privileges(&mut cmd, user);
    } else {
        info!("greeter session: running the greeter as root");
    }

    // XDG_RUNTIME_DIR and XDG_SESSION_ID were copied into our environment by
    // `open_session`; the greeter inherits them, and margo's socket lands in
    // /run/user/<uid> rather than a root-only tmpfs.
    let status = match cmd.spawn() {
        Ok(mut child) => child.wait(),
        Err(err) => {
            error!("greeter session: cannot spawn the greeter host: {err}");
            // Drop `auth` first so pam_close_session runs.
            drop(auth);
            std::process::exit(EXIT_NO_SESSION);
        }
    };

    let code = match status {
        Ok(status) => status.code().unwrap_or(EXIT_NO_SESSION),
        Err(err) => {
            error!("greeter session: cannot wait for the greeter host: {err}");
            EXIT_NO_SESSION
        }
    };

    // pam_close_session + setcred(DELETE) + pam_end. logind then tears the
    // session down and releases the VT for the user's session.
    drop(auth);
    std::process::exit(code)
}

/// Run the greeter as `user`: supplementary groups, then gid, then uid.
///
/// The order is not stylistic. `setgroups` and `setgid` both need privilege we
/// are about to give away, so `setuid` goes last. This is the same sequence
/// `post_login::lower_command_permissions_to_user` uses for the user's session.
fn drop_privileges(cmd: &mut Command, user: &UserInfo) {
    use nix::unistd::{Gid, Uid};
    use std::os::unix::process::CommandExt;

    let uid = user.uid;
    let gid = user.primary_gid;
    let groups: Vec<Gid> = user.all_gids.iter().copied().map(Gid::from_raw).collect();

    // The greeter has no home to speak of, and no business in the daemon's cwd.
    cmd.current_dir("/");

    // SAFETY: `pre_exec` runs between fork and exec in a single-threaded child;
    // the closure allocates nothing and calls only async-signal-safe syscalls.
    unsafe {
        cmd.pre_exec(move || {
            nix::unistd::setgroups(&groups)
                .and(nix::unistd::setgid(Gid::from_raw(gid)))
                .and(nix::unistd::setuid(Uid::from_raw(uid)))
                .map_err(|err| err.into())
        });
    }
}
