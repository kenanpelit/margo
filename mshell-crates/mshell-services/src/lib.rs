use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::runtime::Runtime;
use tracing::info;

static TOKIO_RT: OnceLock<Runtime> = OnceLock::new();

/// The same tokio runtime that `init_services` ran on. Watchers that
/// observe `wayle_*::Property` channels must spawn on this runtime —
/// wayle's monitoring tasks live here, and `tokio::sync::watch`
/// wakeups don't propagate reliably across runtimes (we see only the
/// initial value if we poll from relm4's private runtime; subsequent
/// `replace`/`set` updates are missed).
pub fn tokio_rt() -> &'static Runtime {
    TOKIO_RT.get_or_init(|| Runtime::new().expect("tokio runtime"))
}

/// Convenience wrapper around `tokio_rt().spawn(...)`.
pub fn tokio_rt_spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio_rt().spawn(future)
}
use wayle_audio::AudioService;
use wayle_battery::BatteryService;
use wayle_bluetooth::BluetoothService;
use wayle_brightness::BrightnessService;
use mshell_margo_client::MargoService;
use wayle_media::MediaService;
use wayle_network::NetworkService;
use wayle_notification::NotificationService;
use wayle_power_profiles::PowerProfilesService;
use wayle_sysinfo::SysinfoService;
use wayle_systray::SystemTrayService;
use wayle_weather::{LocationQuery, TemperatureUnit, WeatherService, WeatherServiceBuilder};
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Error};

pub async fn init_services(
    location_query: LocationQuery,
    temperature_unit: TemperatureUnit,
) -> anyhow::Result<()> {
    info!("Initializing services...");
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
        async { Ok::<_, anyhow::Error>(AudioService::new().await?) },
        async { Ok::<_, anyhow::Error>(BatteryService::new().await?) },
        async { Ok::<_, anyhow::Error>(BluetoothService::new().await?) },
        async { Ok::<_, anyhow::Error>(BrightnessService::new().await?) },
        async { Ok::<_, anyhow::Error>(MargoService::new().await?) },
        line_power_fut,
        async { Ok::<_, anyhow::Error>(MediaService::new().await?) },
        async { Ok::<_, anyhow::Error>(NetworkService::new().await?) },
        async { Ok::<_, anyhow::Error>(NotificationService::new().await?) },
        async { Ok::<_, anyhow::Error>(PowerProfilesService::new().await?) },
        async { Ok::<_, anyhow::Error>(SystemTrayService::builder().build().await?) },
    )?;
    let sysinfo = SysinfoService::builder().build();
    let weather = WeatherServiceBuilder::new()
        .poll_interval(Duration::from_mins(15))
        .location(location_query)
        .units(temperature_unit)
        .build();

    AUDIO_SERVICE.set(audio).ok();
    BATTERY_SERVICE.set(Arc::new(battery)).ok();
    BLUETOOTH_SERVICE.set(Arc::new(bluetooth)).ok();
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

static BLUETOOTH_SERVICE: OnceLock<Arc<BluetoothService>> = OnceLock::new();

pub fn bluetooth_service() -> Arc<BluetoothService> {
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

static NOTIFICATION_SERVICE: OnceLock<Arc<NotificationService>> = OnceLock::new();

pub fn notification_service() -> Arc<NotificationService> {
    NOTIFICATION_SERVICE
        .get()
        .expect("NotificationService not initialized")
        .clone()
}

static POWER_PROFILE_SERVICE: OnceLock<Arc<PowerProfilesService>> = OnceLock::new();

pub fn power_profile_service() -> Arc<PowerProfilesService> {
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

static SYS_TRAY_SERVICE: OnceLock<Arc<SystemTrayService>> = OnceLock::new();

pub fn sys_tray_service() -> Arc<SystemTrayService> {
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
