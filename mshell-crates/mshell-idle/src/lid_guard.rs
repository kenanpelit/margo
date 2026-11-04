//! Lid guard — an optional logind `handle-lid-switch` inhibitor tied to
//! Keep Awake. Ports the noctalia `lid-guard` / `super-caffeine`
//! plugins natively: while Keep Awake is on (toggle or timer) and the
//! user opted in (`idle.inhibit_lid_switch`), mshell holds a logind
//! `Inhibit("handle-lid-switch", …, "block")` file descriptor, so
//! closing the laptop lid on an external monitor does not suspend.
//!
//! Unlike the plugins (a transient `systemd-run … systemd-inhibit sleep
//! infinity` user unit) we hold the fd in-process: when mshell exits or
//! crashes the fd closes and logind restores the normal lid policy by
//! itself — nothing can be left behind.
//!
//! The media-playback hold (`idle.inhibit_while_media`) also drives the
//! idle inhibitor; it deliberately does **not** engage the lid guard, so
//! closing the lid while music plays still suspends (a laptop in a bag
//! must not keep running because a player was left open).
//!
//! This only blocks logind's own lid handling. It does not stop an
//! explicit suspend (power menu, `systemctl suspend`) or the idle
//! manager's suspend stage — those are covered by the idle inhibitor.

use crate::inhibitor::{IdleInhibitor, LogindManagerProxy};
use std::sync::OnceLock;
use tokio::sync::{Mutex, watch};
use tracing::{info, warn};
use zbus::{Connection, zvariant::OwnedFd};

/// Everything the hold decision depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LidInputs {
    /// `idle.inhibit_lid_switch` — the user's opt-in.
    pub opt_in: bool,
    /// The idle inhibitor (Keep Awake) is on.
    pub idle_on: bool,
    /// The idle inhibitor is on *only* because media is playing.
    pub media_only: bool,
}

/// Should the lid-switch inhibitor be held for these inputs?
pub fn should_hold(i: LidInputs) -> bool {
    i.opt_in && i.idle_on && !i.media_only
}

/// What the reconcile loop must do to go from `held` to `want`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LidAction {
    Acquire,
    Release,
    Keep,
}

pub fn plan(held: bool, want: bool) -> LidAction {
    match (held, want) {
        (false, true) => LidAction::Acquire,
        (true, false) => LidAction::Release,
        _ => LidAction::Keep,
    }
}

/// Observable lid-guard status for the Keep Awake menu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LidStatus {
    /// The inhibitor fd is currently held.
    pub held: bool,
    /// Last acquire error (logind refused / no system bus), if any.
    pub error: Option<String>,
}

static INSTANCE: OnceLock<LidGuard> = OnceLock::new();

pub struct LidGuard {
    fd: Mutex<Option<OwnedFd>>,
    inputs_tx: watch::Sender<LidInputs>,
    status_tx: watch::Sender<LidStatus>,
}

impl LidGuard {
    pub fn global() -> &'static Self {
        INSTANCE.get_or_init(|| {
            let (inputs_tx, _) = watch::channel(LidInputs::default());
            let (status_tx, _) = watch::channel(LidStatus::default());
            Self {
                fd: Mutex::new(None),
                inputs_tx,
                status_tx,
            }
        })
    }

    /// Feed the `idle.inhibit_lid_switch` setting.
    pub fn set_opt_in(&self, opt_in: bool) {
        self.inputs_tx.send_if_modified(|i| {
            let changed = i.opt_in != opt_in;
            i.opt_in = opt_in;
            changed
        });
    }

    /// Feed "the idle inhibitor is held only for media playback".
    pub fn set_media_only(&self, media_only: bool) {
        self.inputs_tx.send_if_modified(|i| {
            let changed = i.media_only != media_only;
            i.media_only = media_only;
            changed
        });
    }

    fn set_idle_on(&self, idle_on: bool) {
        self.inputs_tx.send_if_modified(|i| {
            let changed = i.idle_on != idle_on;
            i.idle_on = idle_on;
            changed
        });
    }

    pub fn status(&self) -> LidStatus {
        self.status_tx.borrow().clone()
    }

    pub fn watch_status(&self) -> watch::Receiver<LidStatus> {
        self.status_tx.subscribe()
    }

    /// Run the reconcile loop forever (spawn once at startup). Follows the
    /// idle inhibitor's state and the fed inputs, acquiring / releasing
    /// the logind fd as [`should_hold`] dictates.
    pub async fn run(&'static self) {
        let mut idle_rx = IdleInhibitor::global().subscribe();
        let mut inputs_rx = self.inputs_tx.subscribe();
        loop {
            self.set_idle_on(*idle_rx.borrow_and_update());
            let want = should_hold(*inputs_rx.borrow_and_update());
            self.reconcile(want).await;
            tokio::select! {
                r = idle_rx.changed() => if r.is_err() { break },
                r = inputs_rx.changed() => if r.is_err() { break },
            }
        }
    }

    async fn reconcile(&self, want: bool) {
        let mut guard = self.fd.lock().await;
        match plan(guard.is_some(), want) {
            LidAction::Keep => {}
            LidAction::Release => {
                guard.take();
                info!("lid guard: released handle-lid-switch inhibitor");
                self.status_tx.send_replace(LidStatus::default());
            }
            LidAction::Acquire => match acquire().await {
                Ok(fd) => {
                    *guard = Some(fd);
                    info!("lid guard: holding handle-lid-switch inhibitor");
                    self.status_tx.send_replace(LidStatus {
                        held: true,
                        error: None,
                    });
                }
                Err(e) => {
                    warn!(error = %e, "lid guard: logind Inhibit(handle-lid-switch) failed");
                    self.status_tx.send_replace(LidStatus {
                        held: false,
                        error: Some(e.to_string()),
                    });
                }
            },
        }
    }
}

async fn acquire() -> zbus::Result<OwnedFd> {
    let conn = Connection::system().await?;
    let proxy = LogindManagerProxy::new(&conn).await?;
    proxy
        .inhibit(
            "handle-lid-switch",
            "mshell",
            "Keep Awake: keep running with the lid closed",
            "block",
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(opt_in: bool, idle_on: bool, media_only: bool) -> LidInputs {
        LidInputs {
            opt_in,
            idle_on,
            media_only,
        }
    }

    #[test]
    fn holds_only_when_opted_in_and_keep_awake_is_on() {
        assert!(should_hold(inputs(true, true, false)));
        assert!(!should_hold(inputs(false, true, false)));
        assert!(!should_hold(inputs(true, false, false)));
        assert!(!should_hold(inputs(false, false, false)));
    }

    #[test]
    fn media_playback_hold_never_engages_the_lid_guard() {
        assert!(!should_hold(inputs(true, true, true)));
    }

    #[test]
    fn plan_transitions() {
        assert_eq!(plan(false, true), LidAction::Acquire);
        assert_eq!(plan(true, false), LidAction::Release);
        assert_eq!(plan(true, true), LidAction::Keep);
        assert_eq!(plan(false, false), LidAction::Keep);
    }

    #[test]
    fn inputs_are_idempotent_and_default_off() {
        let g = LidGuard {
            fd: Mutex::new(None),
            inputs_tx: watch::channel(LidInputs::default()).0,
            status_tx: watch::channel(LidStatus::default()).0,
        };
        let mut rx = g.inputs_tx.subscribe();
        assert!(!should_hold(*rx.borrow_and_update()));
        g.set_opt_in(false);
        assert!(
            !rx.has_changed().unwrap(),
            "no-op set must not wake the loop"
        );
        g.set_opt_in(true);
        g.set_idle_on(true);
        assert!(rx.has_changed().unwrap());
        assert!(should_hold(*rx.borrow_and_update()));
        g.set_media_only(true);
        assert!(!should_hold(*rx.borrow_and_update()));
    }
}
