//! The screen-share chooser's requests, one on screen at a time.
//!
//! Every `Screenshare` D-Bus call (margo-portal's ScreenCast / interactive
//! Screenshot, mshellshare) hands the chooser a reply channel. The chooser
//! used to keep only the latest one: a second request arriving while the
//! user was choosing for a meeting app took over the reply, so the monitor
//! or window the user picked went to the second app and the first got an
//! error. Now requests wait in arrival order and each answer goes to the
//! request whose payload is on screen.
//!
//! A request that is shown and then dismissed (click outside, another menu
//! opened) is answered with a cancel, so it never blocks the ones behind
//! it. Hiding the menu while the monitor/area overlay runs is not a dismiss.
//!
//! GTK-free so the ordering rules are unit tested.

use std::collections::VecDeque;
use tokio::sync::oneshot;

/// Requests allowed to wait behind the one on screen. More than this from
/// one session is a flood, not a user: the extra ones are cancelled.
const MAX_WAITING: usize = 4;

struct Request {
    reply: oneshot::Sender<String>,
    payload: String,
}

struct Current {
    reply: oneshot::Sender<String>,
    /// The menu has been revealed with this request's payload.
    shown: bool,
    /// The monitor/area overlay is running for it (the menu is hidden on
    /// purpose).
    selecting: bool,
}

/// What the widget does with a new request.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pushed {
    /// It is now the current request: show this payload and open the menu.
    Show(String),
    /// It waits behind the current one.
    Queued,
    /// Too many waiting; it was answered with a cancel.
    Refused,
}

#[derive(Default)]
pub(crate) struct PickerQueue {
    current: Option<Current>,
    waiting: VecDeque<Request>,
}

impl std::fmt::Debug for PickerQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PickerQueue")
            .field("current", &self.current.is_some())
            .field("waiting", &self.waiting.len())
            .finish()
    }
}

impl PickerQueue {
    pub(crate) fn push(&mut self, reply: oneshot::Sender<String>, payload: String) -> Pushed {
        if self.current.is_some() {
            if self.waiting.len() >= MAX_WAITING {
                let _ = reply.send(String::new());
                return Pushed::Refused;
            }
            self.waiting.push_back(Request { reply, payload });
            return Pushed::Queued;
        }
        self.current = Some(Current {
            reply,
            shown: false,
            selecting: false,
        });
        Pushed::Show(payload)
    }

    /// Answer the request on screen with `value` (empty = cancel). Returns
    /// the next request's payload to show, if one is waiting.
    pub(crate) fn answer(&mut self, value: String) -> Option<String> {
        if let Some(current) = self.current.take() {
            let _ = current.reply.send(value);
        }
        self.advance()
    }

    /// The menu was revealed or hidden. Hiding a shown request that is not
    /// in the monitor/area overlay dismisses it; returns the next payload
    /// to show in that case.
    pub(crate) fn revealed(&mut self, visible: bool) -> Option<String> {
        let current = self.current.as_mut()?;
        if visible {
            current.shown = true;
            return None;
        }
        if current.shown && !current.selecting {
            return self.answer(String::new());
        }
        None
    }

    /// The monitor/area overlay starts for the current request.
    pub(crate) fn begin_selecting(&mut self) {
        if let Some(current) = self.current.as_mut() {
            current.selecting = true;
        }
    }

    /// Promote the next waiting request whose caller is still listening (a
    /// portal call that timed out or was dropped meanwhile is skipped).
    fn advance(&mut self) -> Option<String> {
        while let Some(next) = self.waiting.pop_front() {
            if next.reply.is_closed() {
                continue;
            }
            self.current = Some(Current {
                reply: next.reply,
                shown: false,
                selecting: false,
            });
            return Some(next.payload);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(payload: &str) -> (oneshot::Sender<String>, oneshot::Receiver<String>, String) {
        let (tx, rx) = oneshot::channel();
        (tx, rx, payload.to_string())
    }

    #[test]
    fn a_second_request_cannot_take_the_first_ones_answer() {
        let mut q = PickerQueue::default();
        let (a_tx, mut a_rx, a) = request("a");
        let (b_tx, mut b_rx, b) = request("b");
        assert_eq!(q.push(a_tx, a), Pushed::Show("a".into()));
        assert_eq!(q.revealed(true), None);
        // The meeting app's picker is on screen when b arrives.
        assert_eq!(q.push(b_tx, b), Pushed::Queued);

        // The user's pick answers a, then b is shown.
        assert_eq!(q.answer("[SELECTION]/screen:DP-1".into()), Some("b".into()));
        assert_eq!(a_rx.try_recv().unwrap(), "[SELECTION]/screen:DP-1");
        assert!(b_rx.try_recv().is_err(), "b answered before it was shown");

        assert_eq!(q.answer("[SELECTION]/window:7".into()), None);
        assert_eq!(b_rx.try_recv().unwrap(), "[SELECTION]/window:7");
    }

    #[test]
    fn dismissing_a_shown_request_cancels_it_and_shows_the_next() {
        let mut q = PickerQueue::default();
        let (a_tx, mut a_rx, a) = request("a");
        let (b_tx, _b_rx, b) = request("b");
        q.push(a_tx, a);
        q.push(b_tx, b);
        q.revealed(true);
        assert_eq!(q.revealed(false), Some("b".into()));
        assert_eq!(a_rx.try_recv().unwrap(), "");
    }

    #[test]
    fn a_hide_before_the_next_request_was_shown_does_not_cancel_it() {
        // Answering a closes the menu and reopens it for b; the close's
        // reveal(false) may arrive after b became current.
        let mut q = PickerQueue::default();
        let (a_tx, _a_rx, a) = request("a");
        let (b_tx, mut b_rx, b) = request("b");
        q.push(a_tx, a);
        q.revealed(true);
        q.push(b_tx, b);
        assert_eq!(q.answer("x".into()), Some("b".into()));
        assert_eq!(q.revealed(false), None);
        assert!(b_rx.try_recv().is_err(), "b was cancelled by a's close");
        q.revealed(true);
        q.answer("y".into());
        assert_eq!(b_rx.try_recv().unwrap(), "y");
    }

    #[test]
    fn the_monitor_or_area_overlay_is_not_a_dismiss() {
        let mut q = PickerQueue::default();
        let (a_tx, mut a_rx, a) = request("a");
        q.push(a_tx, a);
        q.revealed(true);
        q.begin_selecting();
        assert_eq!(q.revealed(false), None);
        assert!(a_rx.try_recv().is_err());
        q.answer("[SELECTION]/region:DP-1@0,0,10,10".into());
        assert_eq!(
            a_rx.try_recv().unwrap(),
            "[SELECTION]/region:DP-1@0,0,10,10"
        );
    }

    #[test]
    fn a_caller_that_went_away_is_skipped() {
        let mut q = PickerQueue::default();
        let (a_tx, _a_rx, a) = request("a");
        let (b_tx, b_rx, b) = request("b");
        let (c_tx, _c_rx, c) = request("c");
        q.push(a_tx, a);
        q.push(b_tx, b);
        q.push(c_tx, c);
        drop(b_rx);
        assert_eq!(q.answer(String::new()), Some("c".into()));
    }

    #[test]
    fn a_flood_is_cancelled_not_queued_forever() {
        let mut q = PickerQueue::default();
        let (tx, _rx, p) = request("first");
        q.push(tx, p);
        let mut receivers = Vec::new();
        for i in 0..MAX_WAITING {
            let (tx, rx, p) = request(&i.to_string());
            assert_eq!(q.push(tx, p), Pushed::Queued);
            receivers.push(rx);
        }
        let (tx, mut rx, p) = request("one too many");
        assert_eq!(q.push(tx, p), Pushed::Refused);
        assert_eq!(rx.try_recv().unwrap(), "");
    }
}
