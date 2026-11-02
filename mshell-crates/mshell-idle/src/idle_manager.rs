//! Idle manager — staged idle actions via `ext-idle-notify-v1`.
//!
//! Runs a small Wayland client on a dedicated thread (mirroring
//! `mshell-gamma`), binds `ext_idle_notifier_v1`, and arms one
//! `ext_idle_notification_v1` per enabled stage (dim / lock /
//! suspend) with that stage's timeout. As the session sits idle
//! the notifications fire `idled`; the highest reached stage is
//! published on a `watch` channel. Any input fires `resumed` on
//! all of them, dropping back to `Active`.
//!
//! The manager only *reports* the stage — the shell decides what
//! to do (and whether to honour the idle inhibitor). Timeouts are
//! reconfigured live: the shell pushes a new `IdleConfig` and the
//! notifications are torn down and re-armed.

use std::os::fd::AsRawFd;

use anyhow::{Context, Result};
use tokio::sync::watch;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};

/// Per-stage idle timeouts in minutes. `None` disables the stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IdleConfig {
    pub dim_minutes: Option<u32>,
    pub lock_minutes: Option<u32>,
    pub suspend_minutes: Option<u32>,
}

/// The deepest idle stage currently reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleStage {
    Active,
    Dim,
    Lock,
    Suspend,
}

impl IdleStage {
    fn rank(self) -> u8 {
        match self {
            IdleStage::Active => 0,
            IdleStage::Dim => 1,
            IdleStage::Lock => 2,
            IdleStage::Suspend => 3,
        }
    }
}

/// Start the idle manager. Returns a sender to push timeout
/// changes and a receiver that yields the current idle stage.
pub fn start(
    initial: IdleConfig,
) -> Result<(watch::Sender<IdleConfig>, watch::Receiver<IdleStage>)> {
    let (cfg_tx, cfg_rx) = watch::channel(initial);
    let (stage_tx, stage_rx) = watch::channel(IdleStage::Active);

    std::thread::Builder::new()
        .name("mshell-idle".into())
        .spawn(move || {
            if let Err(e) = run(cfg_rx, stage_tx) {
                eprintln!("mshell-idle: idle manager thread exited: {e:#}");
            }
        })
        .context("failed to spawn mshell-idle thread")?;

    Ok((cfg_tx, stage_rx))
}

struct Notification {
    stage: IdleStage,
    object: ext_idle_notification_v1::ExtIdleNotificationV1,
    idled: bool,
}

struct AppData {
    notifier: Option<ext_idle_notifier_v1::ExtIdleNotifierV1>,
    seat: Option<wl_seat::WlSeat>,
    notifications: Vec<Notification>,
    stage_tx: watch::Sender<IdleStage>,
}

impl AppData {
    /// Recompute the deepest idled stage and publish it.
    fn publish_stage(&self) {
        let stage = deepest_stage(self.notifications.iter().map(|n| (n.stage, n.idled)));
        let _ = self.stage_tx.send(stage);
    }

    /// Tear down the current notifications and re-arm from `cfg`.
    fn apply_config(&mut self, qh: &QueueHandle<AppData>, cfg: &IdleConfig) {
        for n in self.notifications.drain(..) {
            n.object.destroy();
        }
        let (Some(notifier), Some(seat)) = (&self.notifier, &self.seat) else {
            return;
        };
        for (stage, timeout_ms) in stage_timeouts(cfg) {
            let object = notifier.get_idle_notification(timeout_ms, seat, qh, stage);
            self.notifications.push(Notification {
                stage,
                object,
                idled: false,
            });
        }
        let _ = self.stage_tx.send(IdleStage::Active);
    }
}

/// The deepest stage among `(stage, idled)` pairs that have idled;
/// `Active` when none has.
fn deepest_stage(stages: impl Iterator<Item = (IdleStage, bool)>) -> IdleStage {
    stages
        .filter(|(_, idled)| *idled)
        .map(|(stage, _)| stage)
        .max_by_key(|s| s.rank())
        .unwrap_or(IdleStage::Active)
}

/// The enabled stages of `cfg` with their ext-idle-notify timeouts in
/// milliseconds, in dim → lock → suspend order. `None` and `Some(0)`
/// disable a stage.
fn stage_timeouts(cfg: &IdleConfig) -> Vec<(IdleStage, u32)> {
    [
        (IdleStage::Dim, cfg.dim_minutes),
        (IdleStage::Lock, cfg.lock_minutes),
        (IdleStage::Suspend, cfg.suspend_minutes),
    ]
    .into_iter()
    .filter_map(|(stage, minutes)| {
        let minutes = minutes.filter(|m| *m > 0)?;
        // Saturate (~49.7 days) rather than wrap to a tiny timeout.
        Some((stage, minutes.saturating_mul(60_000)))
    })
    .collect()
}

impl Dispatch<wl_registry::WlRegistry, ()> for AppData {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "ext_idle_notifier_v1" => {
                state.notifier = Some(
                    registry.bind::<ext_idle_notifier_v1::ExtIdleNotifierV1, _, _>(
                        name,
                        version.min(2),
                        qh,
                        (),
                    ),
                );
            }
            "wl_seat"
                // Only need the first seat — the keyboard/pointer seat.
                if state.seat.is_none() => {
                    state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(
                        name,
                        version.min(5),
                        qh,
                        (),
                    ));
                }
            _ => {}
        }
    }
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, IdleStage> for AppData {
    fn event(
        state: &mut Self,
        _object: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        stage: &IdleStage,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let idled = match event {
            ext_idle_notification_v1::Event::Idled => true,
            ext_idle_notification_v1::Event::Resumed => false,
            _ => return,
        };
        if let Some(n) = state.notifications.iter_mut().find(|n| n.stage == *stage) {
            n.idled = idled;
        }
        state.publish_stage();
    }
}

// The notifier + seat have no events we care about.
delegate_noop!(AppData: ignore ext_idle_notifier_v1::ExtIdleNotifierV1);
delegate_noop!(AppData: ignore wl_seat::WlSeat);

fn run(mut cfg_rx: watch::Receiver<IdleConfig>, stage_tx: watch::Sender<IdleStage>) -> Result<()> {
    let conn = Connection::connect_to_env().context("connect to Wayland")?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());

    let mut data = AppData {
        notifier: None,
        seat: None,
        notifications: Vec::new(),
        stage_tx,
    };

    // First roundtrip: discover the notifier + seat globals.
    queue.roundtrip(&mut data).context("Wayland roundtrip")?;
    if data.notifier.is_none() {
        anyhow::bail!("compositor does not advertise ext_idle_notifier_v1");
    }

    let initial = *cfg_rx.borrow_and_update();
    data.apply_config(&qh, &initial);
    conn.flush().ok();

    loop {
        queue.dispatch_pending(&mut data)?;
        conn.flush().ok();

        // Block on the Wayland fd with a 1 s timeout so we also
        // notice `IdleConfig` updates without a dedicated wake fd.
        let Some(guard) = conn.prepare_read() else {
            queue.dispatch_pending(&mut data)?;
            continue;
        };
        let raw_fd = guard.connection_fd().as_raw_fd();
        let mut pollfd = [libc::pollfd {
            fd: raw_fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        let n = unsafe { libc::poll(pollfd.as_mut_ptr(), 1, 1000) };
        if n > 0 && (pollfd[0].revents & libc::POLLIN) != 0 {
            let _ = guard.read();
        } else {
            drop(guard);
        }
        queue.dispatch_pending(&mut data)?;

        // Re-arm on a timeout change.
        if cfg_rx.has_changed().unwrap_or(false) {
            let cfg = *cfg_rx.borrow_and_update();
            data.apply_config(&qh, &cfg);
            conn.flush().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(dim: Option<u32>, lock: Option<u32>, suspend: Option<u32>) -> IdleConfig {
        IdleConfig {
            dim_minutes: dim,
            lock_minutes: lock,
            suspend_minutes: suspend,
        }
    }

    #[test]
    fn timeouts_are_minutes_in_milliseconds_in_stage_order() {
        assert_eq!(
            stage_timeouts(&cfg(Some(5), Some(10), Some(30))),
            vec![
                (IdleStage::Dim, 300_000),
                (IdleStage::Lock, 600_000),
                (IdleStage::Suspend, 1_800_000),
            ]
        );
    }

    #[test]
    fn none_and_zero_disable_a_stage() {
        assert_eq!(stage_timeouts(&IdleConfig::default()), vec![]);
        assert_eq!(
            stage_timeouts(&cfg(Some(0), None, Some(1))),
            vec![(IdleStage::Suspend, 60_000)]
        );
    }

    #[test]
    fn huge_timeouts_saturate_instead_of_wrapping() {
        // 71_583 min × 60_000 overflows u32: it wrapped to ~12.7 s in a
        // release build (dimming almost at once) and panicked in debug.
        assert_eq!(
            stage_timeouts(&cfg(Some(71_583), None, Some(u32::MAX))),
            vec![(IdleStage::Dim, u32::MAX), (IdleStage::Suspend, u32::MAX)]
        );
        assert_eq!(
            stage_timeouts(&cfg(None, Some(71_582), None)),
            vec![(IdleStage::Lock, 71_582 * 60_000)]
        );
    }

    #[test]
    fn deepest_idled_stage_wins() {
        use IdleStage::*;
        assert_eq!(deepest_stage([].into_iter()), Active);
        assert_eq!(
            deepest_stage([(Dim, false), (Lock, false)].into_iter()),
            Active
        );
        assert_eq!(deepest_stage([(Dim, true), (Lock, false)].into_iter()), Dim);
        assert_eq!(
            deepest_stage([(Dim, true), (Lock, true), (Suspend, false)].into_iter()),
            Lock
        );
        // Order of the notifications doesn't matter, only their rank.
        assert_eq!(
            deepest_stage([(Suspend, true), (Dim, true)].into_iter()),
            Suspend
        );
        // A later stage idling while an earlier one resumed still counts.
        assert_eq!(
            deepest_stage([(Dim, false), (Lock, true)].into_iter()),
            Lock
        );
    }

    #[test]
    fn rank_orders_stages() {
        use IdleStage::*;
        assert!(Active.rank() < Dim.rank());
        assert!(Dim.rank() < Lock.rank());
        assert!(Lock.rank() < Suspend.rank());
    }
}
