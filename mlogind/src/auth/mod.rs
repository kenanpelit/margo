//! Account lookup.
//!
//! PAM itself lives in [`crate::runner`], which owns the whole conversation in
//! one process. What remains here is the passwd-database side: the uid, groups,
//! home and shell the runner needs to drop privileges and exec the compositor.

pub mod utmpx;

use std::fmt;

use log::info;
use uzers::os::unix::UserExt;

/// Everything the runner needs about the account it just authenticated.
///
/// Plain data: no lifetime, no PAM handle. The handle used to live in here and
/// travel across a `fork()`, kept alive by a `std::mem::forget` on the parent
/// side to dodge a double free. The runner now holds its `Authenticator`
/// directly, for the whole session, in the process that opened it.
pub struct UserInfo {
    pub uid: libc::uid_t,
    pub primary_gid: libc::gid_t,
    pub all_gids: Vec<libc::gid_t>,
    pub home_dir: String,
    pub shell: String,
}

#[derive(Clone, Debug)]
pub enum AuthenticationError {
    HomeDirInvalidUtf8,
    ShellInvalidUtf8,
    UsernameNotFound,
}

impl fmt::Display for AuthenticationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeDirInvalidUtf8 => {
                f.write_str("user home directory path contains invalid UTF-8")
            }
            Self::ShellInvalidUtf8 => f.write_str("user shell path contains invalid UTF-8"),
            Self::UsernameNotFound => {
                f.write_str("credentials are valid, but the username is not in the passwd database")
            }
        }
    }
}

impl std::error::Error for AuthenticationError {}

/// Resolve an authenticated username against the passwd database.
///
/// Called *before* `pam_open_session`, whose `initialize_environment` panics on
/// a user it cannot find. Better to fail here, with an error the runner can log.
pub fn lookup(username: &str) -> Result<UserInfo, AuthenticationError> {
    info!("Looking up account '{username}'");

    let user = uzers::get_user_by_name(username).ok_or(AuthenticationError::UsernameNotFound)?;

    Ok(UserInfo {
        uid: user.uid(),
        primary_gid: user.primary_group_id(),
        all_gids: user.groups().map_or_else(Vec::default, |groups| {
            groups.into_iter().map(|group| group.gid()).collect()
        }),
        home_dir: user
            .home_dir()
            .to_str()
            .ok_or(AuthenticationError::HomeDirInvalidUtf8)?
            .to_string(),
        shell: user
            .shell()
            .to_str()
            .ok_or(AuthenticationError::ShellInvalidUtf8)?
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_always_resolves() {
        // Every passwd database has uid 0. This pins the shape of the lookup
        // without needing a fixture user.
        let info = lookup("root").expect("root must exist");
        assert_eq!(info.uid, 0);
        assert!(!info.shell.is_empty());
        assert!(!info.home_dir.is_empty());
    }

    #[test]
    fn an_unknown_user_is_an_error_not_a_panic() {
        // `pam_open_session`'s own environment setup would panic here, which is
        // why the runner calls this first.
        assert!(matches!(
            lookup("\u{1}definitely-not-a-user"),
            Err(AuthenticationError::UsernameNotFound)
        ));
    }
}
