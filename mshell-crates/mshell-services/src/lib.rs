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
pub mod audio_watchdog;
pub mod bluetooth;
pub mod login_net;

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

/// Time one service's construction.
///
/// `init_services` is awaited from the GTK main thread *before* the first bar
/// is painted, so the blank-screen window at login is exactly the slowest arm
/// of the `try_join!` below — the arms run concurrently, so the total is the
/// max, not the sum. Without per-arm timing there is no way to tell which
/// D-Bus peer is responsible, and the obvious suspects (BlueZ registering a
/// pairing agent, UPower's `EnumerateDevices`, the StatusNotifier host) are
/// all equally plausible. Attribute first, restructure second.
async fn timed<T>(name: &'static str, fut: impl std::future::Future<Output = T>) -> T {
    let started = std::time::Instant::now();
    let out = fut.await;
    info!(
        service = name,
        ms = started.elapsed().as_millis() as u64,
        "service ready"
    );
    out
}

pub async fn init_services(
    location_query: LocationQuery,
    temperature_unit: TemperatureUnit,
    weather_poll_minutes: u32,
) -> anyhow::Result<()> {
    info!("Initializing services...");
    let init_started = std::time::Instant::now();
    let line_power_fut = async {
        if let Some(path) = find_line_power_path().await? {
            Ok::<_, anyhow::Error>(Some(
                BatteryService::builder().device_path(path).build().await?,
            ))
        } else {
            Ok(None)
        }
    };

    let (
        audio,
        battery,
        bluetooth,
        brightness,
        hyprland,
        line_power,
        media,
        network,
        notifications,
        power_profiles,
        systray,
    ) = tokio::try_join!(
        timed("audio", async {
            Ok::<_, anyhow::Error>(AudioService::new().await?)
        }),
        timed("battery", async {
            Ok::<_, anyhow::Error>(BatteryService::new().await?)
        }),
        // Bluetooth is best-effort. `BluetoothService::new()` registers a BlueZ
        // pairing agent, which requires a running `org.bluez`. A host with no
        // adapter (e.g. a VM) has bluez installed but `bluetooth.service`
        // condition-skipped (`ConditionPathIsDirectory=/sys/class/bluetooth`),
        // so activating org.bluez fails — without this guard that error aborts
        // the whole `try_join!` and the shell never starts. Degrade to "no
        // Bluetooth" instead; `bluetooth_service()` is then `None`.
        timed("bluetooth", async {
            Ok::<_, anyhow::Error>(match BluetoothService::new().await {
                Ok(bt) => Some(bt),
                Err(e) => {
                    warn!("Bluetooth unavailable; continuing without it: {e:#}");
                    None
                }
            })
        }),
        // Brightness is best-effort. `BrightnessService::new()` returns
        // `Ok(None)` when there's simply no backlight device, but a desktop
        // with no `/sys/class/backlight` at all — or a transient sysfs/D-Bus
        // error — surfaces as `Err`, which without this guard would abort the
        // whole `try_join!` and the shell never starts. Degrade to "no
        // brightness control"; `brightness_service()` is then `None` (its
        // callers already handle that).
        timed("brightness", async {
            Ok::<_, anyhow::Error>(match BrightnessService::new().await {
                Ok(b) => b,
                Err(e) => {
                    warn!("Brightness backlight unavailable; continuing without it: {e:#}");
                    None
                }
            })
        }),
        timed("margo", async { MargoService::new().await }),
        timed("line_power", line_power_fut),
        timed("media", async {
            // `with_art_cache()` resolves MPRIS `mpris:artUrl`s (incl.
            // remote http(s) covers from Spotify / browsers) to local
            // files on `TrackMetadata::cover_art`, which the media
            // widgets render as album art.
            Ok::<_, anyhow::Error>(MediaService::builder().with_art_cache().build().await?)
        }),
        timed("network", async {
            Ok::<_, anyhow::Error>(NetworkService::new().await?)
        }),
        // Notifications are best-effort. `NotificationService::new()` claims the
        // `org.freedesktop.Notifications` well-known name; if another daemon
        // (dunst / mako) already owns it, construction fails — that must not
        // abort login. Degrade to `None`; toasts just don't show.
        timed("notifications", async {
            Ok::<_, anyhow::Error>(match NotificationService::new().await {
                Ok(n) => Some(n),
                Err(e) => {
                    warn!("Notification service unavailable; continuing without it: {e:#}");
                    None
                }
            })
        }),
        // Power profiles are best-effort. `PowerProfilesService::new()` talks to
        // `power-profiles-daemon` over D-Bus; on a host without ppd installed it
        // fails, which must not block the shell. Degrade to `None`; the profile
        // pill / control-center tile hide.
        timed("power_profiles", async {
            Ok::<_, anyhow::Error>(match PowerProfilesService::new().await {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!("Power-profiles service unavailable; continuing without it: {e:#}");
                    None
                }
            })
        }),
        // System tray is best-effort. Building the StatusNotifier host can
        // fail (e.g. the well-known name is already owned by another tray).
        // The tray pill is non-essential, so degrade to "no tray" rather than
        // blocking login; `sys_tray_service()` is then `None`.
        timed("systray", async {
            Ok::<_, anyhow::Error>(match SystemTrayService::builder().build().await {
                Ok(t) => Some(t),
                Err(e) => {
                    warn!("System tray host unavailable; continuing without it: {e:#}");
                    None
                }
            })
        }),
    )?;
    // The bar cannot paint until this line is reached — the caller awaits us on
    // the GTK main thread. Compare this against the slowest `service ready`
    // line above to see whether the barrier is one slow peer or all of them.
    info!(
        ms = init_started.elapsed().as_millis() as u64,
        "all services ready"
    );
    let sysinfo = SysinfoService::builder().build();
    let weather = WeatherServiceBuilder::new()
        .poll_interval(Duration::from_mins(weather_poll_minutes.max(1) as u64))
        .location(location_query)
        .units(temperature_unit)
        .build();

    AUDIO_SERVICE.set(audio).ok();
    BATTERY_SERVICE.set(Arc::new(battery)).ok();
    BLUETOOTH_SERVICE.set(bluetooth.map(Arc::new)).ok();
    BRIGHTNESS_SERVICE.set(brightness).ok();
    MARGO_SERVICE.set(hyprland).ok();
    if let Some(line_power) = line_power {
        LINE_POWER_SERVICE.set(Some(Arc::new(line_power))).ok();
    } else {
        LINE_POWER_SERVICE.set(None).ok();
    }
    MEDIA_SERVICE.set(media).ok();
    NETWORK_SERVICE.set(Arc::new(network)).ok();
    NOTIFICATION_SERVICE.set(notifications).ok();
    POWER_PROFILE_SERVICE.set(power_profiles).ok();
    SYS_INFO_SERVICE.set(Arc::new(sysinfo)).ok();
    SYS_TRAY_SERVICE.set(systray).ok();
    WEATHER_SERVICE.set(Arc::new(weather)).ok();

    info!("Done");

    Ok(())
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

static AUDIO_SERVICE: OnceLock<Arc<AudioService>> = OnceLock::new();

pub fn audio_service() -> Arc<AudioService> {
    AUDIO_SERVICE
        .get()
        .expect("AudioService not initialized")
        .clone()
}

static BATTERY_SERVICE: OnceLock<Arc<BatteryService>> = OnceLock::new();

pub fn battery_service() -> Arc<BatteryService> {
    BATTERY_SERVICE
        .get()
        .expect("BatteryService not initialized")
        .clone()
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

static MEDIA_SERVICE: OnceLock<Arc<MediaService>> = OnceLock::new();

pub fn media_service() -> Arc<MediaService> {
    MEDIA_SERVICE
        .get()
        .expect("MediaService not initialized")
        .clone()
}

static NETWORK_SERVICE: OnceLock<Arc<NetworkService>> = OnceLock::new();

pub fn network_service() -> Arc<NetworkService> {
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
