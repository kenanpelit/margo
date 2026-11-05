//! A poll timer that only exists while its menu is revealed.
//!
//! Several menus refresh on a fixed tick (CPU dashboard, system status,
//! overview). Gating the *work* on a `revealed` flag still left the timer
//! waking the main loop every few seconds for the whole session; this owns
//! the `glib` source itself and removes it on hide (DESIGN.md: a hidden
//! menu does no work).
//!
//! [`RevealGate`] is the same rule for async subscriptions on the services
//! runtime: [`RevealGate::stream_while_revealed`] holds a stream only
//! while the menu is revealed.

use relm4::gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

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

/// A menu's revealed flag that its async poll loops wait on. Set it from
/// the menu's reveal handler; clones share the flag.
#[derive(Clone)]
pub(crate) struct RevealGate(Arc<watch::Sender<bool>>);

impl RevealGate {
    pub(crate) fn new(revealed: bool) -> Self {
        Self(Arc::new(watch::channel(revealed).0))
    }

    /// Show / hide. Idempotent.
    pub(crate) fn set(&self, revealed: bool) {
        self.0.send_if_modified(|cur| {
            let changed = *cur != revealed;
            *cur = revealed;
            changed
        });
    }

    pub(crate) fn is_revealed(&self) -> bool {
        *self.0.borrow()
    }

    /// Resolves once the menu is revealed (at once when it already is).
    pub(crate) async fn until_revealed(&self) {
        // The sender lives in `self`, so the wait can't fail.
        let _ = self.0.subscribe().wait_for(|on| *on).await;
    }

    /// Resolves once the menu is hidden (at once when it already is).
    pub(crate) async fn until_hidden(&self) {
        let _ = self.0.subscribe().wait_for(|on| !*on).await;
    }

    /// Feed `each` from a stream that exists only while the menu is
    /// revealed: `open` subscribes on reveal, the stream is dropped on
    /// hide. For sources (wayle `.watch()`) that poll only while someone
    /// subscribes. Returns when a stream ends.
    pub(crate) async fn stream_while_revealed<S>(
        &self,
        mut open: impl FnMut() -> S,
        mut each: impl FnMut(S::Item),
    ) where
        S: futures::Stream,
    {
        use futures::StreamExt;
        loop {
            self.until_revealed().await;
            let mut stream = Box::pin(open());
            loop {
                tokio::select! {
                    () = self.until_hidden() => break,
                    next = stream.next() => match next {
                        Some(item) => each(item),
                        None => return,
                    },
                }
            }
        }
    }
}

impl std::fmt::Debug for RevealGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RevealGate")
            .field(&self.is_revealed())
            .finish()
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    /// Let spawned tasks run up to their next await.
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    /// A stream that counts how many of it are alive.
    struct Counted(
        Arc<std::sync::atomic::AtomicUsize>,
        tokio::sync::mpsc::UnboundedReceiver<u32>,
    );
    impl futures::Stream for Counted {
        type Item = u32;
        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<u32>> {
            self.1.poll_recv(cx)
        }
    }
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_stream_exists_only_while_revealed() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let gate = RevealGate::new(false);
        let alive = Arc::new(AtomicUsize::new(0));
        let opened = Arc::new(AtomicUsize::new(0));
        let got = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<u32>();
        let rx = Arc::new(std::sync::Mutex::new(Some(rx)));
        let (g, a, o, gt, r) = (
            gate.clone(),
            alive.clone(),
            opened.clone(),
            got.clone(),
            rx.clone(),
        );
        let task = tokio::spawn(async move {
            g.stream_while_revealed(
                || {
                    o.fetch_add(1, Ordering::SeqCst);
                    a.fetch_add(1, Ordering::SeqCst);
                    // Reuse the one channel across subscriptions.
                    let rx = r.lock().ok().and_then(|mut s| s.take());
                    let rx = rx.unwrap_or_else(|| tokio::sync::mpsc::unbounded_channel().1);
                    Counted(a.clone(), rx)
                },
                |v| gt.lock().map(|mut g| g.push(v)).unwrap_or(()),
            )
            .await;
        });
        settle().await;
        assert_eq!(opened.load(Ordering::SeqCst), 0, "hidden: never subscribed");

        gate.set(true);
        settle().await;
        assert_eq!(alive.load(Ordering::SeqCst), 1);
        tx.send(7).ok();
        settle().await;
        assert_eq!(*got.lock().map(|g| g.clone()).unwrap_or_default(), vec![7]);

        gate.set(false);
        settle().await;
        assert_eq!(
            alive.load(Ordering::SeqCst),
            0,
            "hide drops the subscription"
        );

        gate.set(true);
        settle().await;
        assert_eq!(opened.load(Ordering::SeqCst), 2, "reveal subscribes again");
        task.abort();
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
