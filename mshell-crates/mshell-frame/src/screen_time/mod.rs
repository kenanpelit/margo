//! Screen Time — how long each app had the keyboard focus, per day.
//!
//! Off by default: nothing runs until `screen_time.enabled`. While on, one
//! owner task on the shared tokio runtime feeds an [`engine::Tracker`]
//! from three sources:
//!
//! * margo's focused window (`MargoService::focused_client`, its app id),
//! * whether the session is locked (`MargoService::session_locked`),
//! * an [`mshell_idle::presence`] watcher (`away_seconds` without input;
//!   idle inhibitors such as a playing video keep the user present),
//!
//! and wakes once a minute to credit the running segment, roll over at
//! midnight and save. History lives in `$XDG_STATE_HOME/margo/screen-time.json`
//! (side-file rules: an unparseable file is kept and never overwritten);
//! days past `retention_days` are dropped on save. The pill and the menu
//! watch the published [`ScreenTimeState`].

pub(crate) mod engine;

use chrono::{Local, NaiveDateTime};
use engine::{History, Tracker};
use futures::StreamExt;
use mshell_config::schema::config::ScreenTimeConfig;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::watch;
use tracing::warn;

#[derive(Debug, Clone, Default)]
pub(crate) struct ScreenTimeState {
    pub(crate) enabled: bool,
    /// Counting right now (an app focused, user present, not locked).
    pub(crate) counting: bool,
    pub(crate) app: Option<String>,
    pub(crate) history: Arc<History>,
    /// The history file didn't parse: shown in the menu, never saved over.
    pub(crate) broken: Option<String>,
}

static STATE: LazyLock<watch::Sender<ScreenTimeState>> =
    LazyLock::new(|| watch::channel(ScreenTimeState::default()).0);

pub(crate) fn subscribe() -> watch::Receiver<ScreenTimeState> {
    STATE.subscribe()
}

thread_local! {
    /// The running task's settings channel (`None` = not running).
    static RUNNING: RefCell<Option<watch::Sender<ScreenTimeConfig>>> = const { RefCell::new(None) };
}

fn history_path() -> PathBuf {
    margo_fs::xdg::state_home().join("margo/screen-time.json")
}

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// Start / stop / reconfigure from `config.screen_time` (main thread).
pub(crate) fn configure(cfg: &ScreenTimeConfig) {
    RUNNING.with(|r| {
        let mut r = r.borrow_mut();
        match (cfg.enabled, r.as_ref()) {
            (false, Some(_)) => {
                // Dropping the sender ends the task after a final save.
                *r = None;
            }
            (false, None) => {
                STATE.send_modify(|s| s.enabled = false);
            }
            (true, Some(tx)) => {
                tx.send_if_modified(|c| {
                    let changed = c != cfg;
                    *c = cfg.clone();
                    changed
                });
            }
            (true, None) => {
                let (tx, rx) = watch::channel(cfg.clone());
                mshell_services::tokio_rt_spawn(run(history_path(), rx));
                *r = Some(tx);
            }
        }
    });
}

fn publish(tr: &Tracker, enabled: bool, broken: &Option<String>) {
    STATE.send_replace(ScreenTimeState {
        enabled,
        counting: tr.counting(),
        app: tr.app().map(str::to_string),
        history: Arc::new(tr.history.clone()),
        broken: broken.clone(),
    });
}

async fn save(path: &std::path::Path, tr: &mut Tracker, retention: u32) -> Option<String> {
    use margo_fs::side_file;
    tr.history.prune(now().date(), retention);
    let body = tr.history.to_json();
    let p = path.to_path_buf();
    match tokio::task::spawn_blocking(move || {
        side_file::save(&p, body.as_bytes(), |t| History::from_json(t).map(|_| ()))
    })
    .await
    {
        Ok(Ok(())) => None,
        Ok(Err(e)) => {
            warn!(error = %e, "screen time: not saved");
            Some(e.to_string())
        }
        Err(e) => Some(e.to_string()),
    }
}

async fn run(path: PathBuf, mut cfg_rx: watch::Receiver<ScreenTimeConfig>) {
    use margo_fs::side_file::{self, SideLoad};
    let p = path.clone();
    let loaded = tokio::task::spawn_blocking(move || side_file::load(&p, History::from_json))
        .await
        .unwrap_or(SideLoad::Missing);
    let (history, mut broken) = match loaded {
        SideLoad::Loaded(h) => (h, None),
        SideLoad::Missing => (History::default(), None),
        SideLoad::Broken { error, backup } => {
            warn!(%error, ?backup, "screen time: history doesn't parse; not saving over it");
            (History::default(), Some(error))
        }
    };
    let read_only = broken.is_some();
    let mut tr = Tracker::new(history);
    let mut cfg = cfg_rx.borrow_and_update().clone();

    let margo = mshell_services::margo_service();
    let mut focus = margo.focused_client.watch();
    let mut locked = margo.session_locked.watch();
    let start_presence = |secs: u32| {
        mshell_idle::presence::watch_presence(Duration::from_secs(u64::from(secs.max(30))))
            .map_err(|e| warn!(error = %e, "screen time: no idle notifications; idle time counts"))
            .ok()
    };
    let mut presence = start_presence(cfg.away_seconds);
    let mut away_rx = presence.as_ref().map(|p| p.subscribe());
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut dirty = false;
    publish(&tr, true, &broken);

    loop {
        let away_changed = async {
            match away_rx.as_mut() {
                Some(rx) => rx.changed().await.map(|()| *rx.borrow_and_update()),
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            c = focus.next() => {
                let Some(c) = c else { break };
                tr.set_app(c.map(|c| c.class.get()), now());
            }
            l = locked.next() => {
                let Some(l) = l else { break };
                tr.set_locked(l.unwrap_or(false), now());
            }
            a = away_changed => match a {
                Ok(away) => tr.set_away(away, now()),
                Err(_) => {
                    tr.set_away(false, now());
                    away_rx = None;
                }
            },
            _ = tick.tick() => {
                dirty |= tr.flush(now());
                if dirty && !read_only {
                    broken = save(&path, &mut tr, cfg.retention_days).await;
                    dirty = false;
                }
                publish(&tr, true, &broken);
                continue;
            }
            r = cfg_rx.changed() => {
                if r.is_err() {
                    break;
                }
                let new = cfg_rx.borrow_and_update().clone();
                if new.away_seconds != cfg.away_seconds {
                    presence = start_presence(new.away_seconds);
                    away_rx = presence.as_ref().map(|p| p.subscribe());
                    tr.set_away(false, now());
                }
                cfg = new;
            }
        }
        dirty = true;
        publish(&tr, true, &broken);
    }
    // Disabled (or the compositor service went away): credit and save the
    // last segment.
    tr.flush(now());
    if !read_only {
        save(&path, &mut tr, cfg.retention_days).await;
    }
    drop(presence);
    publish(&tr, false, &broken);
}
