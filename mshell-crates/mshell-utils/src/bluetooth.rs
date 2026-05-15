use mshell_common::{watch, watch_cancellable};
use mshell_services::bluetooth_service;
use relm4::gtk::gdk;
use relm4::{Component, ComponentSender, gtk};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use wayle_bluetooth::core::device::Device;

pub fn set_bluetooth_icon(image: &gtk::Image) {
    let bluetooth = bluetooth_service();
    let available = bluetooth.available.get();
    let enabled = bluetooth.enabled.get();

    if !available {
        image.set_icon_name(Some("bluetooth-hardware-disabled-symbolic"));
    } else if enabled {
        image.set_icon_name(Some("bluetooth-active-symbolic"));
    } else {
        image.set_icon_name(Some("bluetooth-disabled-symbolic"));
    }
}

pub fn set_bluetooth_label(label: &gtk::Label) {
    let bluetooth = bluetooth_service();
    let available = bluetooth.available.get();
    let enabled = bluetooth.enabled.get();

    if !available {
        label.set_label("Bluetooth Hardware Missing");
        return;
    }
    if !enabled {
        label.set_label("Bluetooth Disabled");
        return;
    }

    // Enabled — surface what's connected so the user can see at a
    // glance whether the row needs attention. "Bluetooth" alone
    // tells you the radio is on but says nothing about whether
    // anything is paired and live.
    let connected: Vec<String> = bluetooth
        .devices
        .get()
        .iter()
        .filter(|d| d.connected.get())
        .map(|d| d.alias.get().to_string())
        .collect();

    match connected.len() {
        0 => label.set_label("Bluetooth — no devices"),
        1 => label.set_label(&connected[0]),
        n => label.set_label(&format!("{} ({} connected)", connected[0], n)),
    }
}

pub fn get_bluetooth_device_icon(device: Arc<Device>) -> String {
    let icon_theme = gtk::IconTheme::for_display(&gdk::Display::default().unwrap());

    device
        .icon
        .get()
        .map(|i| format!("{}-symbolic", i))
        .filter(|i| icon_theme.has_icon(i))
        .unwrap_or_else(|| "bluetooth-active-symbolic".to_string())
}

pub fn spawn_bluetooth_devices_watcher<C>(
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let bluetooth = bluetooth_service();
    let devices = bluetooth.devices.clone();

    watch!(sender, [devices.watch()], |out| {
        let _ = out.send(map_state());
    });
}

pub fn spawn_bluetooth_enabled_watcher<C>(
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let bluetooth = bluetooth_service();
    let available = bluetooth.available.clone();
    let enabled = bluetooth.enabled.clone();

    watch!(sender, [available.watch(), enabled.watch()], |out| {
        let _ = out.send(map_state());
    });
}

pub fn spawn_bluetooth_device_watcher<C>(
    device: &Device,
    cancellation_token: CancellationToken,
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let paired = device.paired.clone();
    let connected = device.connected.clone();
    let trusted = device.trusted.clone();

    watch_cancellable!(
        sender,
        cancellation_token,
        [paired.watch(), connected.watch(), trusted.watch(),],
        |out| {
            let _ = out.send(map_state());
        }
    );
}

pub fn spawn_bluetooth_device_battery_watcher<C>(
    device: &Device,
    cancellation_token: CancellationToken,
    sender: &ComponentSender<C>,
    map_state: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let battery = device.battery_percentage.clone();

    watch_cancellable!(sender, cancellation_token, [battery.watch(),], |out| {
        let _ = out.send(map_state());
    });
}
