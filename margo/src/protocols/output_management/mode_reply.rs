//! Deferred `succeeded` / `failed` for configurations that change a mode.
//!
//! A mode change can't be applied while the configuration is dispatched:
//! the handler queues it on `output_pending.mode_changes` and the udev
//! backend runs `DrmCompositor::use_mode` on the next loop turn. The
//! client must only hear `succeeded` once that really worked, so each
//! queued change carries a [`ModeReplyPart`] of the configuration's
//! [`ModeReply`]; the reply goes out once the configuration has been
//! sealed and every part has reported.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Pure bookkeeping behind [`ModeReply`]: parts outstanding, whether
/// anything failed, whether the reply has gone out. Each method returns
/// `Some(outcome)` exactly once — when the configuration is sealed and
/// no part is outstanding — and `None` otherwise.
#[derive(Debug, Default)]
pub struct ReplyAggregator {
    outstanding: usize,
    failed: bool,
    sealed: bool,
    replied: bool,
}

impl ReplyAggregator {
    /// One more change will report through [`Self::finish_part`].
    pub fn add_part(&mut self) {
        self.outstanding += 1;
    }

    /// A part finished; `ok` is whether its change was applied.
    pub fn finish_part(&mut self, ok: bool) -> Option<bool> {
        self.outstanding = self.outstanding.saturating_sub(1);
        self.failed |= !ok;
        self.poll()
    }

    /// No more parts will be added; `ok` is whether the synchronous part
    /// of the configuration was applied.
    pub fn seal(&mut self, ok: bool) -> Option<bool> {
        self.sealed = true;
        self.failed |= !ok;
        self.poll()
    }

    fn poll(&mut self) -> Option<bool> {
        if self.replied || !self.sealed || self.outstanding > 0 {
            return None;
        }
        self.replied = true;
        Some(!self.failed)
    }
}

struct Shared {
    agg: ReplyAggregator,
    /// Sends the outcome (`true` = succeeded). Taken when it fires.
    sink: Option<Box<dyn FnOnce(bool)>>,
}

fn fire(shared: &Rc<RefCell<Shared>>, outcome: Option<bool>) {
    let Some(ok) = outcome else {
        return;
    };
    // Release the borrow before calling out.
    let sink = shared.borrow_mut().sink.take();
    if let Some(sink) = sink {
        sink(ok);
    }
}

/// The reply of one applied configuration. Hand out a [`Self::part`]
/// for every mode change queued for it, then [`Self::seal`].
pub struct ModeReply {
    shared: Rc<RefCell<Shared>>,
}

impl ModeReply {
    /// `sink(true)` sends `succeeded`, `sink(false)` `failed`; it runs
    /// at most once.
    pub fn new(sink: impl FnOnce(bool) + 'static) -> Self {
        Self {
            shared: Rc::new(RefCell::new(Shared {
                agg: ReplyAggregator::default(),
                sink: Some(Box::new(sink)),
            })),
        }
    }

    /// A handle for one queued mode change.
    pub fn part(&self) -> ModeReplyPart {
        self.shared.borrow_mut().agg.add_part();
        ModeReplyPart {
            shared: Some(self.shared.clone()),
        }
    }

    /// The configuration is fully dispatched (`ok` = its synchronous
    /// part was applied). Replies now if no part is outstanding.
    pub fn seal(self, ok: bool) {
        let outcome = self.shared.borrow_mut().agg.seal(ok);
        fire(&self.shared, outcome);
    }
}

/// One queued mode change's share of a [`ModeReply`]. Report with
/// [`Self::finish`]; dropping it unreported counts as a failure, so a
/// change that is discarded can't leave the client waiting forever.
pub struct ModeReplyPart {
    shared: Option<Rc<RefCell<Shared>>>,
}

impl ModeReplyPart {
    /// The change was applied (`true`) or not.
    pub fn finish(mut self, ok: bool) {
        self.report(ok);
    }

    fn report(&mut self, ok: bool) {
        if let Some(shared) = self.shared.take() {
            let outcome = shared.borrow_mut().agg.finish_part(ok);
            fire(&shared, outcome);
        }
    }
}

impl Drop for ModeReplyPart {
    fn drop(&mut self) {
        self.report(false);
    }
}

impl fmt::Debug for ModeReplyPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModeReplyPart")
            .field("reported", &self.shared.is_none())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregator_without_parts_replies_at_seal() {
        let mut a = ReplyAggregator::default();
        assert_eq!(a.seal(true), Some(true));
        let mut a = ReplyAggregator::default();
        assert_eq!(a.seal(false), Some(false));
    }

    #[test]
    fn aggregator_waits_for_every_part_in_any_order() {
        for order in [[true, true, true], [false, true, true], [true, true, false]] {
            let mut a = ReplyAggregator::default();
            (0..3).for_each(|_| a.add_part());
            assert_eq!(a.seal(true), None);
            assert_eq!(a.finish_part(order[0]), None);
            assert_eq!(a.finish_part(order[1]), None);
            assert_eq!(a.finish_part(order[2]), Some(!order.contains(&false)));
        }
    }

    #[test]
    fn aggregator_waits_for_seal_when_parts_finish_first() {
        let mut a = ReplyAggregator::default();
        a.add_part();
        assert_eq!(a.finish_part(true), None, "not sealed yet");
        assert_eq!(a.seal(true), Some(true));
    }

    #[test]
    fn aggregator_a_failed_seal_fails_even_if_parts_succeed() {
        let mut a = ReplyAggregator::default();
        a.add_part();
        assert_eq!(a.seal(false), None);
        assert_eq!(a.finish_part(true), Some(false));
    }

    #[test]
    fn aggregator_never_replies_twice() {
        let mut a = ReplyAggregator::default();
        a.add_part();
        a.seal(true);
        assert_eq!(a.finish_part(true), Some(true));
        assert_eq!(a.finish_part(false), None);
        assert_eq!(a.seal(true), None);
    }

    fn recording() -> (ModeReply, Rc<RefCell<Vec<bool>>>) {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        (ModeReply::new(move |ok| log.borrow_mut().push(ok)), sent)
    }

    #[test]
    fn reply_sends_once_after_the_last_part() {
        let (reply, sent) = recording();
        let a = reply.part();
        let b = reply.part();
        reply.seal(true);
        b.finish(true);
        assert!(sent.borrow().is_empty());
        a.finish(true);
        assert_eq!(*sent.borrow(), vec![true]);
    }

    #[test]
    fn reply_fails_once_when_any_part_fails() {
        let (reply, sent) = recording();
        let a = reply.part();
        let b = reply.part();
        reply.seal(true);
        a.finish(false);
        b.finish(true);
        assert_eq!(*sent.borrow(), vec![false]);
    }

    #[test]
    fn dropping_an_unreported_part_fails_the_reply() {
        let (reply, sent) = recording();
        let a = reply.part();
        reply.seal(true);
        drop(a);
        assert_eq!(*sent.borrow(), vec![false]);
    }
}
