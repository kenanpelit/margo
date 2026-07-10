//! The greeter's half of the conversation, as pure decisions.
//!
//! `mgreet` no longer runs PAM. It used to run a full `pam_authenticate` as a
//! pre-flight and then hand the plaintext to the orchestrator through a file,
//! which meant PAM ran twice per login: a fingerprint reader prompted twice, a
//! U2F key wanted two taps, and a single-use OTP could not work at all.
//!
//! Now the session runner owns PAM and asks us questions. Everything here is a
//! pure function over `(what the user typed, what the runner said)`, so the
//! whole tree is unit-tested with no PAM stack, no socket and no GTK.

use mlogind_proto::Event;

/// What one press of Enter (or the login button) should do.
#[derive(Debug, PartialEq, Eq)]
pub enum Submit {
    /// Refuse before troubling the runner: show `.0` as an error.
    Reject(&'static str),
    /// Preview / dry-run (no socket): show `.0`, talk to nobody.
    Preview(String),
    /// Open a conversation: send `Begin { user, session }`.
    Begin,
    /// The runner is holding a prompt open. Send what is in the field.
    Answer,
    /// A conversation is already in flight and PAM has not asked anything yet.
    /// Do nothing — a second `Begin` would arrive while the runner sits inside
    /// `pam_authenticate`, which aborts the attempt it is already running.
    Busy,
}

/// Decide what a submit should do.
///
/// `real` is "we have a socket to a session runner". `awaiting_prompt` is "the
/// runner asked something and the field now holds the answer" — and it wins over
/// everything, because a prompt mid-conversation is not a fresh login and the
/// username/session fields have nothing to say about it. `conversing` is "a
/// `Begin` is in flight"; a second one, sent while the runner is inside
/// `pam_authenticate`, would land in its conversation callback and abort the
/// attempt already running. Pressing Enter twice is not exotic.
///
/// The remaining order is load-bearing: preview must echo even with no session
/// picked, and we must never send a `Begin` without a username.
pub fn decide_submit(
    user: &str,
    session: &str,
    awaiting_prompt: bool,
    conversing: bool,
    real: bool,
) -> Submit {
    if awaiting_prompt {
        return Submit::Answer;
    }
    if conversing {
        return Submit::Busy;
    }
    if user.is_empty() {
        return Submit::Reject("Enter a username");
    }
    if !real {
        return Submit::Preview(format!("(preview) {user} · {session}"));
    }
    if session.is_empty() {
        return Submit::Reject("No login session available");
    }
    Submit::Begin
}

/// What the window should do about one event from the runner.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Answer this prompt with the password already typed. No user interaction.
    AnswerWithPassword,
    /// A question the form cannot answer. Show `.0`, clear the field, focus it.
    AskUser(String),
    /// `PAM_TEXT_INFO`: show it, keep waiting.
    Note(String),
    /// `PAM_ERROR_MSG`: show it as an error, keep waiting — PAM has not given up.
    Warn(String),
    /// Authenticated. Quit so the greeter compositor exits and the runner takes over.
    Done,
    /// This attempt failed. Clear the password, show `.0`, back to a blank form.
    Failed(String),
}

/// Map one runner event onto a window action.
///
/// `password_pending` is whether we still hold the password the user typed at
/// submit. PAM's first *blind* prompt is the password prompt — the runner
/// answers the username prompt itself, from `Begin`, so it never reaches us.
/// Everything after that is a real question and goes back to the user.
pub fn decide_event(event: Event, password_pending: bool) -> Action {
    match event {
        Event::Prompt { echo, text } => {
            if !echo && password_pending {
                Action::AnswerWithPassword
            } else {
                Action::AskUser(text)
            }
        }
        Event::Info { text } => Action::Note(text),
        Event::Error { text } => Action::Warn(text),
        Event::Success => Action::Done,
        Event::Failure { reason } => Action::Failed(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_username_is_rejected_before_the_runner_hears_anything() {
        assert_eq!(
            decide_submit("", "margo", false, false, true),
            Submit::Reject("Enter a username")
        );
    }

    #[test]
    fn preview_echoes_and_opens_no_conversation() {
        assert_eq!(
            decide_submit("alice", "margo", false, false, false),
            Submit::Preview("(preview) alice · margo".to_string())
        );
    }

    #[test]
    fn preview_tolerates_a_missing_session() {
        assert_eq!(
            decide_submit("alice", "", false, false, false),
            Submit::Preview("(preview) alice · ".to_string())
        );
    }

    #[test]
    fn real_mode_requires_a_session_before_begin() {
        assert_eq!(
            decide_submit("alice", "", false, false, true),
            Submit::Reject("No login session available")
        );
    }

    #[test]
    fn a_filled_form_opens_a_conversation() {
        assert_eq!(
            decide_submit("alice", "margo", false, false, true),
            Submit::Begin
        );
    }

    #[test]
    fn a_pending_prompt_beats_every_other_check() {
        // Mid-conversation the username field may be empty, the session unset,
        // and it does not matter: the field holds an answer PAM is waiting for.
        assert_eq!(decide_submit("", "", true, true, true), Submit::Answer);
        assert_eq!(
            decide_submit("alice", "margo", true, true, true),
            Submit::Answer
        );
    }

    #[test]
    fn a_second_enter_while_begin_is_in_flight_does_nothing() {
        // Enter twice, fast. The runner is inside pam_authenticate; a second
        // Begin would reach its conversation callback and kill the attempt.
        assert_eq!(
            decide_submit("alice", "margo", false, true, true),
            Submit::Busy
        );
    }

    #[test]
    fn an_answer_still_wins_over_busy() {
        // A prompt is open: `conversing` is true, but the field holds its answer.
        assert_eq!(
            decide_submit("alice", "margo", true, true, true),
            Submit::Answer
        );
    }

    #[test]
    fn the_first_blind_prompt_is_answered_from_the_password_field() {
        let event = Event::Prompt {
            echo: false,
            text: "Password:".into(),
        };
        assert_eq!(decide_event(event, true), Action::AnswerWithPassword);
    }

    #[test]
    fn a_second_blind_prompt_goes_back_to_the_user() {
        // The password was spent on the first one. "New password:" is a real
        // question — the whole reason A1 exists.
        let event = Event::Prompt {
            echo: false,
            text: "New password:".into(),
        };
        assert_eq!(
            decide_event(event, false),
            Action::AskUser("New password:".into())
        );
    }

    #[test]
    fn an_echo_prompt_always_goes_back_to_the_user() {
        // Even holding a password: an echo prompt is not asking for one.
        let event = Event::Prompt {
            echo: true,
            text: "OTP token:".into(),
        };
        assert_eq!(
            decide_event(event, true),
            Action::AskUser("OTP token:".into())
        );
    }

    #[test]
    fn info_and_error_are_shown_without_ending_the_conversation() {
        assert_eq!(
            decide_event(
                Event::Info {
                    text: "Touch your key".into()
                },
                true
            ),
            Action::Note("Touch your key".into())
        );
        assert_eq!(
            decide_event(
                Event::Error {
                    text: "Fingerprint not recognised".into()
                },
                true
            ),
            Action::Warn("Fingerprint not recognised".into())
        );
    }

    #[test]
    fn success_quits_and_failure_returns_to_the_form() {
        assert_eq!(decide_event(Event::Success, false), Action::Done);
        assert_eq!(
            decide_event(
                Event::Failure {
                    reason: "Invalid login credentials".into()
                },
                false
            ),
            Action::Failed("Invalid login credentials".into())
        );
    }

    #[test]
    fn a_failure_reason_reaches_the_ui_verbatim() {
        // The runner decides what the user is told. Whether the account exists
        // is not the greeter's business.
        assert_eq!(
            decide_event(
                Event::Failure {
                    reason: "Account expired".into()
                },
                false
            ),
            Action::Failed("Account expired".into())
        );
    }
}
