//! Presence — "is the user at the keyboard?" via one `ext-idle-notify-v1`
//! notification with a caller-chosen timeout.
//!
//! The idle manager ([`crate::idle_manager`]) arms the dim / lock /
//! suspend stages from the user's idle config; this is a separate, tiny
//! client for features that need their own threshold (the eye-care break
//! timer treats "no input for N seconds" as the user being away). It runs
//! its own Wayland connection on a dedicated thread that blocks on the
//! socket — no periodic wake-ups — and publishes `true` (away) / `false`
//! (present) on a `watch` channel. Dropping the [`Presence`] handle wakes
//! the thread through a socket pair and tears the notification down.
//!
//! `get_idle_notification` honours idle inhibitors, so a playing video
//! (which holds one) keeps the user "present" — correct for eye care:
//! their eyes are on the screen.

use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::watch;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};

/// A running presence watcher. Drop it to stop the thread.
pub struct Presence {
    away: watch::Receiver<bool>,
    stop: UnixStream,
}

impl Presence {
    /// `true` while the user has been idle for at least the timeout.
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.away.clone()
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        // Any byte (or the hang-up when `stop` closes) wakes the poll.
        let _ = self.stop.write_all(&[1]);
    }
}

/// The notification timeout for `away_after`, in ms: at least one second,
/// saturating instead of wrapping (~49.7 days).
pub fn timeout_ms(away_after: Duration) -> u32 {
    u32::try_from(away_after.as_millis())
        .unwrap_or(u32::MAX)
        .max(1_000)
}

/// Start watching for `away_after` of no input.
pub fn watch_presence(away_after: Duration) -> Result<Presence> {
    let (stop, wake) = UnixStream::pair().context("presence: socket pair")?;
    let (away_tx, away_rx) = watch::channel(false);
    let timeout = timeout_ms(away_after);
    std::thread::Builder::new()
        .name("mshell-presence".into())
        .spawn(move || {
            if let Err(e) = run(timeout, away_tx, wake) {
                tracing::warn!(error = %e, "presence watcher stopped");
            }
        })
        .context("presence: spawn thread")?;
    Ok(Presence {
        away: away_rx,
        stop,
    })
}

struct AppData {
    notifier: Option<ext_idle_notifier_v1::ExtIdleNotifierV1>,
    seat: Option<wl_seat::WlSeat>,
    away_tx: watch::Sender<bool>,
}

fn run(timeout: u32, away_tx: watch::Sender<bool>, wake: UnixStream) -> Result<()> {
    let conn = Connection::connect_to_env().context("connect to Wayland")?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut data = AppData {
        notifier: None,
        seat: None,
        away_tx,
    };
    queue.roundtrip(&mut data).context("Wayland roundtrip")?;
    let (Some(notifier), Some(seat)) = (data.notifier.clone(), data.seat.clone()) else {
        anyhow::bail!("compositor does not advertise ext_idle_notifier_v1 / wl_seat");
    };
    let notification = notifier.get_idle_notification(timeout, &seat, &qh, ());
    conn.flush().ok();

    let result = loop {
        if let Err(e) = queue.dispatch_pending(&mut data) {
            break Err(e.into());
        }
        conn.flush().ok();
        let Some(guard) = conn.prepare_read() else {
            continue;
        };
        let mut fds = [
            libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: `fds` is a valid array of two initialised pollfd structs
        // for the duration of the call; both fds stay open (the guard and
        // `wake` outlive it). Blocks until Wayland traffic or a stop.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if n < 0 {
            drop(guard);
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break Err(anyhow::anyhow!("poll failed"));
        }
        if fds[1].revents != 0 {
            drop(guard);
            break Ok(()); // the Presence handle was dropped
        }
        if fds[0].revents & libc::POLLIN != 0 {
            if let Err(e) = guard.read() {
                break Err(e.into());
            }
        } else if fds[0].revents != 0 {
            drop(guard);
            break Err(anyhow::anyhow!("Wayland connection closed"));
        } else {
            drop(guard);
        }
    };
    notification.destroy();
    conn.flush().ok();
    result
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
                state.notifier = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "wl_seat" if state.seat.is_none() => {
                state.seat = Some(registry.bind(name, version.min(5), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for AppData {
    fn event(
        state: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let away = match event {
            ext_idle_notification_v1::Event::Idled => true,
            ext_idle_notification_v1::Event::Resumed => false,
            _ => return,
        };
        state
            .away_tx
            .send_if_modified(|v| std::mem::replace(v, away) != away);
    }
}

delegate_noop!(AppData: ignore ext_idle_notifier_v1::ExtIdleNotifierV1);
delegate_noop!(AppData: ignore wl_seat::WlSeat);

#[cfg(test)]
mod tests {
    use super::*;

    /// Live check against the running compositor (needs `WAYLAND_DISPLAY`):
    /// the watcher binds, starts "present", and its thread exits when the
    /// handle drops (the receiver then sees the sender close). Read-only:
    /// it only arms an idle notification. `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs a Wayland session with ext-idle-notify"]
    fn live_watcher_starts_present_and_stops_on_drop() {
        let presence = watch_presence(Duration::from_secs(600)).expect("watcher starts");
        let mut rx = presence.subscribe();
        std::thread::sleep(Duration::from_millis(300));
        assert!(!*rx.borrow(), "freshly armed: present");
        drop(presence);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let closed = rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while rx.changed().await.is_ok() {}
            })
            .await
        });
        assert!(closed.is_ok(), "the thread stopped and dropped its sender");
    }

    #[test]
    fn the_timeout_is_whole_milliseconds_with_a_floor_and_no_wrap() {
        assert_eq!(timeout_ms(Duration::from_secs(120)), 120_000);
        assert_eq!(timeout_ms(Duration::from_millis(10)), 1_000);
        assert_eq!(timeout_ms(Duration::from_secs(60 * 60 * 24 * 60)), u32::MAX);
    }
}
