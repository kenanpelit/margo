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

use std::path::Path;
use std::time::{Duration, Instant};

use log::{info, warn};

/// Where logind keeps its session state files.
const SESSIONS: &str = "/run/systemd/sessions";

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
    let path = Path::new(SESSIONS).join(&id);

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

/// The VT a logind session state file claims, if it claims one.
///
/// Sessions that are not on a VT (an SSH login, a `systemd --user` manager)
/// have no `VTNr` at all, which is why this is an `Option` rather than a zero.
pub fn vt_of(contents: &str) -> Option<u32> {
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "VTNr" {
            return value.trim().parse().ok();
        }
    }
    None
}

/// The names of the sessions, among `entries`, that sit on `vtnr`.
///
/// Split from the directory walk so the decision is testable from fixtures.
pub fn sessions_on_vt(entries: &[(String, String)], vtnr: u32) -> Vec<&str> {
    entries
        .iter()
        .filter(|(_, contents)| vt_of(contents) == Some(vtnr))
        .map(|(name, _)| name.as_str())
        .collect()
}

/// Block until no logind session claims `vtnr`, or until [`TIMEOUT`] elapses.
///
/// The greeter session and the user session share one VT — atrium holds a single
/// VT per seat and runs them on it in sequence, and so do we. `pam_close_session`
/// in the greeter-session process starts the teardown, but logind finishes it
/// asynchronously, and `pam_systemd` will not put the user's session on a VT that
/// still has one.
///
/// Like [`wait`], this never fails the login: on timeout it warns and proceeds.
pub fn wait_vt_free(vtnr: u32) {
    let started = Instant::now();
    loop {
        let entries = read_sessions();
        let occupied = sessions_on_vt(&entries, vtnr);
        if occupied.is_empty() {
            info!(
                "runner: VT {vtnr} free after {} ms",
                started.elapsed().as_millis()
            );
            return;
        }

        if started.elapsed() >= TIMEOUT {
            warn!(
                "runner: VT {vtnr} still held by logind session(s) {} after {} ms; continuing",
                occupied.join(", "),
                TIMEOUT.as_millis()
            );
            return;
        }
        std::thread::sleep(POLL);
    }
}

/// `(session id, state file contents)` for every session logind currently knows.
/// An unreadable directory reads as "no sessions" — the caller then proceeds,
/// which is the same thing it would do on a timeout.
fn read_sessions() -> Vec<(String, String)> {
    let Ok(dir) = std::fs::read_dir(SESSIONS) else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let contents = std::fs::read_to_string(entry.path()).ok()?;
            Some((name, contents))
        })
        .collect()
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

    #[test]
    fn a_session_on_a_vt_reports_its_number() {
        assert_eq!(vt_of("UID=0\nVTNr=1\nSTATE=active\n"), Some(1));
        assert_eq!(vt_of("VTNr = 7 \n"), Some(7));
    }

    #[test]
    fn a_session_without_a_vt_has_none() {
        // An SSH login, or the user's `systemd --user` manager.
        assert_eq!(vt_of("UID=1000\nREMOTE=1\n"), None);
        assert_eq!(vt_of(""), None);
    }

    #[test]
    fn a_non_numeric_vt_is_not_a_vt() {
        // Never parse this into a 0 that then matches VT 0.
        assert_eq!(vt_of("VTNr=tty1\n"), None);
        assert_eq!(vt_of("VTNr=\n"), None);
    }

    #[test]
    fn only_the_sessions_on_our_vt_are_reported() {
        let entries = vec![
            ("c1".to_string(), "VTNr=1\nCLASS=greeter\n".to_string()),
            ("c2".to_string(), "VTNr=2\n".to_string()),
            ("c3".to_string(), "REMOTE=1\n".to_string()),
            ("c4".to_string(), "VTNr=1\n".to_string()),
        ];
        assert_eq!(sessions_on_vt(&entries, 1), vec!["c1", "c4"]);
        assert_eq!(sessions_on_vt(&entries, 2), vec!["c2"]);
        assert!(sessions_on_vt(&entries, 3).is_empty());
    }

    #[test]
    fn an_empty_session_list_leaves_every_vt_free() {
        assert!(sessions_on_vt(&[], 1).is_empty());
    }
}
