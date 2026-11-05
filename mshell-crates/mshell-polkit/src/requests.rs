//! Bookkeeping for the `BeginAuthentication` calls in flight.
//!
//! polkitd may start several authentications at once (two apps asking
//! together) and cancels one with `CancelAuthentication(cookie)` when its
//! caller gives up. The prompt is a single window, so:
//!
//! - requests take the dialog one at a time, in arrival order. A second
//!   request must never swap its message and password channel into the
//!   dialog while the user is reading and typing for the first one (the
//!   password would authorise an action the user never saw);
//! - a cancel ends exactly the request with that cookie, whether it is
//!   still queued or already on screen, so its helper connection is dropped
//!   instead of waiting forever for a password.
//!
//! GTK-free so the ordering and cancel rules are unit tested.

use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::watch;

/// Why a request ended without running to completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cancelled {
    /// Cancelled while waiting for its turn: it never reached the dialog, so
    /// the dialog on screen belongs to another request and must stay.
    Queued,
    /// Cancelled while it owned the dialog: the caller hides it.
    Shown,
}

#[derive(Default)]
pub(crate) struct Requests {
    /// The dialog: held for the whole of one request's authentication.
    /// tokio's mutex is fair, so waiters get it in arrival order.
    dialog: tokio::sync::Mutex<()>,
    cancels: Mutex<HashMap<String, watch::Sender<bool>>>,
}

impl Requests {
    /// Run `auth` once this request owns the dialog, unless `cookie` is
    /// cancelled first. Dropping `auth` on a cancel drops whatever it holds
    /// (the helper connection, the password channel).
    pub(crate) async fn run<T>(
        &self,
        cookie: &str,
        auth: impl Future<Output = T>,
        hide_on_cancel: impl FnOnce(),
    ) -> Result<T, Cancelled> {
        let mut cancelled = self.register(cookie);
        let _forget = Forget {
            requests: self,
            cookie,
        };

        let _turn = tokio::select! {
            turn = self.dialog.lock() => turn,
            _ = cancelled.wait_for(|c| *c) => return Err(Cancelled::Queued),
        };
        tokio::select! {
            out = auth => Ok(out),
            _ = cancelled.wait_for(|c| *c) => {
                // Still under the dialog lock: hide must precede the next
                // request's Show, otherwise it can hide the new prompt.
                hide_on_cancel();
                Err(Cancelled::Shown)
            },
        }
    }

    /// Cancel the request with this cookie. `false` when none is in flight
    /// (already finished, or a cookie polkitd never gave us).
    pub(crate) fn cancel(&self, cookie: &str) -> bool {
        let cancels = self.cancels.lock().unwrap_or_else(|p| p.into_inner());
        match cancels.get(cookie) {
            Some(tx) => {
                tx.send_replace(true);
                true
            }
            None => false,
        }
    }

    fn register(&self, cookie: &str) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        self.cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(cookie.to_string(), tx);
        rx
    }

    #[cfg(test)]
    fn in_flight(&self) -> usize {
        self.cancels.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

/// Removes a request's cancel handle however `run` ends (done, cancelled,
/// or the D-Bus call future dropped).
struct Forget<'a> {
    requests: &'a Requests,
    cookie: &'a str,
}

impl Drop for Forget<'_> {
    fn drop(&mut self) {
        self.requests
            .cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(self.cookie);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::oneshot;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime")
    }

    /// Let the spawned tasks run until they block.
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn a_second_request_waits_until_the_first_is_done() {
        rt().block_on(async {
            let requests = Arc::new(Requests::default());
            let log = Arc::new(Mutex::new(Vec::new()));
            let (finish_a, a_done) = oneshot::channel::<()>();

            let a = tokio::spawn({
                let (requests, log) = (requests.clone(), log.clone());
                async move {
                    requests
                        .run(
                            "a",
                            async {
                                log.lock().unwrap().push("a shown");
                                let _ = a_done.await;
                                log.lock().unwrap().push("a done");
                            },
                            || {},
                        )
                        .await
                }
            });
            settle().await;
            let b = tokio::spawn({
                let (requests, log) = (requests.clone(), log.clone());
                async move {
                    requests
                        .run(
                            "b",
                            async {
                                log.lock().unwrap().push("b shown");
                            },
                            || {},
                        )
                        .await
                }
            });
            settle().await;
            assert_eq!(
                *log.lock().unwrap(),
                ["a shown"],
                "b took the dialog from a"
            );

            finish_a.send(()).unwrap();
            assert_eq!(a.await.unwrap(), Ok(()));
            assert_eq!(b.await.unwrap(), Ok(()));
            assert_eq!(*log.lock().unwrap(), ["a shown", "a done", "b shown"]);
            assert_eq!(requests.in_flight(), 0);
        });
    }

    #[test]
    fn cancel_ends_the_request_on_screen_and_drops_its_work() {
        rt().block_on(async {
            let requests = Arc::new(Requests::default());
            let (_keep, never) = oneshot::channel::<()>();
            let (held_tx, held_rx) = oneshot::channel::<()>();
            let run = tokio::spawn({
                let requests = requests.clone();
                async move {
                    requests
                        .run(
                            "c",
                            async move {
                                // Stands in for the helper connection: dropped
                                // when the request is cancelled.
                                let _helper = held_tx;
                                let _ = never.await;
                            },
                            || {},
                        )
                        .await
                }
            });
            settle().await;
            assert!(requests.cancel("c"));
            let out = tokio::time::timeout(Duration::from_secs(2), run)
                .await
                .expect("cancel did not end the request");
            assert_eq!(out.unwrap(), Err(Cancelled::Shown));
            assert!(held_rx.await.is_err(), "the request's work was not dropped");
            assert_eq!(requests.in_flight(), 0);
        });
    }

    #[test]
    fn cancelling_a_queued_request_leaves_the_shown_one_alone() {
        rt().block_on(async {
            let requests = Arc::new(Requests::default());
            let (finish_a, a_done) = oneshot::channel::<()>();
            let a = tokio::spawn({
                let requests = requests.clone();
                async move {
                    requests
                        .run("a", async { a_done.await.is_ok() }, || {})
                        .await
                }
            });
            settle().await;
            let b = tokio::spawn({
                let requests = requests.clone();
                async move { requests.run("b", async { true }, || {}).await }
            });
            settle().await;

            assert!(requests.cancel("b"));
            assert_eq!(b.await.unwrap(), Err(Cancelled::Queued));
            finish_a.send(()).unwrap();
            assert_eq!(a.await.unwrap(), Ok(true));
        });
    }

    #[test]
    fn cancelling_hides_while_the_request_still_owns_the_dialog() {
        rt().block_on(async {
            let requests = Arc::new(Requests::default());
            let hidden = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let run = tokio::spawn({
                let requests = requests.clone();
                let hidden = hidden.clone();
                async move {
                    requests
                        .run("hide", std::future::pending::<()>(), || {
                            assert!(
                                requests.dialog.try_lock().is_err(),
                                "dialog was released before hide"
                            );
                            hidden.store(true, std::sync::atomic::Ordering::SeqCst);
                        })
                        .await
                }
            });
            settle().await;
            requests.cancel("hide");
            assert_eq!(run.await.unwrap(), Err(Cancelled::Shown));
            assert!(hidden.load(std::sync::atomic::Ordering::SeqCst));
        });
    }

    #[test]
    fn an_unknown_or_finished_cookie_cancels_nothing() {
        rt().block_on(async {
            let requests = Requests::default();
            assert!(!requests.cancel("nope"));
            assert_eq!(requests.run("d", async { 7 }, || {}).await, Ok(7));
            assert!(!requests.cancel("d"));
            assert_eq!(requests.in_flight(), 0);
        });
    }
}
