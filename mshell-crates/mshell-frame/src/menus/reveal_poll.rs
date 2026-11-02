//! A poll timer that only exists while its menu is revealed.
//!
//! Several menus refresh on a fixed tick (CPU dashboard, system status,
//! overview). Gating the *work* on a `revealed` flag still left the timer
//! waking the main loop every few seconds for the whole session; this owns
//! the `glib` source itself and removes it on hide (DESIGN.md: a hidden
//! menu does no work).

use relm4::gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub(crate) struct RevealPoll {
    interval: Duration,
    /// Runs every `interval` while revealed; returning `false` ends the poll
    /// for good (the component is gone).
    tick: Rc<dyn Fn() -> bool>,
    /// The live source, if any. Cleared by the callback itself when it ends,
    /// so `stop` never removes an already-destroyed source.
    source: Rc<RefCell<Option<glib::SourceId>>>,
}

impl RevealPoll {
    /// A started poll (menus are built on their first reveal).
    pub(crate) fn new(interval: Duration, tick: impl Fn() -> bool + 'static) -> Self {
        let poll = Self {
            interval,
            tick: Rc::new(tick),
            source: Rc::new(RefCell::new(None)),
        };
        poll.set_revealed(true);
        poll
    }

    /// Start the timer on reveal, remove it on hide. Idempotent.
    pub(crate) fn set_revealed(&self, revealed: bool) {
        if revealed {
            if self.source.borrow().is_some() {
                return;
            }
            let tick = self.tick.clone();
            let slot = self.source.clone();
            let id = glib::timeout_add_local(self.interval, move || {
                if tick() {
                    glib::ControlFlow::Continue
                } else {
                    slot.borrow_mut().take();
                    glib::ControlFlow::Break
                }
            });
            *self.source.borrow_mut() = Some(id);
        } else if let Some(id) = self.source.borrow_mut().take() {
            id.remove();
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        self.source.borrow().is_some()
    }
}

impl Drop for RevealPoll {
    fn drop(&mut self) {
        self.set_revealed(false);
    }
}

impl std::fmt::Debug for RevealPoll {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevealPoll")
            .field("interval", &self.interval)
            .field("running", &self.is_running())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Spin `ctx` until `done` or a deadline.
    fn spin(ctx: &glib::MainContext, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !done() && std::time::Instant::now() < deadline {
            ctx.iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// One test, because `timeout_add_local` attaches to the *global*
    /// default main context, which only one thread can own at a time.
    #[test]
    fn ticks_only_while_revealed_and_survives_its_own_end() {
        let ctx = glib::MainContext::default();
        let _owner = ctx.acquire().expect("own the default main context");

        let ticks = Rc::new(Cell::new(0u32));
        let counter = ticks.clone();
        let poll = RevealPoll::new(Duration::from_millis(1), move || {
            counter.set(counter.get() + 1);
            true
        });
        spin(&ctx, || ticks.get() >= 2);
        assert!(ticks.get() >= 2, "ticks while revealed");

        poll.set_revealed(false);
        assert!(!poll.is_running());
        let frozen = ticks.get();
        for _ in 0..20 {
            ctx.iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(ticks.get(), frozen, "no ticks while hidden");

        poll.set_revealed(true);
        poll.set_revealed(true); // idempotent: still one source
        spin(&ctx, || ticks.get() > frozen);
        assert!(ticks.get() > frozen, "resumes on reveal");
        drop(poll);

        // A tick returning false ends the poll; hiding or dropping it
        // afterwards must not try to remove the destroyed source (panics).
        let finished = RevealPoll::new(Duration::from_millis(1), || false);
        spin(&ctx, || !finished.is_running());
        assert!(!finished.is_running(), "returning false ends the poll");
        finished.set_revealed(false);
        drop(finished);
    }
}
