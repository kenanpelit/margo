//! The single, headless toast producer.
//!
//! One instance is spawned for the whole shell (mirroring
//! [`crate::sound_alerts::SoundAlertsModel`]). It subscribes to every system
//! event source *once* — battery / AC power, default audio devices, keyboard
//! layout, lock keys, VPN, now-playing — tracks the previous value of each so
//! it only fires on a *change* (the reactive watchers emit the current value
//! on subscribe, which would otherwise toast a flurry at login), gates each
//! event on its config switch, and calls [`crate::toast::push_toast`]. The
//! per-output [`crate::toast::ToastSurfaceModel`] surfaces render whatever it
//! broadcasts.
//!
//! Centralising here is deliberate: the subprocess pollers (VPN `mvpn`,
//! lock-key sysfs reads) run once total, not once per monitor.

use crate::toast::{ToastEvent, ToastSeverity, push_toast};
use futures::StreamExt;
use gtk4::glib;
use gtk4::prelude::{GtkWindowExt, WidgetExt};
use gtk4_layer_shell::{Layer, LayerShell};
use mshell_common::{WatcherToken, watch_cancellable};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GameModeStoreFields, Toasts};
use mshell_services::{
    audio_service, battery_service, bluetooth_service, line_power_service, margo_service,
    media_service, notification_service, power_profile_service,
};
use mshell_utils::audio::{spawn_default_input_watcher, spawn_default_output_watcher};
use mshell_utils::battery::{
    get_battery_icon, spawn_battery_online_watcher, spawn_battery_watcher,
};
use mshell_utils::bluetooth::{
    spawn_bluetooth_device_watcher, spawn_bluetooth_devices_watcher,
    spawn_bluetooth_enabled_watcher,
};
use mshell_utils::idle::{idle_inhibited, spawn_idle_inhibitor_watcher};
use mshell_utils::media::spawn_media_players_watcher;
use mshell_utils::network::{
    active_network_label, spawn_network_watcher, spawn_wifi_watcher, spawn_wired_watcher,
};
use mshell_utils::notifications::spawn_dnd_watcher;
use mshell_utils::power_profile::{
    get_power_profile_icon, get_power_profile_label, spawn_active_profile_watcher,
};
use reactive_graph::traits::GetUntracked;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::time::Duration;
use wayle_battery::types::DeviceState;

pub struct ToastProducerModel {
    /// Skip the first line-power sample (the watcher emits the current state
    /// on subscribe — we baseline it rather than toasting at login).
    line_power_first: bool,
    prev_online: Option<bool>,
    prev_layout: Option<String>,
    prev_out_dev: Option<String>,
    prev_in_dev: Option<String>,
    prev_caps: Option<bool>,
    prev_num: Option<bool>,
    prev_vpn: Option<bool>,
    prev_track: Option<String>,
    /// Active primary network link label (`None` inner = disconnected). Outer
    /// `None` = not yet baselined.
    prev_net: Option<Option<String>>,
    /// Sorted names of the currently-connected Bluetooth devices. Outer `None`
    /// = not yet baselined.
    prev_bt: Option<Vec<String>>,
    prev_profile: Option<String>,
    prev_dnd: Option<bool>,
    prev_idle: Option<bool>,
    prev_game_mode: Option<bool>,
    /// Cancels the per-player metadata watchers when the player list changes.
    media_token: WatcherToken,
    /// Cancels the per-Bluetooth-device connectivity watchers when the device
    /// list changes (re-armed on every list change, like the bluetooth pill).
    bt_token: WatcherToken,
    /// Cancel + re-arm the Wi-Fi / wired sub-watchers on hot-plug (the
    /// top-level network watcher only re-fires when a device appears/vanishes).
    net_wifi_token: WatcherToken,
    net_wired_token: WatcherToken,
    /// Lowest battery warn level already toasted this discharge cycle.
    bat_warned: Option<u8>,
    bat_crit_warned: bool,
}

#[derive(Debug)]
pub enum ToastProducerInput {
    LockKeysTick,
}

#[derive(Debug)]
pub enum ToastProducerOutput {}

#[derive(Debug)]
pub enum ToastProducerCmd {
    Charging,
    Battery,
    Layout(String),
    OutputDevice,
    InputDevice,
    Vpn(bool),
    MediaListChanged,
    Track,
    /// Primary network link changed.
    Network,
    /// Wi-Fi device (un)plugged — re-arm its sub-watcher, then re-check.
    NetworkWifi,
    /// Wired device (un)plugged — re-arm its sub-watcher, then re-check.
    NetworkWired,
    /// Bluetooth adapter or device list changed — re-arm per-device watchers.
    Bluetooth,
    /// A per-device `connected` flag flipped — re-check the connected set.
    BluetoothConn,
    PowerProfile,
    Dnd,
    Idle,
}

/// Poll cadence for the `mvpn` subprocess (no push API; mirrors the VPN pill).
const VPN_POLL: Duration = Duration::from_secs(5);
const VPN_STARTUP_DELAY: Duration = Duration::from_millis(800);
/// Lock keys never toggle fast; a slow tick keeps CPU at zero.
const LOCK_KEYS_TICK: Duration = Duration::from_millis(700);

fn toasts_cfg() -> Toasts {
    config_manager().config().toasts().get_untracked()
}

#[relm4::component(pub)]
impl Component for ToastProducerModel {
    type CommandOutput = ToastProducerCmd;
    type Input = ToastProducerInput;
    type Output = ToastProducerOutput;
    type Init = ();

    view! {
        #[root]
        gtk::Window {
            add_css_class: "toast-producer",
            set_decorated: false,
            set_visible: false,
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Headless: a never-shown background surface, like SoundAlerts.
        root.init_layer_shell();
        root.set_layer(Layer::Background);
        root.set_exclusive_zone(-1);

        // Reactive event sources (each emits the current value on subscribe).
        spawn_battery_online_watcher(&sender, || ToastProducerCmd::Charging);
        spawn_battery_watcher(&sender, || ToastProducerCmd::Battery);
        spawn_default_output_watcher(&sender, None, || ToastProducerCmd::OutputDevice);
        spawn_default_input_watcher(&sender, None, || ToastProducerCmd::InputDevice);
        spawn_media_players_watcher(
            &sender,
            || ToastProducerCmd::MediaListChanged,
            || ToastProducerCmd::MediaListChanged,
        );
        spawn_network_watcher(
            &sender,
            || ToastProducerCmd::Network,
            || ToastProducerCmd::NetworkWifi,
            || ToastProducerCmd::NetworkWired,
        );
        spawn_bluetooth_enabled_watcher(&sender, || ToastProducerCmd::Bluetooth);
        spawn_bluetooth_devices_watcher(&sender, || ToastProducerCmd::Bluetooth);
        spawn_active_profile_watcher(&sender, None, || ToastProducerCmd::PowerProfile);
        spawn_dnd_watcher(&sender, || ToastProducerCmd::Dnd);
        spawn_idle_inhibitor_watcher(&sender, || ToastProducerCmd::Idle);

        // Keyboard layout — reactive String mirrored from the compositor.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut stream = margo_service().keyboard_layout.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(name) => { let _ = out.send(ToastProducerCmd::Layout(name)); }
                        None => break,
                    },
                }
            }
        });

        // VPN — poll `mvpn`, but only while VPN toasts are enabled (so the
        // subprocess never runs when the user has the toggle off).
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut first = true;
            loop {
                let delay = if first { VPN_STARTUP_DELAY } else { VPN_POLL };
                first = false;
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                let cfg = toasts_cfg();
                if !cfg.enabled || !cfg.vpn {
                    continue;
                }
                let _ = out.send(ToastProducerCmd::Vpn(vpn_connected().await));
            }
        });

        // Lock keys — sysfs LED poll on the glib main loop.
        {
            let s = sender.clone();
            glib::timeout_add_local(LOCK_KEYS_TICK, move || {
                if s.input_sender()
                    .send(ToastProducerInput::LockKeysTick)
                    .is_err()
                {
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        }

        let mut model = ToastProducerModel {
            line_power_first: false,
            prev_online: None,
            prev_layout: None,
            prev_out_dev: None,
            prev_in_dev: None,
            prev_caps: None,
            prev_num: None,
            prev_vpn: None,
            prev_track: None,
            prev_net: None,
            prev_bt: None,
            prev_profile: None,
            prev_dnd: None,
            prev_idle: None,
            prev_game_mode: None,
            media_token: WatcherToken::new(),
            bt_token: WatcherToken::new(),
            net_wifi_token: WatcherToken::new(),
            net_wired_token: WatcherToken::new(),
            bat_warned: None,
            bat_crit_warned: false,
        };

        // Arm the per-device Bluetooth connectivity watchers + the Wi-Fi /
        // wired sub-watchers for whatever's already present — the list /
        // top-level watchers only re-fire when a device appears or vanishes.
        model.arm_bt_watchers(&sender);
        let wifi_token = model.net_wifi_token.reset();
        spawn_wifi_watcher(&sender, wifi_token, || ToastProducerCmd::Network);
        let wired_token = model.net_wired_token.reset();
        spawn_wired_watcher(&sender, wired_token, || ToastProducerCmd::Network);

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ToastProducerInput::LockKeysTick => {
                let (caps, num, _) = read_lock_state();
                let cfg = toasts_cfg();
                lock_key_step(self.prev_caps, caps, "Caps Lock", &cfg);
                lock_key_step(self.prev_num, num, "Num Lock", &cfg);
                self.prev_caps = Some(caps);
                self.prev_num = Some(num);
                // Game Mode is config-driven (no service watcher), so poll it on
                // the same slow tick — it toggles rarely and the cost is one bool
                // read.
                self.on_game_mode();
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ToastProducerCmd::Charging => self.on_charging(),
            ToastProducerCmd::Battery => self.on_battery(),
            ToastProducerCmd::Layout(name) => self.on_layout(name),
            ToastProducerCmd::OutputDevice => self.on_audio_device(true),
            ToastProducerCmd::InputDevice => self.on_audio_device(false),
            ToastProducerCmd::Vpn(connected) => self.on_vpn(connected),
            ToastProducerCmd::MediaListChanged => self.on_media_list_changed(&sender),
            ToastProducerCmd::Track => self.on_track(),
            ToastProducerCmd::Network => self.on_network(),
            ToastProducerCmd::NetworkWifi => {
                let token = self.net_wifi_token.reset();
                spawn_wifi_watcher(&sender, token, || ToastProducerCmd::Network);
                self.on_network();
            }
            ToastProducerCmd::NetworkWired => {
                let token = self.net_wired_token.reset();
                spawn_wired_watcher(&sender, token, || ToastProducerCmd::Network);
                self.on_network();
            }
            ToastProducerCmd::Bluetooth => {
                self.arm_bt_watchers(&sender);
                self.on_bluetooth();
            }
            ToastProducerCmd::BluetoothConn => self.on_bluetooth(),
            ToastProducerCmd::PowerProfile => self.on_power_profile(),
            ToastProducerCmd::Dnd => self.on_dnd(),
            ToastProducerCmd::Idle => self.on_idle(),
        }
    }
}

impl ToastProducerModel {
    fn on_charging(&mut self) {
        let Some(service) = line_power_service() else {
            return;
        };
        let online = service.device.online.get();
        if !self.line_power_first {
            self.line_power_first = true;
            self.prev_online = Some(online);
            return;
        }
        if self.prev_online == Some(online) {
            return;
        }
        self.prev_online = Some(online);

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.charging {
            return;
        }
        if online {
            push_toast(ToastEvent {
                icon: "battery-level-100-charging-symbolic".to_string(),
                title: "Charging".to_string(),
                body: Some("AC power connected".to_string()),
                severity: ToastSeverity::Positive,
            });
        } else {
            let pct = battery_service().device.percentage.get();
            push_toast(ToastEvent {
                icon: get_battery_icon(pct).to_string(),
                title: "On battery".to_string(),
                body: Some(format!("{}% remaining", pct.round() as i32)),
                severity: ToastSeverity::Calm,
            });
        }
    }

    fn on_battery(&mut self) {
        let device = battery_service().device.clone();
        if !device.is_present.get() {
            return;
        }
        let pct = device.percentage.get();
        let state = device.state.get();
        let charging = matches!(state, DeviceState::Charging | DeviceState::FullyCharged)
            || line_power_service()
                .map(|s| s.device.online.get())
                .unwrap_or(false);

        // Recharging re-arms the ladder for the next discharge cycle.
        if charging {
            self.bat_warned = None;
            self.bat_crit_warned = false;
            return;
        }

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.battery {
            return;
        }
        let pctu = pct.round().clamp(0.0, 100.0) as u8;

        let crit = cfg.battery_critical_level;
        if crit > 0 && pctu <= crit {
            if !self.bat_crit_warned {
                self.bat_crit_warned = true;
                push_toast(ToastEvent {
                    icon: get_battery_icon(pct).to_string(),
                    title: "Battery critical".to_string(),
                    body: Some(format!("{pctu}% — plug in now")),
                    severity: ToastSeverity::Danger,
                });
            }
            return;
        }

        // The most urgent warn level reached (lowest threshold ≥ which the
        // charge has fallen). Firing only when it changes gives exactly one
        // toast per level as the battery drains.
        let reached = cfg
            .battery_warn_levels
            .iter()
            .copied()
            .filter(|&l| l > crit && pctu <= l)
            .min();
        if let Some(level) = reached
            && self.bat_warned != Some(level)
        {
            self.bat_warned = Some(level);
            push_toast(ToastEvent {
                icon: get_battery_icon(pct).to_string(),
                title: "Battery low".to_string(),
                body: Some(format!("{pctu}% remaining")),
                severity: ToastSeverity::Warn,
            });
        }
    }

    fn on_layout(&mut self, layout: String) {
        if layout.is_empty() {
            return;
        }
        match &self.prev_layout {
            None => {
                self.prev_layout = Some(layout);
                return;
            }
            Some(p) if *p == layout => return,
            Some(_) => {}
        }
        self.prev_layout = Some(layout.clone());

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.kb_layout {
            return;
        }
        push_toast(ToastEvent {
            icon: "input-keyboard-symbolic".to_string(),
            title: "Keyboard layout".to_string(),
            body: Some(layout),
            severity: ToastSeverity::Calm,
        });
    }

    fn on_audio_device(&mut self, output: bool) {
        let name = if output {
            audio_service()
                .default_output
                .get()
                .map(|d| d.description.get())
                .unwrap_or_default()
        } else {
            audio_service()
                .default_input
                .get()
                .map(|d| d.description.get())
                .unwrap_or_default()
        };
        if name.is_empty() {
            return;
        }
        let prev = if output {
            &mut self.prev_out_dev
        } else {
            &mut self.prev_in_dev
        };
        match prev {
            None => {
                *prev = Some(name);
                return;
            }
            Some(p) if *p == name => return,
            Some(_) => {}
        }
        *prev = Some(name.clone());

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.audio_device {
            return;
        }
        push_toast(ToastEvent {
            icon: if output {
                "audio-volume-high-symbolic"
            } else {
                "microphone-sensitivity-high-symbolic"
            }
            .to_string(),
            title: if output {
                "Audio output"
            } else {
                "Audio input"
            }
            .to_string(),
            body: Some(name),
            severity: ToastSeverity::Calm,
        });
    }

    fn on_vpn(&mut self, connected: bool) {
        match self.prev_vpn {
            None => {
                self.prev_vpn = Some(connected);
                return;
            }
            Some(p) if p == connected => return,
            Some(_) => {}
        }
        self.prev_vpn = Some(connected);

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.vpn {
            return;
        }
        push_toast(ToastEvent {
            icon: "network-vpn-symbolic".to_string(),
            title: if connected {
                "VPN connected"
            } else {
                "VPN disconnected"
            }
            .to_string(),
            body: None,
            severity: if connected {
                ToastSeverity::Positive
            } else {
                ToastSeverity::Calm
            },
        });
    }

    fn on_media_list_changed(&mut self, sender: &ComponentSender<Self>) {
        // Re-subscribe to every player's title/artist under a fresh token, so
        // a track change on any player fires `Track`.
        let token = self.media_token.reset();
        for player in media_service().player_list.get() {
            let title = player.metadata.title.clone();
            let artist = player.metadata.artist.clone();
            let t = token.clone();
            watch_cancellable!(sender, t, [title.watch(), artist.watch()], |out| {
                let _ = out.send(ToastProducerCmd::Track);
            });
        }
    }

    fn on_track(&mut self) {
        let track = current_track();
        if track.is_empty() {
            // Playback stopped — baseline so the next track toasts.
            self.prev_track = Some(String::new());
            return;
        }
        match &self.prev_track {
            None => {
                self.prev_track = Some(track);
                return;
            }
            Some(p) if *p == track => return,
            Some(_) => {}
        }
        self.prev_track = Some(track.clone());

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.now_playing {
            return;
        }
        push_toast(ToastEvent {
            icon: "media-playback-start-symbolic".to_string(),
            title: "Now playing".to_string(),
            body: Some(track),
            severity: ToastSeverity::Calm,
        });
    }

    /// Re-attach the per-Bluetooth-device connectivity watchers under a fresh
    /// token (cancelling the previous batch). Called on init and whenever the
    /// device list changes, mirroring the bluetooth bar pill — without it a
    /// `connected` flip on an already-listed device would never fire.
    fn arm_bt_watchers(&mut self, sender: &ComponentSender<Self>) {
        let Some(bt) = bluetooth_service() else {
            return;
        };
        let token = self.bt_token.reset();
        for device in bt.devices.get() {
            spawn_bluetooth_device_watcher(&device, token.clone(), sender, || {
                ToastProducerCmd::BluetoothConn
            });
        }
    }

    fn on_network(&mut self) {
        let label = active_network_label();
        match &self.prev_net {
            None => {
                self.prev_net = Some(label);
                return;
            }
            Some(p) if *p == label => return,
            Some(_) => {}
        }
        self.prev_net = Some(label.clone());

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.network {
            return;
        }
        match label {
            Some(name) => push_toast(ToastEvent {
                icon: "network-wireless-symbolic".to_string(),
                title: "Network".to_string(),
                body: Some(name),
                severity: ToastSeverity::Positive,
            }),
            None => push_toast(ToastEvent {
                icon: "network-wireless-offline-symbolic".to_string(),
                title: "Network disconnected".to_string(),
                body: None,
                severity: ToastSeverity::Calm,
            }),
        }
    }

    fn on_bluetooth(&mut self) {
        let Some(svc) = bluetooth_service() else {
            return;
        };
        // Only the adapter being present + on counts — a stale `connected`
        // flag on a disabled adapter shouldn't surface as a device.
        let mut connected: Vec<String> = if svc.available.get() && svc.enabled.get() {
            svc.devices
                .get()
                .iter()
                .filter(|d| d.connected.get())
                .map(|d| d.alias.get().to_string())
                .collect()
        } else {
            Vec::new()
        };
        connected.sort();

        let prev = match self.prev_bt.take() {
            None => {
                self.prev_bt = Some(connected);
                return;
            }
            Some(p) => p,
        };
        if prev == connected {
            self.prev_bt = Some(prev);
            return;
        }
        let newly: Vec<String> = connected
            .iter()
            .filter(|n| !prev.contains(n))
            .cloned()
            .collect();
        let gone: Vec<String> = prev
            .iter()
            .filter(|n| !connected.contains(n))
            .cloned()
            .collect();
        self.prev_bt = Some(connected);

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.bluetooth {
            return;
        }
        for name in newly {
            push_toast(ToastEvent {
                icon: "bluetooth-active-symbolic".to_string(),
                title: "Bluetooth connected".to_string(),
                body: Some(name),
                severity: ToastSeverity::Positive,
            });
        }
        for name in gone {
            push_toast(ToastEvent {
                icon: "bluetooth-disabled-symbolic".to_string(),
                title: "Bluetooth disconnected".to_string(),
                body: Some(name),
                severity: ToastSeverity::Calm,
            });
        }
    }

    fn on_power_profile(&mut self) {
        let Some(svc) = power_profile_service() else {
            return;
        };
        let profile = svc.power_profiles.active_profile.get();
        let label = get_power_profile_label(&profile).to_string();
        match &self.prev_profile {
            None => {
                self.prev_profile = Some(label);
                return;
            }
            Some(p) if *p == label => return,
            Some(_) => {}
        }
        self.prev_profile = Some(label.clone());

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.power_profile {
            return;
        }
        push_toast(ToastEvent {
            icon: get_power_profile_icon(&profile).to_string(),
            title: "Power profile".to_string(),
            body: Some(label),
            severity: ToastSeverity::Calm,
        });
    }

    fn on_dnd(&mut self) {
        let Some(svc) = notification_service() else {
            return;
        };
        let on = svc.dnd.get();
        match self.prev_dnd {
            None => {
                self.prev_dnd = Some(on);
                return;
            }
            Some(p) if p == on => return,
            Some(_) => {}
        }
        self.prev_dnd = Some(on);

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.dnd {
            return;
        }
        push_toast(ToastEvent {
            icon: if on {
                "notifications-disabled-symbolic"
            } else {
                "notification-symbolic"
            }
            .to_string(),
            title: format!("Do Not Disturb {}", if on { "on" } else { "off" }),
            body: None,
            severity: ToastSeverity::Calm,
        });
    }

    fn on_idle(&mut self) {
        let on = idle_inhibited();
        match self.prev_idle {
            None => {
                self.prev_idle = Some(on);
                return;
            }
            Some(p) if p == on => return,
            Some(_) => {}
        }
        self.prev_idle = Some(on);

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.idle_inhibitor {
            return;
        }
        push_toast(ToastEvent {
            icon: "preferences-desktop-screensaver-symbolic".to_string(),
            title: format!("Keep awake {}", if on { "on" } else { "off" }),
            body: None,
            severity: ToastSeverity::Calm,
        });
    }

    fn on_game_mode(&mut self) {
        let active = config_manager()
            .config()
            .game_mode()
            .active()
            .get_untracked();
        match self.prev_game_mode {
            None => {
                self.prev_game_mode = Some(active);
                return;
            }
            Some(p) if p == active => return,
            Some(_) => {}
        }
        self.prev_game_mode = Some(active);

        let cfg = toasts_cfg();
        if !cfg.enabled || !cfg.game_mode {
            return;
        }
        push_toast(ToastEvent {
            icon: "applications-games-symbolic".to_string(),
            title: format!("Game Mode {}", if active { "on" } else { "off" }),
            body: None,
            severity: ToastSeverity::Calm,
        });
    }
}

/// Toast a Caps/Num lock transition. Fires only when `prev` is a known
/// previous value that differs from `cur` (the first observation just
/// baselines), and the toggle is on. The caller stores `cur` as the new prev.
fn lock_key_step(prev: Option<bool>, cur: bool, label: &str, cfg: &Toasts) {
    if matches!(prev, Some(p) if p != cur) && cfg.enabled && cfg.lock_keys {
        push_toast(ToastEvent {
            icon: "input-keyboard-symbolic".to_string(),
            title: format!("{label} {}", if cur { "on" } else { "off" }),
            body: None,
            severity: ToastSeverity::Calm,
        });
    }
}

/// The currently displayed track as `title — artist` (mirrors the media pill's
/// `display_player`: prefer the active player, else the first). Empty when
/// nothing is playing.
fn current_track() -> String {
    let svc = media_service();
    let player = svc
        .active_player
        .get()
        .or_else(|| svc.player_list.get().into_iter().next());
    match player {
        Some(p) => {
            let title = p.metadata.title.get();
            let artist = p.metadata.artist.get();
            let title = title.trim();
            let artist = artist.trim();
            if title.is_empty() {
                String::new()
            } else if artist.is_empty() {
                title.to_string()
            } else {
                format!("{title} — {artist}")
            }
        }
        None => String::new(),
    }
}

/// `mvpn status --json` → connected. Missing `mvpn` reads as disconnected.
async fn vpn_connected() -> bool {
    match tokio::process::Command::new("mvpn")
        .args(["status", "--json"])
        .output()
        .await
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout).contains("\"connected\":true"),
        Err(_) => false,
    }
}

/// Read Caps / Num lock from `/sys/class/leds` (kernel LED state, focus- and
/// display-server-independent). Mirrors the lock-keys bar pill.
fn read_lock_state() -> (bool, bool, bool) {
    let dir = PathBuf::from("/sys/class/leds");
    let (mut caps, mut num, mut scroll) = (None, None, None);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (false, false, false);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(n) = name.to_str() else { continue };
        let target = if n.ends_with("::capslock") {
            &mut caps
        } else if n.ends_with("::numlock") {
            &mut num
        } else if n.ends_with("::scrolllock") {
            &mut scroll
        } else {
            continue;
        };
        if target.is_some() {
            continue;
        }
        if let Ok(s) = std::fs::read_to_string(entry.path().join("brightness")) {
            *target = Some(s.trim() != "0");
        }
    }
    (
        caps.unwrap_or(false),
        num.unwrap_or(false),
        scroll.unwrap_or(false),
    )
}
