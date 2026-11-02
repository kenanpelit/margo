use std::collections::HashMap;
use std::ffi::OsString;

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

/// Fallbacks for what `pam_systemd` normally provides. Only the runtime dir:
/// `XDG_SESSION_ID` used to default to `"1"`, but a made-up id names some
/// other session (or none), and `loginctl`, polkit agents and screen lockers
/// that trust it then act on the wrong one. Without pam_systemd it stays unset.
fn session_var_fallbacks(uid: u32) -> [(&'static str, String); 1] {
    [("XDG_RUNTIME_DIR", format!("/run/user/{uid}"))]
}

// NOTE: This uid: u32 might be better set to libc::uid_t
/// Set the XDG environment variables
pub fn set_session_vars(uid: u32) {
    info!("Setting XDG Session Variables");

    for (key, value) in session_var_fallbacks(uid) {
        set_or_own_env(key, &value);
    }
}

/// Inherited from the daemon as they are: terminal and locale. Plus every
/// `LC_*`.
const PASSTHROUGH: &[&str] = &["TERM", "LANG", "LANGUAGE", "TZ"];
/// What the `set_*` functions in this file manage for the session; kept even
/// when the value happens to equal the daemon's.
const MANAGED: &[&str] = &[
    "DISPLAY",
    "XDG_SESSION_CLASS",
    "XDG_SESSION_TYPE",
    "XDG_SEAT",
    "XDG_VTNR",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_ID",
    "HOME",
    "SHELL",
    "USER",
    "LOGNAME",
    "PATH",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_DATA_DIRS",
    "XDG_CONFIG_DIRS",
];

/// The daemon's environment, taken before anything is set for the session.
pub type EnvSnapshot = HashMap<OsString, OsString>;

pub fn snapshot_env() -> EnvSnapshot {
    std::env::vars_os().collect()
}

/// The environment the user's session starts with.
///
/// The session command used to inherit the root daemon's whole environment:
/// systemd's `NOTIFY_SOCKET`, `INVOCATION_ID` and `JOURNAL_STREAM`, whatever
/// the unit or an admin's drop-in exported, the `MLOGIND_*` plumbing. Now it
/// gets what mlogind set for it ([`MANAGED`]), the terminal and locale
/// ([`PASSTHROUGH`]), and everything that is new or changed since `before` —
/// which is exactly what `pam_open_session` copied in from the PAM
/// environment (`pam_env`, `pam_systemd`, …), since the pam crate exposes no
/// other way to read it.
pub fn session_env(
    before: &EnvSnapshot,
    now: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = now
        .into_iter()
        .filter(|(key, value)| {
            let name = key.to_str().unwrap_or_default();
            MANAGED.contains(&name)
                || PASSTHROUGH.contains(&name)
                || name.starts_with("LC_")
                || before.get(key) != Some(value)
        })
        .collect();
    env.sort();
    env
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

#[cfg(test)]
mod tests {
    use super::*;

    fn os(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }

    fn keys(env: &[(OsString, OsString)]) -> Vec<&str> {
        env.iter().map(|(k, _)| k.to_str().unwrap()).collect()
    }

    #[test]
    fn the_session_does_not_inherit_the_daemons_plumbing() {
        let daemon = os(&[
            ("NOTIFY_SOCKET", "/run/systemd/notify"),
            ("INVOCATION_ID", "abc"),
            ("JOURNAL_STREAM", "8:1234"),
            ("MLOGIND_CACHE_PATH", "/var/cache/mlogind"),
            ("LANG", "tr_TR.UTF-8"),
            ("LC_TIME", "en_GB.UTF-8"),
            ("PATH", "/usr/bin"),
        ]);
        let before: EnvSnapshot = daemon.iter().cloned().collect();
        let mut now = daemon.clone();
        // Set by mlogind (same PATH as the daemon's) and by PAM.
        now.extend(os(&[
            ("HOME", "/home/u"),
            ("XDG_SESSION_ID", "c7"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("GNUPGHOME", "/home/u/.gnupg"),
        ]));
        let env = session_env(&before, now);
        assert_eq!(
            keys(&env),
            vec![
                "GNUPGHOME",
                "HOME",
                "LANG",
                "LC_TIME",
                "PATH",
                "XDG_RUNTIME_DIR",
                "XDG_SESSION_ID"
            ]
        );
    }

    #[test]
    fn a_daemon_variable_pam_overrode_is_kept_with_the_new_value() {
        let before: EnvSnapshot = os(&[("EDITOR", "nano")]).into_iter().collect();
        let env = session_env(&before, os(&[("EDITOR", "vim")]));
        assert_eq!(env, os(&[("EDITOR", "vim")]));
    }

    #[test]
    fn no_session_id_is_invented() {
        let fallbacks = session_var_fallbacks(1000);
        assert!(fallbacks.iter().all(|(k, _)| *k != "XDG_SESSION_ID"));
        assert!(
            fallbacks
                .iter()
                .any(|(k, v)| *k == "XDG_RUNTIME_DIR" && v == "/run/user/1000")
        );
    }
}
