//! One `exec` loop per distinct custom-pill source, shared by every bar.
//!
//! Each monitor's bar builds its own pill, so a `!Custom` pill on three
//! monitors used to spawn its `exec` three times per interval. [`ExecHub`]
//! keeps one producer per key ([`ExecKey`]: every config field that changes
//! what a run delivers) and fans each result out to all its pills over a
//! `watch` channel. The producer runs while at least one
//! [`ExecSubscription`] is active (its pill is shown) and is aborted when
//! the last one is dropped. A pill rebuilt with a different config gets a
//! different key, so it never shares a loop with the old one.

use mshell_config::schema::config::{CustomWidgetConfig, PluginSecretEnv};
use mshell_utils::shared_poll::BoxFuture;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;
use tokio::sync::{Notify, broadcast, watch};
use tokio::task::AbortHandle;

/// Everything about a pill's config that changes what one `exec` run
/// delivers. Two pills share a producer only when all of it is equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecKey {
    /// `mshellctl custom-refresh <name>` targets pills by name.
    pub(crate) name: String,
    pub(crate) exec: String,
    /// The child's extra environment (the rest is mshell's own, as is the
    /// working directory, so it is the same for every pill).
    pub(crate) secret_env: Vec<PluginSecretEnv>,
    pub(crate) interval: u64,
    pub(crate) json: bool,
    pub(crate) art: bool,
    pub(crate) template: String,
    pub(crate) max_chars: u32,
}

impl ExecKey {
    pub(crate) fn of(config: &CustomWidgetConfig) -> Self {
        // No `..`: a new config field fails to compile here until someone
        // decides whether it changes the exec's output.
        let CustomWidgetConfig {
            name,
            exec,
            secret_env,
            interval,
            json,
            art,
            template,
            max_chars,
            icon: _,
            image: _,
            label: _,
            tooltip: _,
            on_click: _,
            on_click_right: _,
            menu: _,
            panel_entry: _,
            panel_settings: _,
            panel_capabilities: _,
            panel_min_width: _,
            panel_max_height: _,
        } = config.clone();
        Self {
            name,
            exec,
            secret_env,
            interval,
            json,
            art,
            template,
            max_chars,
        }
    }
}

/// One run of a source: `None` when the command failed (pills keep what
/// they show).
pub(crate) type Probe<V> = Arc<dyn Fn() -> BoxFuture<Option<V>> + Send + Sync>;

/// How a source runs.
pub(crate) struct Producer<V> {
    pub(crate) probe: Probe<V>,
    /// Time between runs while shown; `None` runs once, then only on a
    /// forced refresh.
    pub(crate) period: Option<Duration>,
    /// The name `mshellctl custom-refresh <name>` matches.
    pub(crate) name: String,
}

/// State shared by a source's loop and its subscriptions.
struct Shared<V> {
    value: watch::Sender<Option<V>>,
    /// Active (shown) subscriptions.
    active: watch::Sender<usize>,
    /// A forced refresh (right-click on any of its pills).
    refresh: Notify,
}

/// A running producer. Dropped with its last subscription, which ends the
/// loop.
pub(crate) struct Source<V> {
    shared: Arc<Shared<V>>,
    task: AbortHandle,
}

impl<V> Drop for Source<V> {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The producers by key. Keep it in a `static`.
pub(crate) struct ExecHub<K, V> {
    sources: Mutex<Vec<(K, Weak<Source<V>>)>>,
}

impl<K: PartialEq, V: Clone + PartialEq + Send + Sync + 'static> ExecHub<K, V> {
    pub(crate) const fn new() -> Self {
        Self {
            sources: Mutex::new(Vec::new()),
        }
    }

    /// A new, initially inactive subscription to the source for `key`.
    /// When there is none yet, `make` describes it and `spawn` starts its
    /// loop.
    pub(crate) fn subscribe(
        &self,
        key: K,
        make: impl FnOnce() -> Producer<V>,
        spawn: impl FnOnce(BoxFuture<()>) -> AbortHandle,
    ) -> ExecSubscription<V> {
        let mut sources = self.sources.lock().unwrap_or_else(PoisonError::into_inner);
        sources.retain(|(_, s)| s.strong_count() > 0);
        let source = sources
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, s)| s.upgrade());
        let source = match source {
            Some(source) => source,
            None => {
                let shared = Arc::new(Shared {
                    value: watch::channel(None).0,
                    active: watch::channel(0).0,
                    refresh: Notify::new(),
                });
                let task = spawn(Box::pin(drive(shared.clone(), make())));
                let source = Arc::new(Source { shared, task });
                sources.push((key, Arc::downgrade(&source)));
                source
            }
        };
        ExecSubscription {
            source,
            active: AtomicBool::new(false),
        }
    }

    /// Live sources (tests).
    #[cfg(test)]
    fn len(&self) -> usize {
        let sources = self.sources.lock().unwrap_or_else(PoisonError::into_inner);
        sources.iter().filter(|(_, s)| s.strong_count() > 0).count()
    }
}

/// One pill's handle on its source.
pub(crate) struct ExecSubscription<V> {
    source: Arc<Source<V>>,
    active: AtomicBool,
}

impl<V> ExecSubscription<V> {
    /// The pill is shown (the source should run) or not. Idempotent.
    pub(crate) fn set_active(&self, on: bool) {
        if self.active.swap(on, Ordering::SeqCst) != on {
            self.source
                .shared
                .active
                .send_modify(|n| *n = if on { *n + 1 } else { n.saturating_sub(1) });
        }
    }

    /// Run the source again now, for every pill on it.
    pub(crate) fn refresh(&self) {
        self.source.shared.refresh.notify_one();
    }

    /// The latest result (`None` until the first successful run).
    pub(crate) fn watch(&self) -> watch::Receiver<Option<V>> {
        self.source.shared.value.subscribe()
    }
}

impl<V> Drop for ExecSubscription<V> {
    fn drop(&mut self) {
        self.set_active(false);
    }
}

/// The producer loop: run while shown, publish, wait for the next interval
/// of shown time (a source shown again after a hide runs at once), a
/// right-click, or an IPC refresh aimed at it.
async fn drive<V: PartialEq + Send + Sync>(shared: Arc<Shared<V>>, producer: Producer<V>) {
    let mut ipc = mshell_common::custom_refresh::subscribe();
    let mut active = shared.active.subscribe();
    loop {
        if active.wait_for(|n| *n > 0).await.is_err() {
            return;
        }
        if let Some(value) = (producer.probe)().await {
            shared.value.send_if_modified(|cur| {
                let changed = cur.as_ref() != Some(&value);
                if changed {
                    *cur = Some(value);
                }
                changed
            });
        }
        let mut gate = shared.active.subscribe();
        let sleep = async {
            match producer.period {
                Some(period) => shown_tick(&mut gate, period).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                () = &mut sleep => break,
                () = shared.refresh.notified() => break,
                msg = ipc.recv() => match msg {
                    Ok(target) if mshell_common::custom_refresh::targets(&target, &producer.name) => break,
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    // Can't happen (the sender is a static); don't spin on it.
                    Err(broadcast::error::RecvError::Closed) => {
                        ipc = mshell_common::custom_refresh::subscribe();
                    }
                },
            }
        }
    }
}

/// Wait `period` of time with at least one pill shown. While none is, no
/// timer is armed; a hide mid-wait drops the sleep, and the next show ends
/// the wait at once.
async fn shown_tick(active: &mut watch::Receiver<usize>, period: Duration) {
    if *active.borrow_and_update() > 0 {
        tokio::select! {
            () = tokio::time::sleep(period) => return,
            _ = active.wait_for(|n| *n == 0) => {}
        }
    }
    let _ = active.wait_for(|n| *n > 0).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn spawn(fut: BoxFuture<()>) -> AbortHandle {
        tokio::spawn(fut).abort_handle()
    }

    /// A producer that counts its runs and returns the run number.
    fn counting(name: &str, period: Option<u64>, runs: &Arc<AtomicUsize>) -> Producer<usize> {
        let runs = runs.clone();
        Producer {
            probe: Arc::new(move || {
                let runs = runs.clone();
                Box::pin(async move { Some(runs.fetch_add(1, Ordering::SeqCst) + 1) })
            }),
            period: period.map(Duration::from_secs),
            name: name.to_string(),
        }
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn pills_with_one_source_share_one_run_per_interval() {
        let hub: ExecHub<&str, usize> = ExecHub::new();
        let runs = Arc::new(AtomicUsize::new(0));
        // Three monitors, one pill each.
        let pills: Vec<_> = (0..3)
            .map(|_| hub.subscribe("cpu", || counting("cpu", Some(10), &runs), spawn))
            .collect();
        assert_eq!(hub.len(), 1);
        for p in &pills {
            p.set_active(true);
        }
        settle().await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 4, "runs at 0, 10, 20, 30 s");
        // Every pill sees the same, latest result.
        for p in &pills {
            assert_eq!(*p.watch().borrow(), Some(4));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn different_keys_never_share_a_source() {
        let hub: ExecHub<&str, usize> = ExecHub::new();
        let (a, b) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let pa = hub.subscribe("a", || counting("a", Some(10), &a), spawn);
        let pb = hub.subscribe("b", || counting("b", Some(10), &b), spawn);
        assert_eq!(hub.len(), 2);
        pa.set_active(true);
        pb.set_active(true);
        settle().await;
        assert_eq!((a.load(Ordering::SeqCst), b.load(Ordering::SeqCst)), (1, 1));
    }

    #[tokio::test(start_paused = true)]
    async fn runs_only_while_some_pill_is_shown() {
        let hub: ExecHub<&str, usize> = ExecHub::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let p1 = hub.subscribe("k", || counting("k", Some(10), &runs), spawn);
        let p2 = hub.subscribe("k", || counting("k", Some(10), &runs), spawn);
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 0, "nothing shown: no run");
        p1.set_active(true);
        settle().await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        // The other monitor's pill showing and hiding changes nothing.
        p2.set_active(true);
        p2.set_active(false);
        settle().await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        p1.set_active(false);
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "all hidden: stopped");
        // Shown again: stale, runs at once.
        p2.set_active(true);
        settle().await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_last_drop_stops_the_source_and_a_new_config_rekeys() {
        let hub: ExecHub<String, usize> = ExecHub::new();
        let old = Arc::new(AtomicUsize::new(0));
        let p1 = hub.subscribe("v1".into(), || counting("w", Some(10), &old), spawn);
        let p2 = hub.subscribe("v1".into(), || counting("w", Some(10), &old), spawn);
        p1.set_active(true);
        p2.set_active(true);
        settle().await;
        drop(p1);
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(old.load(Ordering::SeqCst), 2, "one pill left: still runs");
        // Config change: the bars rebuild their pills with the new config.
        let new = Arc::new(AtomicUsize::new(0));
        drop(p2);
        let p3 = hub.subscribe("v2".into(), || counting("w", Some(10), &new), spawn);
        p3.set_active(true);
        tokio::time::sleep(Duration::from_secs(25)).await;
        assert_eq!(old.load(Ordering::SeqCst), 2, "old source stopped");
        assert_eq!(new.load(Ordering::SeqCst), 3);
        assert_eq!(hub.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refresh_runs_once_for_every_pill() {
        let hub: ExecHub<&str, usize> = ExecHub::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let name = "exec-hub-test-refresh";
        let p1 = hub.subscribe("r", || counting(name, None, &runs), spawn);
        let p2 = hub.subscribe("r", || counting(name, None, &runs), spawn);
        p1.set_active(true);
        p2.set_active(true);
        settle().await;
        tokio::time::sleep(Duration::from_secs(600)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "interval 0: runs once");
        // Right-click on one monitor's pill.
        p2.refresh();
        settle().await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert_eq!(*p1.watch().borrow(), Some(2), "delivered to the other pill");
        // `mshellctl custom-refresh <name>`, another name, and every pill.
        assert!(mshell_common::custom_refresh::request(name) >= 1);
        settle().await;
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        mshell_common::custom_refresh::request("exec-hub-test-other");
        settle().await;
        assert_eq!(runs.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_pill_gets_the_current_value_without_a_run() {
        let hub: ExecHub<&str, usize> = ExecHub::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let p1 = hub.subscribe("l", || counting("l", Some(10), &runs), spawn);
        p1.set_active(true);
        settle().await;
        let p2 = hub.subscribe("l", || counting("l", Some(10), &runs), spawn);
        p2.set_active(true);
        settle().await;
        assert_eq!(*p2.watch().borrow(), Some(1));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn every_output_field_splits_the_key_and_the_rest_does_not() {
        let base = CustomWidgetConfig {
            name: "w".into(),
            exec: "date".into(),
            interval: 5,
            ..Default::default()
        };
        let k = ExecKey::of(&base);
        let differs: Vec<CustomWidgetConfig> = vec![
            CustomWidgetConfig {
                name: "w2".into(),
                ..base.clone()
            },
            CustomWidgetConfig {
                exec: "date -u".into(),
                ..base.clone()
            },
            CustomWidgetConfig {
                interval: 6,
                ..base.clone()
            },
            CustomWidgetConfig {
                json: true,
                ..base.clone()
            },
            CustomWidgetConfig {
                art: true,
                ..base.clone()
            },
            CustomWidgetConfig {
                template: "[{output}]".into(),
                ..base.clone()
            },
            CustomWidgetConfig {
                max_chars: 3,
                ..base.clone()
            },
            CustomWidgetConfig {
                secret_env: vec![PluginSecretEnv {
                    var: "MPLUGIN_SECRET_K".into(),
                    plugin: "p".into(),
                    key: "k".into(),
                }],
                ..base.clone()
            },
        ];
        for c in &differs {
            assert_ne!(ExecKey::of(c), k, "{c:?}");
        }
        let same = CustomWidgetConfig {
            icon: "x".into(),
            image: "/x.png".into(),
            label: "l".into(),
            tooltip: "t".into(),
            on_click: "a".into(),
            on_click_right: "b".into(),
            panel_min_width: 9,
            ..base.clone()
        };
        assert_eq!(ExecKey::of(&same), k);
    }
}
