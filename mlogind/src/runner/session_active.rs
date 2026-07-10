//! Wait for logind to finish activating the session before we open DRM.
//!
//! `pam_open_session` returns as soon as `pam_systemd` has *asked* logind to
//! create the session. Device access — the DRM master the compositor is about
//! to want — is granted asynchronously, a little later. Launching the
//! compositor into that gap is the "first login after boot sometimes can't take
//! the device" class of flake. atrium waits on `sd_session_is_active()` for
//! exactly this reason (`daemon/session/compositor.c`).
//!
//! `sd_session_is_active()` reads `/run/systemd/sessions/<id>`, so we read it
//! too rather than linking `libsystemd` for one boolean.

use std::time::{Duration, Instant};

use log::{info, warn};

/// How long we are willing to wait. Generous: activation is normally a few
/// milliseconds, and the cost of guessing low is a black screen.
const TIMEOUT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);

/// Whether a logind session state file describes an active session.
///
/// `None` means the file carried no state we recognise — a systemd that renamed
/// the key, most likely. Treated as "don't wait" rather than "not active", so a
/// future systemd cannot hang the login.
pub fn parse_active(contents: &str) -> Option<bool> {
    let mut seen = None;
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "STATE" => return Some(value.trim() == "active"),
            // Older systemd wrote ACTIVE=1 alongside STATE. Keep it as a
            // fallback, but let STATE win if both are present.
            "ACTIVE" => seen = Some(value.trim() == "1" || value.trim() == "yes"),
            _ => {}
        }
    }
    seen
}

/// Block until `$XDG_SESSION_ID` is active, or until [`TIMEOUT`] elapses.
///
/// Never fails the login. A timeout logs a warning and launches the compositor
/// anyway: a greeter that refuses to proceed because a wait did not converge is
/// worse than a compositor that retries its DRM open.
pub fn wait() {
    let Some(id) = std::env::var_os("XDG_SESSION_ID") else {
        // No `pam_systemd` in the stack. Nothing to wait for.
        info!("runner: XDG_SESSION_ID unset; not waiting for a logind session");
        return;
    };
    let path = std::path::Path::new("/run/systemd/sessions").join(&id);

    let started = Instant::now();
    loop {
        // The file appears a moment after pam_open_session returns; until then
        // there is simply nothing to read.
        if let Ok(text) = std::fs::read_to_string(&path) {
            match parse_active(&text) {
                Some(true) => {
                    info!(
                        "runner: logind session {} active after {} ms",
                        id.to_string_lossy(),
                        started.elapsed().as_millis()
                    );
                    return;
                }
                Some(false) => {}
                None => {
                    warn!(
                        "runner: {} has no state we recognise; not waiting",
                        path.display()
                    );
                    return;
                }
            }
        }

        if started.elapsed() >= TIMEOUT {
            warn!(
                "runner: logind session {} still not active after {} ms; launching anyway",
                id.to_string_lossy(),
                TIMEOUT.as_millis()
            );
            return;
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_active_session_reads_as_active() {
        let text = "# This is private data\nUID=1000\nUSER=alice\nACTIVE=1\nSTATE=active\n";
        assert_eq!(parse_active(text), Some(true));
    }

    #[test]
    fn an_online_session_is_not_yet_active() {
        // `online` means the user is logged in but the session is in the
        // background — exactly the state we must not launch DRM from.
        assert_eq!(parse_active("STATE=online\n"), Some(false));
    }

    #[test]
    fn a_closing_session_is_not_active() {
        assert_eq!(parse_active("STATE=closing\n"), Some(false));
    }

    #[test]
    fn state_wins_over_a_stale_active_key() {
        assert_eq!(parse_active("ACTIVE=1\nSTATE=online\n"), Some(false));
    }

    #[test]
    fn the_legacy_active_key_is_honoured_when_state_is_absent() {
        assert_eq!(parse_active("UID=1000\nACTIVE=1\n"), Some(true));
        assert_eq!(parse_active("UID=1000\nACTIVE=0\n"), Some(false));
    }

    #[test]
    fn an_unrecognised_file_says_dont_wait_rather_than_not_active() {
        // A future systemd renaming the key must not hang the login.
        assert_eq!(parse_active("UID=1000\nUSER=alice\n"), None);
        assert_eq!(parse_active(""), None);
    }

    #[test]
    fn whitespace_and_comments_do_not_confuse_the_parse() {
        assert_eq!(parse_active("  STATE = active \n"), Some(true));
        assert_eq!(parse_active("#STATE=active\nSTATE=online\n"), Some(false));
    }
}
