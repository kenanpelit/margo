use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::runtime::Runtime;
use tracing::{info, warn};

static TOKIO_RT: OnceLock<Runtime> = OnceLock::new();

/// The same tokio runtime that `init_services` ran on. Watchers that
/// observe `wayle_*::Property` channels must spawn on this runtime —
/// wayle's monitoring tasks live here, and `tokio::sync::watch`
/// wakeups don't propagate reliably across runtimes (we see only the
/// initial value if we poll from relm4's private runtime; subsequent
/// `replace`/`set` updates are missed).
pub fn tokio_rt() -> &'static Runtime {
    TOKIO_RT.get_or_init(|| {
        // The wayle services this drives are I/O-bound (D-Bus signal
        // streams, periodic polls) — not CPU-bound. `Runtime::new()`
        // sizes the worker pool to the CPU count (22 threads on this
        // box), which is wasted scheduling + thread overhead for a
        // desktop shell; 4 async workers comfortably multiplex every
        // service's monitoring task. Named so its threads are
        // attributable in `/proc/<pid>/task/*/comm`.
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .thread_name("mshell-tokio")
            .build()
            .expect("tokio runtime")
    })
}

/// Convenience wrapper around `tokio_rt().spawn(...)`.
pub fn tokio_rt_spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio_rt().spawn(future)
}
pub mod audio_groups;
pub mod audio_watchdog;
pub mod bluetooth;
pub mod login_net;
pub mod mpd;
pub mod mtune;
pub mod systemd;

use mshell_margo_client::MargoService;
use wayle_audio::AudioService;
use wayle_battery::BatteryService;
use wayle_bluetooth::BluetoothService;
use wayle_brightness::BrightnessService;
use wayle_media::MediaService;
use wayle_network::NetworkService;
use wayle_notification::NotificationService;
use wayle_power_profiles::PowerProfilesService;
use wayle_sysinfo::SysinfoService;
use wayle_systray::SystemTrayService;
use wayle_weather::{LocationQuery, TemperatureUnit, WeatherService, WeatherServiceBuilder};
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Error};

/// How long one service may take to come up before the shell starts without
/// it. A wedged system bus or a stuck activation otherwise held the first
/// paint for the whole D-Bus timeout (~25 s).
const SERVICE_TIMEOUT: Duration = Duration::from_secs(5);

/// How one service's start went.
#[derive(Debug, PartialEq, Eq)]
enum Started<T> {
    Ready(T),
    Failed(String),
    TimedOut,
}

impl<T> Started<T> {
    fn ok(self) -> Option<T> {
        match self {
            Started::Ready(v) => Some(v),
            Started::Failed(_) | Started::TimedOut => None,
        }
    }
}

/// Start one service with a timeout, timing it and logging a failure.
///
/// `init_services` is awaited from the GTK main thread *before* the first bar
/// is painted, and the services start concurrently, so the blank-screen
/// window at login is the slowest of them — now bounded by `limit`. A
/// failure or timeout of one service no longer stops the others or the
/// shell; the caller decides what a missing service means.
async fn start<T, E: std::fmt::Display>(
    name: &'static str,
    limit: Duration,
    fut: impl std::future::Future<Output = Result<T, E>>,
) -> Started<T> {
    let started = std::time::Instant::now();
    let out = match tokio::time::timeout(limit, fut).await {
        Ok(Ok(v)) => Started::Ready(v),
        Ok(Err(e)) => Started::Failed(format!("{e:#}")),
        Err(_) => Started::TimedOut,
    };
    let ms = started.elapsed().as_millis() as u64;
    match &out {
        Started::Ready(_) => info!(service = name, ms, "service ready"),
        Started::Failed(e) => {
            warn!(service = name, ms, error = %e, "service unavailable; continuing without it")
        }
        Started::TimedOut => warn!(
            service = name,
            ms, "service did not start in time; continuing without it"
        ),
    }
    out
}

/// margo is the one hard dependency: workspaces, clients and outputs come
/// from its IPC, and mshell only makes sense inside a margo session. Its
/// failure gets an error that says so instead of a bare join error.
fn require_margo<T>(margo: Started<T>) -> anyhow::Result<T> {
    match margo {
        Started::Ready(v) => Ok(v),
        Started::Failed(e) => Err(anyhow::anyhow!(
            "margo IPC failed to start: {e}. mshell needs a running margo session \
             (check $MARGO_SOCKET / $XDG_RUNTIME_DIR/margo/margo-ipc.sock)"
        )),
        Started::TimedOut => Err(anyhow::anyhow!(
            "margo IPC did not answer within {}s. mshell needs a running, responsive margo session \
             (check $MARGO_SOCKET / $XDG_RUNTIME_DIR/margo/margo-ipc.sock)",
            SERVICE_TIMEOUT.as_secs()
        )),
    }
}

/// The one toast (title, body) shown when the shell started without audio
/// and/or media, or `None` when both are up.
fn degraded_notice(audio: bool, media: bool) -> Option<(String, String)> {
    let mut missing = Vec::new();
    let mut effects = Vec::new();
    if !audio {
        missing.push("audio");
        effects.push("volume, mic and mixer controls are hidden (PipeWire/PulseAudio unavailable)");
    }
    if !media {
        missing.push("media");
        effects.push("media controls are hidden (MPRIS unavailable)");
    }
    if missing.is_empty() {
        return None;
    }
    let mut body = effects.join("; ");
    body.push_str(". Restart mshell once it is back.");
    Some((format!("Started without {}", missing.join(" and ")), body))
}

/// Deliver the degraded-start toast once a toast surface exists (they are
/// created after `init_services` returns), giving up after a minute.
fn spawn_degraded_toast(title: String, body: String) {
    tokio::spawn(async move {
        for _ in 0..60 {
            let delivered =
                mshell_config::toast::try_push_toast(mshell_config::toast::ToastEvent {
                    icon: "dialog-warning-symbolic".to_string(),
                    title: title.clone(),
                    body: Some(body.clone()),
                    severity: mshell_config::toast::ToastSeverity::Warn,
                });
            if delivered {
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

pub async fn init_services(
    location_query: LocationQuery,
    temperature_unit: TemperatureUnit,
    weather_poll_minutes: u32,
) -> anyhow::Result<()> {
    info!("Initializing services...");
    let init_started = std::time::Instant::now();
    let t = SERVICE_TIMEOUT;
    let line_power_fut = async {
        if let Some(path) = find_line_power_path().await? {
            Ok::<_, anyhow::Error>(Some(
                BatteryService::builder().device_path(path).build().await?,
            ))
        } else {
            Ok(None)
        }
    };

    // Every service starts on its own: one failing or hanging peer (no
    // NetworkManager under iwd, no UPower in a VM, a stuck BlueZ activation)
    // used to abort the whole `try_join!` and the shell never painted.
    let (
        audio,
        battery,
        bluetooth,
        brightness,
        margo,
        line_power,
        media,
        network,
        notifications,
        power_profiles,
        systray,
    ) = tokio::join!(
        start("audio", t, AudioService::new()),
        start("battery", t, BatteryService::new()),
        // Needs a running `org.bluez` (a VM without an adapter has none).
        start("bluetooth", t, BluetoothService::new()),
        // `Ok(None)` when there's simply no backlight device.
        start("brightness", t, BrightnessService::new()),
        start("margo", t, MargoService::new()),
        start("line_power", t, line_power_fut),
        // `with_art_cache()` resolves MPRIS `mpris:artUrl`s (incl. remote
        // http(s) covers from Spotify / browsers) to local files on
        // `TrackMetadata::cover_art`, which the media widgets render.
        start("media", t, MediaService::builder().with_art_cache().build()),
        start("network", t, NetworkService::new()),
        // Fails when another daemon (dunst / mako) owns the name.
        start("notifications", t, NotificationService::new()),
        // Needs power-profiles-daemon.
        start("power_profiles", t, PowerProfilesService::new()),
        // Fails when another tray already owns the StatusNotifier host name.
        start("systray", t, SystemTrayService::builder().build()),
    );
    // The bar cannot paint until this line is reached — the caller awaits us on
    // the GTK main thread. Compare this against the slowest `service ready`
    // line above to see whether the barrier is one slow peer or all of them.
    info!(
        ms = init_started.elapsed().as_millis() as u64,
        "all services ready"
    );

    // margo is the only hard dependency. Audio (PipeWire/PulseAudio) and
    // media (MPRIS) are optional like the rest: their widgets hide, and
    // one toast says what is missing.
    let margo = require_margo(margo)?;
    let (audio, media) = (audio.ok(), media.ok());
    if let Some((title, body)) = degraded_notice(audio.is_some(), media.is_some()) {
        warn!(%title, %body, "starting in degraded mode");
        spawn_degraded_toast(title, body);
    }

    let sysinfo = SysinfoService::builder().build();
    let weather = WeatherServiceBuilder::new()
        .poll_interval(Duration::from_mins(weather_poll_minutes.max(1) as u64))
        .location(location_query)
        .units(temperature_unit)
        .build();

    AUDIO_SERVICE.set(audio).ok();
    BATTERY_SERVICE.set(battery.ok().map(Arc::new)).ok();
    BLUETOOTH_SERVICE.set(bluetooth.ok().map(Arc::new)).ok();
    BRIGHTNESS_SERVICE.set(brightness.ok().flatten()).ok();
    MARGO_SERVICE.set(margo).ok();
    spawn_margo_sync_health_toast();
    LINE_POWER_SERVICE
        .set(line_power.ok().flatten().map(Arc::new))
        .ok();
    MEDIA_SERVICE.set(media).ok();
    NETWORK_SERVICE.set(network.ok().map(Arc::new)).ok();
    NOTIFICATION_SERVICE.set(notifications.ok()).ok();
    POWER_PROFILE_SERVICE.set(power_profiles.ok()).ok();
    SYS_INFO_SERVICE.set(Arc::new(sysinfo)).ok();
    SYS_TRAY_SERVICE.set(systray.ok()).ok();
    WEATHER_SERVICE.set(Arc::new(weather)).ok();

    info!("Done");

    Ok(())
}

/// Toast when margo's state frames stop parsing (a margo/mshell version skew
/// after an upgrade): otherwise tags, focus and the dock freeze at the last
/// good state with nothing on screen saying why. Once per degraded spell.
fn spawn_margo_sync_health_toast() {
    let mut rx = mshell_margo_client::sync_degraded();
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            if *rx.borrow_and_update() {
                mshell_config::toast::push_toast(mshell_config::toast::ToastEvent {
                    icon: "dialog-warning-symbolic".to_string(),
                    title: "margo state not understood".to_string(),
                    body: Some(
                        "The bar shows the last good state — restart mshell after upgrading margo"
                            .to_string(),
                    ),
                    severity: mshell_config::toast::ToastSeverity::Warn,
                });
            }
        }
    });
}

pub async fn find_line_power_path() -> Result<Option<OwnedObjectPath>, Error> {
    let connection = Connection::system().await?;

    let reply = connection
        .call_method(
            Some("org.freedesktop.UPower"),
            "/org/freedesktop/UPower",
            Some("org.freedesktop.UPower"),
            "EnumerateDevices",
            &(),
        )
        .await?;

    let devices: Vec<OwnedObjectPath> = reply.body().deserialize()?;

    Ok(devices
        .into_iter()
        .find(|p| p.as_str().contains("line_power")))
}

static AUDIO_SERVICE: OnceLock<Option<Arc<AudioService>>> = OnceLock::new();

/// `None` when PipeWire/PulseAudio was unavailable at startup: audio
/// widgets hide and the volume/mic IPC commands do nothing.
pub fn audio_service() -> Option<Arc<AudioService>> {
    AUDIO_SERVICE
        .get()
        .expect("AudioService not initialized")
        .clone()
}

static BATTERY_SERVICE: OnceLock<Option<Arc<BatteryService>>> = OnceLock::new();

/// The UPower battery service, or `None` when UPower wasn't reachable at
/// startup (a VM / container). Callers hide battery UI.
pub fn battery_service() -> Option<Arc<BatteryService>> {
    BATTERY_SERVICE
        .get()
        .expect("BatteryService not initialized")
        .clone()
}

/// A point-in-time battery reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatteryReading {
    pub present: bool,
    pub percentage: f64,
    pub state: wayle_battery::types::DeviceState,
}

/// The battery's presence, charge and state; "no battery" without UPower.
pub fn battery_reading() -> BatteryReading {
    match battery_service() {
        Some(b) => BatteryReading {
            present: b.device.is_present.get(),
            percentage: b.device.percentage.get(),
            state: b.device.state.get(),
        },
        None => BatteryReading {
            present: false,
            percentage: 0.0,
            state: wayle_battery::types::DeviceState::Unknown,
        },
    }
}

static BLUETOOTH_SERVICE: OnceLock<Option<Arc<BluetoothService>>> = OnceLock::new();

/// The Bluetooth service, or `None` when no usable adapter / `org.bluez` was
/// found at startup (e.g. a VM with no Bluetooth hardware). Callers degrade
/// gracefully — the bar pill hides, watchers no-op, menus show "no adapter".
pub fn bluetooth_service() -> Option<Arc<BluetoothService>> {
    BLUETOOTH_SERVICE
        .get()
        .expect("BluetoothService not initialized")
        .clone()
}

static BRIGHTNESS_SERVICE: OnceLock<Option<Arc<BrightnessService>>> = OnceLock::new();

pub fn brightness_service() -> Option<Arc<BrightnessService>> {
    BRIGHTNESS_SERVICE
        .get()
        .expect("BrightnessService not initialized")
        .clone()
}

static MARGO_SERVICE: OnceLock<Arc<MargoService>> = OnceLock::new();

pub fn margo_service() -> Arc<MargoService> {
    MARGO_SERVICE
        .get()
        .expect("MargoService not initialized")
        .clone()
}

static LINE_POWER_SERVICE: OnceLock<Option<Arc<BatteryService>>> = OnceLock::new();

pub fn line_power_service() -> Option<Arc<BatteryService>> {
    LINE_POWER_SERVICE
        .get()
        .expect("LinePower not initialized")
        .clone()
}

static MEDIA_SERVICE: OnceLock<Option<Arc<MediaService>>> = OnceLock::new();

/// `None` when MPRIS was unavailable at startup: media widgets see no
/// players and stay hidden.
pub fn media_service() -> Option<Arc<MediaService>> {
    MEDIA_SERVICE
        .get()
        .expect("MediaService not initialized")
        .clone()
}

static NETWORK_SERVICE: OnceLock<Option<Arc<NetworkService>>> = OnceLock::new();

/// The NetworkManager service, or `None` when NetworkManager isn't running
/// (iwd / systemd-networkd hosts). Callers hide network UI.
pub fn network_service() -> Option<Arc<NetworkService>> {
    NETWORK_SERVICE
        .get()
        .expect("NetworkService not initialized")
        .clone()
}

static NOTIFICATION_SERVICE: OnceLock<Option<Arc<NotificationService>>> = OnceLock::new();

/// The notification service, or `None` when its D-Bus name couldn't be claimed
/// at startup (another notification daemon owns it). Callers degrade — toasts
/// and the notification list are empty.
pub fn notification_service() -> Option<Arc<NotificationService>> {
    NOTIFICATION_SERVICE
        .get()
        .expect("NotificationService not initialized")
        .clone()
}

static POWER_PROFILE_SERVICE: OnceLock<Option<Arc<PowerProfilesService>>> = OnceLock::new();

/// The power-profiles service, or `None` when `power-profiles-daemon` isn't
/// available (not installed). Callers degrade — the profile pill and
/// control-center tile hide.
pub fn power_profile_service() -> Option<Arc<PowerProfilesService>> {
    POWER_PROFILE_SERVICE
        .get()
        .expect("PowerProfilesService not initialized")
        .clone()
}

static SYS_INFO_SERVICE: OnceLock<Arc<SysinfoService>> = OnceLock::new();

pub fn sys_info_service() -> Arc<SysinfoService> {
    SYS_INFO_SERVICE
        .get()
        .expect("SysinfoService not initialized")
        .clone()
}

static SYS_TRAY_SERVICE: OnceLock<Option<Arc<SystemTrayService>>> = OnceLock::new();

/// The system-tray (StatusNotifier host) service, or `None` when the host
/// couldn't be stood up at startup (e.g. another tray already owns the name).
/// Callers degrade — the tray pill shows nothing.
pub fn sys_tray_service() -> Option<Arc<SystemTrayService>> {
    SYS_TRAY_SERVICE
        .get()
        .expect("SystemTrayService not initialized")
        .clone()
}

static WEATHER_SERVICE: OnceLock<Arc<WeatherService>> = OnceLock::new();

pub fn weather_service() -> Arc<WeatherService> {
    WEATHER_SERVICE
        .get()
        .expect("WeatherService not initialized")
        .clone()
}

#[cfg(test)]
mod start_tests {
    use super::*;

    #[tokio::test]
    async fn a_hanging_service_times_out_instead_of_blocking_startup() {
        let hang = std::future::pending::<Result<(), String>>();
        let out = start("hang", Duration::from_millis(20), hang).await;
        assert_eq!(out, Started::TimedOut);
    }

    #[tokio::test]
    async fn a_failing_service_is_isolated() {
        let (bad, good) = tokio::join!(
            start("bad", Duration::from_secs(1), async {
                Err::<u8, _>("ServiceUnknown")
            }),
            start("good", Duration::from_secs(1), async {
                Ok::<_, String>(7u8)
            }),
        );
        assert_eq!(bad, Started::Failed("ServiceUnknown".into()));
        assert_eq!(good.ok(), Some(7));
    }

    #[test]
    fn margo_is_required_with_a_clear_error() {
        assert_eq!(require_margo(Started::Ready(3u8)).unwrap(), 3);
        let failed = require_margo::<u8>(Started::Failed("connection refused".into()))
            .unwrap_err()
            .to_string();
        assert!(failed.contains("margo"), "{failed}");
        assert!(failed.contains("connection refused"), "{failed}");
        assert!(failed.contains("margo session"), "{failed}");
        let timed_out = require_margo::<u8>(Started::TimedOut)
            .unwrap_err()
            .to_string();
        assert!(timed_out.contains("did not answer"), "{timed_out}");
    }

    #[test]
    fn degraded_notice_names_every_missing_optional_service() {
        assert_eq!(degraded_notice(true, true), None);
        let (title, body) = degraded_notice(false, true).expect("audio missing");
        assert_eq!(title, "Started without audio");
        assert!(body.contains("volume"), "{body}");
        assert!(!body.contains("media"), "{body}");
        let (title, body) = degraded_notice(true, false).expect("media missing");
        assert_eq!(title, "Started without media");
        assert!(body.contains("MPRIS"), "{body}");
        let (title, body) = degraded_notice(false, false).expect("both missing");
        assert_eq!(title, "Started without audio and media");
        assert!(body.contains("volume") && body.contains("MPRIS"), "{body}");
    }
}
