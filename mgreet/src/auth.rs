//! PAM credential pre-flight for the greeter, plus the pure submit decision.
//!
//! This is a *pre-flight* only: it authenticates the password so the greeter can
//! show "wrong password" immediately, instead of tearing down the whole greeter
//! compositor and re-greeting on every typo. The mlogind orchestrator re-runs
//! the identical PAM conversation via `try_validate` when it opens the real
//! session — this mirrors that path exactly (same `pam` crate + version, same
//! calls, no `open_session`) so a login that passes here always passes there.

use pam::Authenticator;

/// The credential check behind the login button. The production impl runs the
/// PAM pre-flight; tests inject a fake so the submit decision tree can be driven
/// with no PAM stack. The seam is a wrapper, NOT a reimplementation of the PAM
/// conversation: [`PamAuthenticator::validate`] holds the original body
/// byte-for-byte, so what passes here still passes in the orchestrator.
pub trait Authenticate {
    /// `Ok(())` on success; `Err(message)` — already human-readable — on any
    /// failure. Never opens a session: the orchestrator owns the session so it
    /// can be the leader PID.
    fn validate(&self, username: &str, password: &str, pam_service: &str) -> Result<(), String>;
}

/// The production authenticator: the exact PAM pre-flight, unchanged.
pub struct PamAuthenticator;

impl Authenticate for PamAuthenticator {
    fn validate(&self, username: &str, password: &str, pam_service: &str) -> Result<(), String> {
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
}

/// What one submit attempt resolved to, computed WITHOUT touching GTK so the
/// whole tree — including the auth success/failure branches — is testable
/// against a fake [`Authenticate`]. The GTK caller maps each variant to widget
/// effects (status text, clearing the password, the hand-off, quitting).
#[derive(Debug, PartialEq, Eq)]
pub enum Submit {
    /// Refuse before authenticating: show `.0` as an error, change nothing else.
    Reject(&'static str),
    /// Preview / dry-run (no PAM configured): show `.0`, never authenticate.
    Preview(String),
    /// Authenticated: the caller performs the hand-off, updates the cache, quits.
    Success,
    /// Auth failed: the caller clears the password field and shows `.0`.
    Failure(String),
}

/// Decide what a submit should do. `pam_service == None` is preview/dry-run — no
/// PAM is invoked and an empty session is tolerated, matching the live UI.
///
/// Constraint: the empty-user → preview → empty-session → authenticate ORDER is
/// load-bearing — preview must echo even with no session, and PAM must never be
/// reached without a username — so keep the branches in this sequence.
pub fn decide_submit(
    auth: &dyn Authenticate,
    user: &str,
    password: &str,
    session: &str,
    pam_service: Option<&str>,
) -> Submit {
    if user.is_empty() {
        return Submit::Reject("Enter a username");
    }
    let Some(service) = pam_service else {
        return Submit::Preview(format!("(preview) {user} · {session}"));
    };
    if session.is_empty() {
        return Submit::Reject("No login session available");
    }
    match auth.validate(user, password, service) {
        Ok(()) => Submit::Success,
        Err(msg) => Submit::Failure(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A PAM stand-in: counts calls and returns a scripted result so the decision
    /// tree runs with no PAM stack. Passwords handed to it are test literals and
    /// never reach real PAM.
    struct FakeAuth {
        result: Result<(), String>,
        calls: RefCell<u32>,
    }

    impl FakeAuth {
        fn ok() -> Self {
            Self {
                result: Ok(()),
                calls: RefCell::new(0),
            }
        }
        fn err(msg: &str) -> Self {
            Self {
                result: Err(msg.to_string()),
                calls: RefCell::new(0),
            }
        }
        fn calls(&self) -> u32 {
            *self.calls.borrow()
        }
    }

    impl Authenticate for FakeAuth {
        fn validate(&self, _user: &str, _password: &str, _service: &str) -> Result<(), String> {
            *self.calls.borrow_mut() += 1;
            self.result.clone()
        }
    }

    #[test]
    fn empty_username_rejected_before_pam() {
        let auth = FakeAuth::ok();
        let out = decide_submit(&auth, "", "test-password", "GNOME", Some("login"));
        assert_eq!(out, Submit::Reject("Enter a username"));
        assert_eq!(auth.calls(), 0, "must not reach PAM without a username");
    }

    #[test]
    fn preview_echoes_and_never_authenticates() {
        // A wrong-result fake proves preview never calls it.
        let auth = FakeAuth::err("should not run");
        let out = decide_submit(&auth, "alice", "test-password", "GNOME", None);
        assert_eq!(out, Submit::Preview("(preview) alice · GNOME".to_string()));
        assert_eq!(auth.calls(), 0);
    }

    #[test]
    fn preview_tolerates_a_missing_session() {
        // Preview echoes even with no session picked; the real path rejects that.
        let auth = FakeAuth::ok();
        let out = decide_submit(&auth, "alice", "test-password", "", None);
        assert_eq!(out, Submit::Preview("(preview) alice · ".to_string()));
    }

    #[test]
    fn real_mode_requires_a_session_before_pam() {
        let auth = FakeAuth::ok();
        let out = decide_submit(&auth, "alice", "test-password", "", Some("login"));
        assert_eq!(out, Submit::Reject("No login session available"));
        assert_eq!(auth.calls(), 0);
    }

    #[test]
    fn success_routes_to_the_handoff() {
        let auth = FakeAuth::ok();
        let out = decide_submit(&auth, "alice", "test-password", "GNOME", Some("login"));
        assert_eq!(out, Submit::Success);
        assert_eq!(auth.calls(), 1);
    }

    #[test]
    fn wrong_password_surfaces_the_failure_verbatim() {
        let auth = FakeAuth::err("Invalid login credentials");
        let out = decide_submit(&auth, "alice", "test-password", "GNOME", Some("login"));
        assert_eq!(
            out,
            Submit::Failure("Invalid login credentials".to_string())
        );
    }

    #[test]
    fn account_expired_message_passes_through() {
        // Any PAM error text reaches the UI unchanged; the caller then clears the
        // password field (the Failure contract) rather than tearing the greeter down.
        let auth = FakeAuth::err("Account expired");
        let out = decide_submit(&auth, "alice", "test-password", "GNOME", Some("login"));
        assert_eq!(out, Submit::Failure("Account expired".to_string()));
    }
}
