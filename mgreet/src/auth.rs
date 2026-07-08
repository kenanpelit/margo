//! PAM credential pre-flight for the greeter.
//!
//! This is a *pre-flight* only: it authenticates the password so the greeter can
//! show "wrong password" immediately, instead of tearing down the whole greeter
//! compositor and re-greeting on every typo. The mlogind orchestrator re-runs
//! the identical PAM conversation via `try_validate` when it opens the real
//! session — this mirrors that path exactly (same `pam` crate + version, same
//! calls, no `open_session`) so a login that passes here always passes there.

use pam::Authenticator;

/// Authenticate `username`/`password` against `pam_service`. `Ok(())` on success;
/// `Err(message)` — already human-readable — on any failure. Never opens a
/// session: the orchestrator owns the session so it can be the leader PID.
pub fn validate(username: &str, password: &str, pam_service: &str) -> Result<(), String> {
    let mut authenticator = Authenticator::with_password(pam_service)
        .map_err(|_| format!("PAM service '{pam_service}' unavailable"))?;

    authenticator
        .get_handler()
        .set_credentials(username, password);

    authenticator
        .authenticate()
        .map_err(|_| "Invalid login credentials".to_string())?;

    Ok(())
}
