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
use mshell_config::schema::config::{ConfigStoreFields, Toasts};
use mshell_services::{
    audio_service, battery_service, line_power_service, margo_service, media_service,
};
use mshell_utils::audio::{spawn_default_input_watcher, spawn_default_output_watcher};
use mshell_utils::battery::{
    get_battery_icon, spawn_battery_online_watcher, spawn_battery_watcher,
};
use mshell_utils::media::spawn_media_players_watcher;
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
    /// Cancels the per-player metadata watchers when the player list changes.
    media_token: WatcherToken,
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

        let model = ToastProducerModel {
            line_power_first: false,
            prev_online: None,
            prev_layout: None,
            prev_out_dev: None,
            prev_in_dev: None,
            prev_caps: None,
            prev_num: None,
            prev_vpn: None,
            prev_track: None,
            media_token: WatcherToken::new(),
            bat_warned: None,
            bat_crit_warned: false,
        };

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
