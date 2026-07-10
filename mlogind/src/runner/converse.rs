//! The PAM conversation, answered by whichever greeter is on the other end of
//! the socket.
//!
//! `pam::PasswordConv` replays one fixed username/password for every question
//! PAM asks. That is why a fingerprint reader prompts twice, why U2F asks for
//! two taps, why OTP cannot work at all, and why "your password has expired,
//! enter a new one" has no answer. Here every `pam_conv` callback becomes a
//! round trip to the greeter, so PAM may ask whatever it likes, as often as it
//! likes.

use std::ffi::{CStr, CString};

use log::{info, warn};
use mlogind_proto::{Conn, Event, ProtoError, Request, Transport};

/// Why a conversation stopped before PAM reached a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abort {
    /// The greeter asked to start over.
    Cancelled,
    /// The greeter closed the socket — it quit, or it crashed.
    Eof,
    /// The socket broke, or the greeter spoke nonsense.
    Broken,
}

/// A [`pam::Converse`] that forwards every prompt to the greeter.
///
/// Borrows the connection rather than owning it: `Authenticator::with_handler`
/// takes its `Converse` by value, so an owned socket would be swallowed by the
/// first failed login attempt. The runner keeps the fd and lends it out per
/// `pam_start`.
pub struct GreeterConv<'a, T: Transport> {
    conn: &'a mut Conn<T>,
    /// The login name from `Begin`. Also what `Authenticator::open_session`
    /// reads back via [`pam::Converse::username`] to look the account up.
    username: String,
    /// PAM asks for the login name with an echo prompt, because the `pam` crate
    /// calls `pam_start(service, None, …)`. We already know it, so the *first*
    /// echo prompt is answered locally and costs no round trip. Any later one
    /// is a real question (a second factor's identity, say) and is forwarded.
    username_answered: bool,
    /// Set once the conversation can no longer make progress.
    abort: Option<Abort>,
    /// A `Begin` that arrived while PAM was mid-prompt. The greeter is not
    /// supposed to do this — it is blocked on our prompt — but if it does, the
    /// request is not dropped on the floor: the runner picks it up as the next
    /// attempt instead of blocking on a socket the greeter is no longer driving.
    pending_begin: Option<(String, String)>,
}

impl<'a, T: Transport> GreeterConv<'a, T> {
    pub fn new(conn: &'a mut Conn<T>, username: String) -> Self {
        Self {
            conn,
            username,
            username_answered: false,
            abort: None,
            pending_begin: None,
        }
    }

    pub fn abort(&self) -> Option<Abort> {
        self.abort
    }

    /// A `Begin` the greeter sent out of turn, if any. Consumed by the runner.
    pub fn take_pending_begin(&mut self) -> Option<(String, String)> {
        self.pending_begin.take()
    }

    pub fn send_success(&mut self) {
        if let Err(err) = self.conn.send_event(&Event::Success) {
            warn!("runner: could not tell the greeter it succeeded: {err}");
        }
    }

    pub fn send_failure(&mut self, reason: &str) {
        if let Err(err) = self.conn.send_event(&Event::Failure {
            reason: reason.to_owned(),
        }) {
            warn!("runner: could not tell the greeter it failed: {err}");
        }
    }

    /// Block until the greeter closes its end.
    ///
    /// The TTY host has no greeter *process* to reap — the greeter is the
    /// parent, drawing over the very VT the session compositor is about to
    /// claim. EOF on this socket is the parent saying it has left the alternate
    /// screen. Waiting for it is what stops the compositor from opening DRM
    /// underneath a live ratatui frame.
    pub fn wait_for_eof(&mut self) {
        loop {
            match self.conn.recv_request() {
                Ok(None) => return,
                Ok(Some(_)) => {} // a late Cancel, say. Ignore it; we are past that.
                Err(err) => {
                    warn!("runner: error while waiting for the greeter to exit: {err}");
                    return;
                }
            }
        }
    }

    /// Ask the greeter one question and block for the answer.
    fn ask(&mut self, echo: bool, msg: &CStr) -> Result<CString, ()> {
        if self.abort.is_some() {
            return Err(());
        }

        let text = msg.to_string_lossy().into_owned();
        if let Err(err) = self.conn.send_event(&Event::Prompt { echo, text }) {
            self.give_up(err);
            return Err(());
        }

        // Exactly one frame answers a prompt. Anything else ends the attempt.
        match self.conn.recv_request() {
            Ok(Some(Request::Response { secret })) => {
                // PAM's C API cannot carry an interior NUL, so a response
                // holding one is unanswerable rather than silently truncated.
                //
                // The CString we hand back is freed by libpam, not by us —
                // there is no hook to scrub it. Our own copy dies with
                // `secret`, which is `Zeroizing`.
                CString::new(secret.to_vec()).map_err(|_| {
                    warn!("runner: greeter sent a response containing a NUL byte");
                })
            }
            Ok(Some(Request::Cancel)) => {
                info!("runner: greeter cancelled the conversation");
                self.abort = Some(Abort::Cancelled);
                Err(())
            }
            Ok(Some(Request::Begin { user, session })) => {
                // Out of turn. Keep it for the next attempt rather than
                // deadlocking on a prompt nobody is listening to.
                warn!("runner: greeter restarted the form mid-prompt");
                self.pending_begin = Some((user, session));
                self.abort = Some(Abort::Cancelled);
                Err(())
            }
            Ok(None) => {
                info!("runner: greeter closed the socket mid-prompt");
                self.abort = Some(Abort::Eof);
                Err(())
            }
            Err(err) => {
                self.give_up(err);
                Err(())
            }
        }
    }

    fn give_up(&mut self, err: ProtoError) {
        warn!("runner: conversation broke: {err}");
        self.abort = Some(Abort::Broken);
    }
}

impl<T: Transport> pam::Converse for GreeterConv<'_, T> {
    fn prompt_echo(&mut self, msg: &CStr) -> Result<CString, ()> {
        if !self.username_answered {
            self.username_answered = true;
            return CString::new(self.username.clone()).map_err(|_| ());
        }
        self.ask(true, msg)
    }

    fn prompt_blind(&mut self, msg: &CStr) -> Result<CString, ()> {
        self.ask(false, msg)
    }

    fn info(&mut self, msg: &CStr) {
        let text = msg.to_string_lossy().into_owned();
        let _ = self.conn.send_event(&Event::Info { text });
    }

    fn error(&mut self, msg: &CStr) {
        let text = msg.to_string_lossy().into_owned();
        let _ = self.conn.send_event(&Event::Error { text });
    }

    fn username(&self) -> &str {
        &self.username
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlogind_proto::{decode_event, encode_request, MemTransport};
    use pam::Converse;
    use zeroize::Zeroizing;

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// A conversation driven from memory: the greeter's replies are scripted,
    /// its view of our events is recorded. No PAM, no socket, no root.
    fn conv_over(replies: Vec<Request>) -> Conn<MemTransport> {
        let frames = replies
            .iter()
            .map(|r| encode_request(r).expect("encode").to_vec())
            .collect();
        Conn::new(MemTransport::with_incoming(frames))
    }

    fn events(conn: &mut Conn<MemTransport>) -> Vec<Event> {
        conn.get_mut()
            .sent
            .iter()
            .map(|f| decode_event(f).expect("decode"))
            .collect()
    }

    #[test]
    fn the_first_echo_prompt_is_answered_from_begin_without_a_round_trip() {
        let mut conn = conv_over(vec![]);
        let mut conv = GreeterConv::new(&mut conn, "alice".into());

        assert_eq!(conv.prompt_echo(&c("login:")), Ok(c("alice")));
        // Nothing was asked of the greeter.
        assert!(conn.get_mut().sent.is_empty());
    }

    #[test]
    fn a_second_echo_prompt_is_a_real_question_and_is_forwarded() {
        let mut conn = conv_over(vec![Request::Response {
            secret: Zeroizing::new(b"123456".to_vec()),
        }]);
        let mut conv = GreeterConv::new(&mut conn, "alice".into());

        assert_eq!(conv.prompt_echo(&c("login:")), Ok(c("alice")));
        assert_eq!(conv.prompt_echo(&c("OTP token:")), Ok(c("123456")));

        assert_eq!(
            events(&mut conn),
            vec![Event::Prompt {
                echo: true,
                text: "OTP token:".into()
            }]
        );
    }

    #[test]
    fn a_blind_prompt_forwards_with_echo_off() {
        let mut conn = conv_over(vec![Request::Response {
            secret: Zeroizing::new(b"hunter2".to_vec()),
        }]);
        let mut conv = GreeterConv::new(&mut conn, "alice".into());

        assert_eq!(conv.prompt_blind(&c("Password:")), Ok(c("hunter2")));
        assert_eq!(
            events(&mut conn),
            vec![Event::Prompt {
                echo: false,
                text: "Password:".into()
            }]
        );
    }

    #[test]
    fn multi_step_pam_is_possible_at_all() {
        // The whole point of A1: password, then a fingerprint touch, then an
        // expired-password change — three questions, three answers, one PAM run.
        let mut conn = conv_over(vec![
            Request::Response {
                secret: Zeroizing::new(b"old".to_vec()),
            },
            Request::Response {
                secret: Zeroizing::new(b"new".to_vec()),
            },
        ]);
        let mut conv = GreeterConv::new(&mut conn, "alice".into());

        assert_eq!(conv.prompt_blind(&c("Current password:")), Ok(c("old")));
        conv.info(&c("Touch your security key"));
        assert_eq!(conv.prompt_blind(&c("New password:")), Ok(c("new")));

        assert_eq!(
            events(&mut conn),
            vec![
                Event::Prompt {
                    echo: false,
                    text: "Current password:".into()
                },
                Event::Info {
                    text: "Touch your security key".into()
                },
                Event::Prompt {
                    echo: false,
                    text: "New password:".into()
                },
            ]
        );
    }

    #[test]
    fn info_and_error_reach_the_greeter_verbatim() {
        let mut conn = conv_over(vec![]);
        let mut conv = GreeterConv::new(&mut conn, "alice".into());

        conv.info(&c("Place your finger"));
        conv.error(&c("Fingerprint not recognised"));

        assert_eq!(
            events(&mut conn),
            vec![
                Event::Info {
                    text: "Place your finger".into()
                },
                Event::Error {
                    text: "Fingerprint not recognised".into()
                },
            ]
        );
    }

    #[test]
    fn a_greeter_that_disappears_mid_prompt_aborts_the_conversation() {
        let mut conn = conv_over(vec![]); // empty queue == EOF
        let mut conv = GreeterConv::new(&mut conn, "alice".into());

        assert_eq!(conv.prompt_blind(&c("Password:")), Err(()));
        assert_eq!(conv.abort(), Some(Abort::Eof));
    }

    #[test]
    fn a_cancel_aborts_the_conversation() {
        let mut conn = conv_over(vec![Request::Cancel]);
        let mut conv = GreeterConv::new(&mut conn, "alice".into());

        assert_eq!(conv.prompt_blind(&c("Password:")), Err(()));
        assert_eq!(conv.abort(), Some(Abort::Cancelled));
        assert!(conv.take_pending_begin().is_none());
    }

    #[test]
    fn a_begin_sent_out_of_turn_is_kept_for_the_next_attempt() {
        let mut conn = conv_over(vec![Request::Begin {
            user: "bob".into(),
            session: "margo".into(),
        }]);
        let mut conv = GreeterConv::new(&mut conn, "alice".into());

        assert_eq!(conv.prompt_blind(&c("Password:")), Err(()));
        assert_eq!(conv.abort(), Some(Abort::Cancelled));
        assert_eq!(
            conv.take_pending_begin(),
            Some(("bob".into(), "margo".into()))
        );
    }

    #[test]
    fn a_response_with_an_interior_nul_is_refused_not_truncated() {
        let mut conn = conv_over(vec![Request::Response {
            secret: Zeroizing::new(b"hun\0ter2".to_vec()),
        }]);
        let mut conv = GreeterConv::new(&mut conn, "alice".into());

        // PAM's C API cannot carry it. Better an auth failure than logging in
        // with a silently shortened password.
        assert_eq!(conv.prompt_blind(&c("Password:")), Err(()));
    }

    #[test]
    fn once_aborted_no_further_prompt_reaches_the_greeter() {
        let mut conn = conv_over(vec![Request::Cancel]);
        let mut conv = GreeterConv::new(&mut conn, "alice".into());

        assert_eq!(conv.prompt_blind(&c("Password:")), Err(()));
        let sent = conn.get_mut().sent.len();

        let mut conv = GreeterConv::new(&mut conn, "alice".into());
        conv.abort = Some(Abort::Eof);
        assert_eq!(conv.prompt_blind(&c("Password again:")), Err(()));
        assert_eq!(
            conn.get_mut().sent.len(),
            sent,
            "an aborted conversation must not keep prompting"
        );
    }

    #[test]
    fn username_is_what_open_session_will_look_up() {
        let mut conn = conv_over(vec![]);
        let conv = GreeterConv::new(&mut conn, "alice".into());
        assert_eq!(Converse::username(&conv), "alice");
    }
}
