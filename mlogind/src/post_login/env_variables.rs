use log::{error, info};

use super::PostLoginEnvironment;

/// Set one environment variable.
///
/// mlogind is single-threaded — the same documented invariant every bare
/// `fork()` in this crate already rests on — so mutating the environment
/// cannot race another thread reading it. Edition 2024 makes that argument
/// explicit; these two wrappers keep it in one place instead of at forty
/// call sites.
fn set_env(key: &str, value: &str) {
    // SAFETY: single-threaded process; no concurrent getenv can observe a
    // half-written environment.
    unsafe { std::env::set_var(key, value) };
}

/// Remove one environment variable. See [`set_env`] for the safety argument.
fn remove_env(key: &str) {
    // SAFETY: see `set_env`.
    unsafe { std::env::remove_var(key) };
}

pub fn set_display(display: &str) {
    info!("Setting Display");

    set_env("DISPLAY", display);
}

pub fn remove_xdg() {
    info!("Clearing XDG preemptively to set later");

    remove_env("XDG_SESSION_CLASS");
    remove_env("XDG_CURRENT_DESKTOP");
    remove_env("XDG_SESSION_DESKTOP");

    remove_env("XDG_SEAT");
    remove_env("XDG_VTNR");

    remove_env("XDG_RUNTIME_DIR");
    remove_env("XDG_SESSION_ID");

    remove_env("XDG_CONFIG_DIR");
    remove_env("XDG_CACHE_HOME");
    remove_env("XDG_DATA_HOME");
    remove_env("XDG_STATE_HOME");
    remove_env("XDG_DATA_DIRS");
    remove_env("XDG_CONFIG_DIRS");
}

pub fn set_session_params(post_login_env: &PostLoginEnvironment) {
    info!("Setting XDG Session Parameters");

    set_env("XDG_SESSION_CLASS", "user");
    set_env("XDG_SESSION_TYPE", post_login_env.to_xdg_type());

    // TODO: Implement
    // process_env.set("XDG_CURRENT_DESKTOP", post_login_env.to_xdg_desktop());
    // process_env.set("XDG_SESSION_DESKTOP", post_login_env.to_xdg_desktop());
}

pub fn set_or_own_env(key: &'static str, value: &str) {
    if std::env::var(key) == Err(std::env::VarError::NotPresent) {
        set_env(key, value);
    }
}

pub fn set_seat_vars(tty: u8) {
    info!("Setting XDG Seat Variables");

    set_or_own_env("XDG_SEAT", "seat0");
    set_or_own_env("XDG_VTNR", &tty.to_string());
}

// NOTE: This uid: u32 might be better set to libc::uid_t
/// Set the XDG environment variables
pub fn set_session_vars(uid: u32) {
    info!("Setting XDG Session Variables");

    set_or_own_env("XDG_RUNTIME_DIR", &format!("/run/user/{uid}"));
    set_or_own_env("XDG_SESSION_ID", "1");
}

/// Set all the environment variables
pub fn set_basic_variables(username: &str, homedir: &str, shell: &str, path: &str) {
    info!("Setting Basic Environment Variables");

    let pwd = homedir;
    if std::env::set_current_dir(pwd).is_err() {
        error!("Failed to set current working directory.");
    }

    set_env("HOME", homedir);
    set_env("SHELL", shell);
    set_env("USER", username);
    set_env("LOGNAME", username);
    set_env("PATH", path);

    // process_env.set("MAIL", "..."); TODO: Add
}

pub fn set_xdg_common_paths(homedir: &str) {
    info!("Setting XDG Common Paths");

    // This is according to https://wiki.archlinux.org/title/XDG_Base_Directory
    set_or_own_env("XDG_CONFIG_HOME", &format!("{homedir}/.config"));
    set_or_own_env("XDG_CACHE_HOME", &format!("{homedir}/.cache"));
    set_or_own_env("XDG_DATA_HOME", &format!("{homedir}/.local/share"));
    set_or_own_env("XDG_STATE_HOME", &format!("{homedir}/.local/state"));
    set_or_own_env("XDG_DATA_DIRS", "/usr/local/share:/usr/share");
    set_or_own_env("XDG_CONFIG_DIRS", "/etc/xdg");
}
