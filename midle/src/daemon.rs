//! Daemon entry point. Two concurrent tasks:
//!   • Wayland thread — owns the connection + event_queue, creates
//!     one `ext_idle_notification_v1` per configured step. Events
//!     are pushed into a tokio mpsc channel.
//!   • Async runtime — tokio multi-thread; owns the manager state,
//!     reacts to Wayland events + IPC connections + signals.

use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{Mutex, mpsc};
use tracing::{error, info, warn};

use crate::config::{self, Config};
use crate::ipc::{self, DaemonInfo, Request, Response, StepInfo};
use crate::state::{Manager, PauseState};

#[derive(Debug, Clone)]
pub enum WaylandEvent {
    StepIdled(usize),
    Active,
    /// `inhibit_apps` regex matched a process (or stopped matching).
    AppInhibit(Option<String>),
    /// At least one audio stream is RUNNING (or all stopped).
    MediaInhibit(bool),
    /// Logind told us a suspend is starting / resuming.
    PrepareForSleep(bool),
}

pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let cfg = config::load(config_path.as_deref())?;
    info!(
        steps = cfg.steps.len(),
        "midle daemon starting"
    );

    let manager = Arc::new(Mutex::new(Manager::new(cfg.clone())));

    // Wayland thread — synchronous so the protocol calls stay
    // simple. Events are forwarded to tokio via mpsc. The channel
    // is shared with inhibitor tasks below.
    let (wl_tx, wl_rx) = mpsc::channel::<WaylandEvent>(64);
    let wl_tx_async = wl_tx.clone();
    let timeouts: Vec<Duration> = cfg.steps.iter().map(|s| s.timeout).collect();
    let wayland_join = std::thread::Builder::new()
        .name("midle-wayland".to_string())
        .spawn(move || {
            if let Err(e) = wayland_loop(timeouts, wl_tx) {
                error!("wayland loop crashed: {e:#}");
            }
        })?;

    // Tokio runtime.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime")?;
    rt.block_on(async {
        let stop = Arc::new(tokio::sync::Notify::new());
        let stop_for_ipc = stop.clone();
        let stop_for_wl = stop.clone();
        let stop_for_sig = stop.clone();

        let manager_wl = manager.clone();
        let mut wl_rx = wl_rx;
        let wl_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(evt) = wl_rx.recv() => {
                        let mut m = manager_wl.lock().await;
                        match evt {
                            WaylandEvent::StepIdled(idx) => m.on_step_idled(idx),
                            WaylandEvent::Active => m.on_active(),
                            WaylandEvent::AppInhibit(name) => {
                                m.inhibit_app = name;
                            }
                            WaylandEvent::MediaInhibit(playing) => {
                                m.inhibit_media = playing;
                            }
                            WaylandEvent::PrepareForSleep(true) => {
                                // Pre-emptively treat as active wake-up
                                // upon resume: reset fired bitmap.
                                tracing::info!("prepare-for-sleep: pre-suspend");
                            }
                            WaylandEvent::PrepareForSleep(false) => {
                                tracing::info!("resume from sleep — clearing fired steps");
                                m.on_active();
                            }
                        }
                    }
                    _ = stop_for_wl.notified() => break,
                }
            }
        });

        // Inhibitor tasks — each one runs forever and pushes events
        // through the same channel.
        let inhibit_apps = cfg.settings.inhibit_apps.clone();
        let scan_interval = cfg.settings.inhibit_scan_interval;
        let monitor_media = cfg.settings.monitor_media;
        let prepare_sleep = cfg.settings.prepare_sleep_command.clone();
        let tx_app = wl_tx_async.clone();
        let tx_media = wl_tx_async.clone();
        let tx_sleep = wl_tx_async.clone();
        let app_task = tokio::spawn(async move {
            crate::inhibitors::app_scan_loop(inhibit_apps, scan_interval, tx_app).await;
        });
        let media_task = if monitor_media {
            Some(tokio::spawn(async move {
                crate::inhibitors::media_scan_loop(scan_interval, tx_media).await;
            }))
        } else {
            None
        };
        let logind_task = tokio::spawn(async move {
            crate::inhibitors::logind_loop(prepare_sleep, tx_sleep).await;
        });

        let manager_ipc = manager.clone();
        let ipc_task = tokio::spawn(async move {
            if let Err(e) = serve_ipc(manager_ipc, stop_for_ipc).await {
                error!("ipc loop crashed: {e:#}");
            }
        });

        // SIGHUP → reload, SIGINT/SIGTERM → stop.
        let manager_sig = manager.clone();
        let sig_task = tokio::spawn(async move {
            use tokio::signal::unix::{SignalKind, signal};
            let mut hup = signal(SignalKind::hangup()).expect("install SIGHUP");
            let mut int = signal(SignalKind::interrupt()).expect("install SIGINT");
            let mut term = signal(SignalKind::terminate()).expect("install SIGTERM");
            loop {
                tokio::select! {
                    _ = hup.recv() => {
                        info!("SIGHUP — reloading config");
                        match config::load(None) {
                            Ok(c) => manager_sig.lock().await.replace_config(c),
                            Err(e) => error!("reload failed: {e:#}"),
                        }
                    }
                    _ = int.recv() => {
                        info!("SIGINT — stopping");
                        stop_for_sig.notify_waiters();
                        break;
                    }
                    _ = term.recv() => {
                        info!("SIGTERM — stopping");
                        stop_for_sig.notify_waiters();
                        break;
                    }
                }
            }
        });

        // Wait for stop signal.
        stop.notified().await;
        wl_task.abort();
        ipc_task.abort();
        sig_task.abort();
    });

    // Best-effort join — the Wayland thread loops on
    // blocking_dispatch and will only exit when the connection
    // drops. The runtime tear-down will signal that.
    drop(wayland_join);
    info!("midle daemon exited");
    Ok(())
}

async fn serve_ipc(manager: Arc<Mutex<Manager>>, stop: Arc<tokio::sync::Notify>) -> Result<()> {
    let path = ipc::socket_path();
    // Stale socket from a previous run.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("bind {}", path.display()))?;
    info!(socket = %path.display(), "IPC listening");

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (sock, _) = match accept {
                    Ok(p) => p,
                    Err(e) => { warn!("accept: {e}"); continue; }
                };
                let mgr = manager.clone();
                let stop = stop.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_ipc_connection(sock, mgr, stop).await {
                        warn!("ipc conn: {e:#}");
                    }
                });
            }
            _ = stop.notified() => break,
        }
    }
    let _ = std::fs::remove_file(&path);
    Ok(())
}

async fn handle_ipc_connection(
    sock: tokio::net::UnixStream,
    manager: Arc<Mutex<Manager>>,
    stop: Arc<tokio::sync::Notify>,
) -> Result<()> {
    let (rd, mut wr) = sock.into_split();
    let mut reader = BufReader::new(rd);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let req: Request = serde_json::from_str(line.trim())
        .map_err(|e| anyhow!("bad request {line:?}: {e}"))?;

    let resp = match req {
        Request::Info => {
            let m = manager.lock().await;
            Response::Ok {
                info: Some(snapshot(&m)),
            }
        }
        Request::Pause { duration } => {
            let dur = duration
                .as_deref()
                .map(config::parse_duration)
                .transpose()
                .map_err(|e| anyhow!("invalid duration: {e}"))?;
            manager.lock().await.pause_for(dur);
            Response::Ok { info: None }
        }
        Request::Resume => {
            manager.lock().await.resume_from_pause();
            Response::Ok { info: None }
        }
        Request::ToggleInhibit => {
            manager.lock().await.toggle_inhibit();
            Response::Ok { info: None }
        }
        Request::Reload => match config::load(None) {
            Ok(c) => {
                manager.lock().await.replace_config(c);
                Response::Ok { info: None }
            }
            Err(e) => Response::Err {
                message: format!("{e:#}"),
            },
        },
        Request::Stop => {
            stop.notify_waiters();
            Response::Ok { info: None }
        }
    };

    let body = serde_json::to_string(&resp)?;
    wr.write_all(body.as_bytes()).await?;
    wr.write_all(b"\n").await?;
    Ok(())
}

fn snapshot(m: &Manager) -> DaemonInfo {
    let pause = match m.pause {
        PauseState::Running => "running".to_string(),
        PauseState::Indefinite => "paused (indefinite)".to_string(),
        PauseState::UntilInstant(t) => {
            let remaining = t.saturating_duration_since(std::time::Instant::now());
            format!("paused ({}s left)", remaining.as_secs())
        }
    };
    DaemonInfo {
        running: !m.is_suppressed(),
        inhibit: m.inhibit,
        pause,
        steps: m
            .cfg
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| StepInfo {
                name: s.name.clone(),
                timeout_seconds: s.timeout.as_secs(),
                fired: *m.fired.get(i).unwrap_or(&false),
            })
            .collect(),
    }
}

// ── Wayland — synchronous side ───────────────────────────────────────

fn wayland_loop(timeouts: Vec<Duration>, tx: mpsc::Sender<WaylandEvent>) -> Result<()> {
    use wayland_client::Connection;
    use wayland_client::globals::registry_queue_init;

    let conn = Connection::connect_to_env().context("wayland connect")?;
    let (globals, mut event_queue) = registry_queue_init::<WaylandState>(&conn)?;
    let qh = event_queue.handle();

    let seat: wayland_client::protocol::wl_seat::WlSeat = globals
        .bind(&qh, 1..=8, ())
        .context("wl_seat global missing")?;

    use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1;
    let notifier: ext_idle_notifier_v1::ExtIdleNotifierV1 = globals
        .bind(&qh, 1..=1, ())
        .context("ext_idle_notifier_v1 unsupported — compositor needs ext-idle-notify-v1")?;

    let mut state = WaylandState { tx, seat: seat.clone() };

    // Create one notification per step.
    let notifications: Vec<_> = timeouts
        .iter()
        .enumerate()
        .map(|(idx, dur)| {
            let ms = dur.as_millis().min(u32::MAX as u128) as u32;
            notifier.get_idle_notification(ms, &seat, &qh, idx)
        })
        .collect();
    info!(count = notifications.len(), "idle notifications armed");

    loop {
        event_queue.blocking_dispatch(&mut state)?;
    }
}

struct WaylandState {
    tx: mpsc::Sender<WaylandEvent>,
    seat: wayland_client::protocol::wl_seat::WlSeat,
}

use wayland_client::{Dispatch, QueueHandle};
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &wayland_client::Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_idle_notifier_v1::ExtIdleNotifierV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ext_idle_notifier_v1::ExtIdleNotifierV1,
        _: ext_idle_notifier_v1::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, usize> for WaylandState {
    fn event(
        state: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        idx: &usize,
        _: &wayland_client::Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                let _ = state.tx.blocking_send(WaylandEvent::StepIdled(*idx));
            }
            ext_idle_notification_v1::Event::Resumed => {
                let _ = state.tx.blocking_send(WaylandEvent::Active);
            }
            _ => {}
        }
        let _ = &state.seat;
    }
}
