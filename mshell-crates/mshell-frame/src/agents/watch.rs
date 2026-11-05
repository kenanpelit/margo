//! Session status changes worth a desktop notification: a session that
//! starts waiting for you (a permission prompt or a question), and one
//! that finishes a long turn (working → idle). Pure state machine over
//! the polled session list; the poll sends what it returns through
//! `mshell_process::desktop::notify` when the user turned it on.

use super::sessions::{LiveSession, SessionStatus};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionEvent {
    /// `(session key, title, what it waits for)`.
    NeedsYou(String, String, String),
    /// `(session key, title, turn length ms)`.
    Finished(String, String, i64),
}

/// The last status seen per session.
#[derive(Debug, Default)]
pub(crate) struct StatusWatch {
    prev: HashMap<String, (SessionStatus, i64)>,
    primed: bool,
}

/// A turn shorter than this finishing is not news.
pub(crate) const FINISHED_MIN_MS: i64 = 2 * 60_000;

impl StatusWatch {
    /// Compare `live` with the last call. The first call only records:
    /// sessions already waiting when the shell starts are on the bar, not
    /// news. `title` names a session in the notification.
    pub(crate) fn update(
        &mut self,
        live: &[LiveSession],
        title: impl Fn(&LiveSession) -> String,
    ) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        let mut next = HashMap::with_capacity(live.len());
        for s in live {
            let key = s.key();
            if self.primed {
                let before = self.prev.get(&key).copied();
                match (before.map(|b| b.0), s.status) {
                    (b, SessionStatus::Waiting) if b != Some(SessionStatus::Waiting) => {
                        events.push(SessionEvent::NeedsYou(
                            key.clone(),
                            title(s),
                            s.waiting_for.clone(),
                        ));
                    }
                    (Some(SessionStatus::Busy), SessionStatus::Idle) => {
                        let started = before.map_or(s.since_ms, |b| b.1);
                        let turn = s.since_ms - started;
                        if turn >= FINISHED_MIN_MS {
                            events.push(SessionEvent::Finished(key.clone(), title(s), turn));
                        }
                    }
                    _ => {}
                }
            }
            // Keep when the current status began (a busy session's start
            // is what the finished turn is measured from).
            let since = match self.prev.get(&key) {
                Some((st, t)) if *st == s.status => *t,
                _ => s.since_ms,
            };
            next.insert(key, (s.status, since));
        }
        self.prev = next;
        self.primed = true;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::sessions::test_session;

    fn s(id: &str, status: SessionStatus, since: i64) -> LiveSession {
        let mut x = test_session(status, since);
        x.session_id = id.into();
        x.name = format!("n-{id}");
        x
    }

    fn run(w: &mut StatusWatch, v: &[LiveSession]) -> Vec<SessionEvent> {
        w.update(v, |x| x.name.clone())
    }

    #[test]
    fn first_poll_only_records() {
        let mut w = StatusWatch::default();
        assert!(run(&mut w, &[s("a", SessionStatus::Waiting, 0)]).is_empty());
        // Still waiting: no repeat.
        assert!(run(&mut w, &[s("a", SessionStatus::Waiting, 0)]).is_empty());
    }

    #[test]
    fn starting_to_wait_notifies_once() {
        let mut w = StatusWatch::default();
        run(&mut w, &[s("a", SessionStatus::Busy, 0)]);
        let mut waiting = s("a", SessionStatus::Waiting, 10);
        waiting.waiting_for = "approve Bash".into();
        assert_eq!(
            run(&mut w, std::slice::from_ref(&waiting)),
            [SessionEvent::NeedsYou(
                waiting.key(),
                "n-a".into(),
                "approve Bash".into()
            )]
        );
        assert!(run(&mut w, &[waiting]).is_empty());
        // A new session that appears already waiting is news too.
        let b = s("b", SessionStatus::Waiting, 20);
        assert_eq!(run(&mut w, &[b]).len(), 1);
    }

    #[test]
    fn a_long_turn_finishing_notifies_a_short_one_does_not() {
        let mut w = StatusWatch::default();
        run(&mut w, &[s("a", SessionStatus::Idle, 0)]);
        run(&mut w, &[s("a", SessionStatus::Busy, 1_000)]);
        // Busy updates its timestamp while working: the turn still counts
        // from when it started.
        run(&mut w, &[s("a", SessionStatus::Busy, 60_000)]);
        let ev = run(
            &mut w,
            &[s("a", SessionStatus::Idle, 1_000 + FINISHED_MIN_MS)],
        );
        assert_eq!(
            ev,
            [SessionEvent::Finished(
                s("a", SessionStatus::Idle, 0).key(),
                "n-a".into(),
                FINISHED_MIN_MS
            )]
        );
        run(&mut w, &[s("a", SessionStatus::Busy, 500_000)]);
        assert!(run(&mut w, &[s("a", SessionStatus::Idle, 510_000)]).is_empty());
    }
}
