/// The current user's login name, for the lock screen's greeting.
///
/// `getpwuid` returns NULL on error and when the uid has no passwd entry at
/// all — an unreachable LDAP/SSSD backend, a minimal container. Dereferencing
/// it then segfaults the caller. The two sibling lookups in this workspace
/// (`mshell-auth`'s `pam.rs` and `mlock`'s `auth.rs`) both guard it; this one
/// did not.
///
/// Falls back to `$USER` and then to a placeholder rather than failing, since
/// the only consumer is a label.
pub fn current_username() -> String {
    // SAFETY: `getpwuid` is called with a valid uid and its result is
    // null-checked before any field is read. The returned `passwd` and its
    // `pw_name` are owned by libc and valid until the next passwd-db call,
    // which we do not make before copying the string out.
    let from_passwd = unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_name.is_null() {
            None
        } else {
            Some(
                std::ffi::CStr::from_ptr((*pw).pw_name)
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    };

    from_passwd
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "user".to_string())
}
