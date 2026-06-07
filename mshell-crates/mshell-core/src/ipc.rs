use crate::relm_app::{Shell, ShellInput};
use mshell_cache::wallpaper::set_wallpaper;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    AlarmConfigStoreFields, AudioConfigStoreFields, ConfigStoreFields, WallpaperStoreFields,
};
use mshell_services::{
    audio_service, brightness_service, margo_service, media_service, notification_service, tokio_rt,
};
use mshell_session::session_lock::{lock_session, session_locked};
use mshell_settings::{close_settings, open_settings, open_wizard};
use mshell_sounds::{play_alarm_loop, play_audio_volume_change, stop_alarm};
use mshell_utils::session::SessionAction;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib;
use relm4::{ComponentSender, gtk};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::volume::types::Volume;
use wayle_brightness::Percentage;
use wayle_media::core::player::Player;
use wayle_media::types::PlaybackState;
use zbus::connection;
use zbus::interface;

pub fn init_ipc_shell_service(sender: &ComponentSender<Shell>) {
    let (shell_tx, mut shell_rx) = mpsc::unbounded_channel();

    spawn_daily_wallpaper_task(shell_tx.clone());
    spawn_alarm_scheduler();
    spawn_plugin_watcher_task(shell_tx.clone());
    tokio::spawn(start_shell_service(shell_tx));

    let app_sender = sender.input_sender().clone();
    glib::spawn_future_local(async move {
        // Resolve the active monitor name via the focused-client-
        // first heuristic in `MargoService::active_monitor_name`.
        // We used to chain through `active_workspace().monitor` —
        // that path bounces through the workspaces cache and
        // can return None right after reboot before sync has
        // populated, sending menus to whichever Frame iterated
        // first (effectively eDP-1). `active_monitor_name`
        // reads state.json directly + prefers the focused
        // client's monitor over the top-level `active_output`
        // field, which closes the boot-time window where margo's
        // `active_output` stays pinned to the first-enumerated
        // output until the user manually switches.
        async fn active_monitor() -> Option<String> {
            margo_service().active_monitor_name().await
        }
        while let Some(cmd) = shell_rx.recv().await {
            match cmd {
                IPCCommand::Quit => app_sender.emit(ShellInput::Quit),
                IPCCommand::AppLauncher => {
                    app_sender.emit(ShellInput::ToggleAppLauncher(active_monitor().await));
                }
                IPCCommand::AppLauncherTab(tab) => {
                    app_sender.emit(ShellInput::ToggleAppLauncherWithTab(
                        active_monitor().await,
                        tab,
                    ));
                }
                IPCCommand::Clock => {
                    app_sender.emit(ShellInput::ToggleClockMenu(active_monitor().await));
                }
                IPCCommand::Clipboard => {
                    app_sender.emit(ShellInput::ToggleClipboard(active_monitor().await));
                }
                IPCCommand::ClipboardAction(spec) => {
                    // Handled directly against the clipboard service
                    // singleton (no UI round-trip needed). The verb→op parse
                    // is a pure, unit-tested function.
                    let svc = mshell_clipboard::clipboard_service();
                    match parse_clipboard_action(&spec) {
                        Some(ClipboardAction::Copy(id)) => svc.copy_entry(id),
                        Some(ClipboardAction::TogglePin(id)) => svc.toggle_pin(id),
                        Some(ClipboardAction::Delete(id)) => {
                            svc.history().remove(id);
                        }
                        Some(ClipboardAction::Clear) => svc.clear_unpinned(),
                        Some(ClipboardAction::Wipe) => svc.clear_history(),
                        None => tracing::warn!(%spec, "clipboard: unknown action"),
                    }
                }
                IPCCommand::ClipboardList(reply) => {
                    let out = mshell_clipboard::clipboard_service()
                        .history()
                        .entries()
                        .iter()
                        .map(|e| {
                            let cat = format!("{:?}", e.category()).to_lowercase();
                            let preview = match &e.preview {
                                mshell_clipboard::EntryPreview::Text(t) => {
                                    t.replace(['\n', '\t'], " ")
                                }
                                mshell_clipboard::EntryPreview::Image { width, height, .. } => {
                                    format!("[image {width}x{height}]")
                                }
                                mshell_clipboard::EntryPreview::Binary { mime_type, size } => {
                                    format!("[{mime_type} {size}B]")
                                }
                            };
                            let pin = if e.pinned { "★ " } else { "" };
                            format!("{}\t{cat}\t{pin}{preview}", e.id)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let _ = reply.send(out);
                }
                IPCCommand::Notifications => {
                    app_sender.emit(ShellInput::ToggleNotifications(active_monitor().await));
                }
                IPCCommand::NotificationsClearAll => {
                    app_sender.emit(ShellInput::NotificationsClearAll);
                }
                IPCCommand::NotificationsReadPopups => {
                    app_sender.emit(ShellInput::NotificationsReadPopups);
                }
                IPCCommand::Screenshot => {
                    app_sender.emit(ShellInput::ToggleScreenshotMenu(active_monitor().await));
                }
                IPCCommand::ScreenshotCapture(spec) => {
                    app_sender.emit(ShellInput::CaptureScreenshot(spec));
                }
                IPCCommand::ScreenRecord(spec) => {
                    app_sender.emit(ShellInput::ScreenRecord(spec));
                }
                IPCCommand::Wallpaper => {
                    app_sender.emit(ShellInput::ToggleWallpaperMenu(active_monitor().await));
                }
                IPCCommand::WallpaperCycle(direction) => {
                    app_sender.emit(ShellInput::CycleWallpaper(direction));
                }
                IPCCommand::Ufw => {
                    app_sender.emit(ShellInput::ToggleUfwMenu(active_monitor().await));
                }
                IPCCommand::Privacy => {
                    app_sender.emit(ShellInput::TogglePrivacyMenu(active_monitor().await));
                }
                IPCCommand::Bluetooth => {
                    app_sender.emit(ShellInput::ToggleBluetoothMenu(active_monitor().await));
                }
                IPCCommand::BluetoothCtl(action) => {
                    // Spawn so a 10–12s connect wait never blocks the IPC loop.
                    tokio_rt().spawn(async move {
                        match action.as_str() {
                            "connect" => {
                                mshell_services::bluetooth::connect_configured().await;
                            }
                            "disconnect" => {
                                mshell_services::bluetooth::disconnect_configured().await;
                            }
                            _ => mshell_services::bluetooth::toggle().await,
                        }
                    });
                }
                IPCCommand::CpuDashboard => {
                    app_sender.emit(ShellInput::ToggleCpuDashboardMenu(active_monitor().await));
                }
                IPCCommand::AudioDashboard => {
                    app_sender.emit(ShellInput::ToggleAudioDashboardMenu(active_monitor().await));
                }
                IPCCommand::SystemUpdate => {
                    app_sender.emit(ShellInput::ToggleSystemUpdateMenu(active_monitor().await));
                }
                IPCCommand::Valent => {
                    app_sender.emit(ShellInput::ToggleValentMenu(active_monitor().await));
                }
                IPCCommand::KeepAwake => {
                    app_sender.emit(ShellInput::ToggleKeepAwakeMenu(active_monitor().await));
                }
                IPCCommand::Twilight => {
                    app_sender.emit(ShellInput::ToggleTwilightMenu(active_monitor().await));
                }
                IPCCommand::Weather => {
                    app_sender.emit(ShellInput::ToggleWeatherMenu(active_monitor().await));
                }
                IPCCommand::Keybinds => {
                    app_sender.emit(ShellInput::ToggleKeybindsMenu(active_monitor().await));
                }
                IPCCommand::AlarmClock => {
                    app_sender.emit(ShellInput::ToggleAlarmClockMenu(active_monitor().await));
                }
                IPCCommand::ControlCenter => {
                    app_sender.emit(ShellInput::ToggleControlCenterMenu(active_monitor().await));
                }
                IPCCommand::HiddenBar(verb) => {
                    app_sender.emit(ShellInput::HiddenBar(verb));
                }
                IPCCommand::SshSessions => {
                    app_sender.emit(ShellInput::ToggleSshSessionsMenu(active_monitor().await));
                }
                IPCCommand::Dns => {
                    app_sender.emit(ShellInput::ToggleDnsMenu(active_monitor().await));
                }
                IPCCommand::Podman => {
                    app_sender.emit(ShellInput::TogglePodmanMenu(active_monitor().await));
                }
                IPCCommand::Notes => {
                    app_sender.emit(ShellInput::ToggleNotesMenu(active_monitor().await));
                }
                IPCCommand::Ip => {
                    app_sender.emit(ShellInput::ToggleIpMenu(active_monitor().await));
                }
                IPCCommand::Network => {
                    app_sender.emit(ShellInput::ToggleNetworkMenu(active_monitor().await));
                }
                IPCCommand::Power => {
                    app_sender.emit(ShellInput::TogglePowerMenu(active_monitor().await));
                }
                IPCCommand::MediaPlayer => {
                    app_sender.emit(ShellInput::ToggleMediaPlayerMenu(active_monitor().await));
                }
                IPCCommand::Session => {
                    app_sender.emit(ShellInput::ToggleSessionMenu(active_monitor().await));
                }
                IPCCommand::Dashboard => {
                    app_sender.emit(ShellInput::ToggleDashboardMenu(active_monitor().await));
                }
                IPCCommand::SessionAction(action) => {
                    app_sender.emit(ShellInput::RunSessionAction(action));
                }
                IPCCommand::PluginMenu(key) => {
                    app_sender.emit(ShellInput::TogglePluginMenu(active_monitor().await, key));
                }
                IPCCommand::PluginReload(key) => {
                    app_sender.emit(ShellInput::ReloadPlugin(key));
                }
                IPCCommand::PluginKeybind(key, id) => {
                    app_sender.emit(ShellInput::FirePluginKeybind(
                        active_monitor().await,
                        key,
                        id,
                    ));
                }
                IPCCommand::CloseAllMenus => app_sender.emit(ShellInput::CloseAllMenus),
                IPCCommand::VolumeUp => {
                    if let Some(output) = audio_service().default_output.get() {
                        let current_volume = output.volume.get();
                        let max_volume: f64 = 1.0;
                        let new_volume = max_volume.min(current_volume.average() + 0.05);
                        let _ = output
                            .set_volume(Volume::stereo(new_volume, new_volume))
                            .await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::VolumeDown => {
                    if let Some(output) = audio_service().default_output.get() {
                        let current_volume = output.volume.get();
                        let min_volume: f64 = 0.0;
                        let new_volume = min_volume.max(current_volume.average() - 0.05);
                        let _ = output
                            .set_volume(Volume::stereo(new_volume, new_volume))
                            .await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::Mute => {
                    if let Some(output) = audio_service().default_output.get() {
                        let _ = output.set_mute(!output.muted.get()).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MicMute => {
                    // Toggle the default *source* (microphone). The mic
                    // OSD watches `default_input.muted` and pops the
                    // bottom-centre pill on the change — no explicit
                    // show needed here, same as the volume path.
                    if let Some(input) = audio_service().default_input.get() {
                        let _ = input.set_mute(!input.muted.get()).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::VolumeSet(frac) => {
                    if let Some(output) = audio_service().default_output.get() {
                        let v = frac.clamp(0.0, 1.5);
                        let _ = output.set_volume(Volume::stereo(v, v)).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MuteSet(mode) => {
                    if let Some(output) = audio_service().default_output.get() {
                        let target = match mode {
                            0 => false,
                            1 => true,
                            _ => !output.muted.get(),
                        };
                        let _ = output.set_mute(target).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MicUp => {
                    if let Some(input) = audio_service().default_input.get() {
                        let nv = 1.0_f64.min(input.volume.get().average() + 0.05);
                        let _ = input.set_volume(Volume::stereo(nv, nv)).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MicDown => {
                    if let Some(input) = audio_service().default_input.get() {
                        let nv = 0.0_f64.max(input.volume.get().average() - 0.05);
                        let _ = input.set_volume(Volume::stereo(nv, nv)).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MicVolumeSet(frac) => {
                    if let Some(input) = audio_service().default_input.get() {
                        let v = frac.clamp(0.0, 1.5);
                        let _ = input.set_volume(Volume::stereo(v, v)).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MicMuteSet(mode) => {
                    if let Some(input) = audio_service().default_input.get() {
                        let target = match mode {
                            0 => false,
                            1 => true,
                            _ => !input.muted.get(),
                        };
                        let _ = input.set_mute(target).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::SwitchOutput(target) => {
                    let devs = usable_outputs();
                    let names: Vec<(String, String)> = devs
                        .iter()
                        .map(|d| (d.name.get(), d.description.get()))
                        .collect();
                    let cur = audio_service().default_output.get().map(|d| d.name.get());
                    if let Some(i) = pick_device(&names, cur.as_deref(), &target)
                        && devs[i].set_as_default().await.is_ok()
                    {
                        notify_audio("Audio output", &devs[i].description.get());
                    }
                }
                IPCCommand::SwitchInput(target) => {
                    let devs = usable_inputs();
                    let names: Vec<(String, String)> = devs
                        .iter()
                        .map(|d| (d.name.get(), d.description.get()))
                        .collect();
                    let cur = audio_service().default_input.get().map(|d| d.name.get());
                    if let Some(i) = pick_device(&names, cur.as_deref(), &target)
                        && devs[i].set_as_default().await.is_ok()
                    {
                        notify_audio("Audio input", &devs[i].description.get());
                    }
                }
                IPCCommand::MediaToggle(target) => {
                    if let Some(p) = pick_player(&target) {
                        let _ = p.play_pause().await;
                        notify_media(p);
                    }
                }
                IPCCommand::MediaNext(target) => {
                    if let Some(p) = pick_player(&target) {
                        let _ = p.next().await;
                        notify_media(p);
                    }
                }
                IPCCommand::MediaPrev(target) => {
                    if let Some(p) = pick_player(&target) {
                        let _ = p.previous().await;
                        notify_media(p);
                    }
                }
                IPCCommand::BrightnessUp => {
                    if let Some(brightness_service) = brightness_service()
                        && let Some(primary) = brightness_service.primary.get()
                    {
                        let current_brightness = primary.percentage().value();
                        let max_brightness: f64 = 100.0;
                        let new_brightness = max_brightness.min(current_brightness + 5.0);
                        let _ = primary
                            .set_percentage(Percentage::new(new_brightness))
                            .await;
                    }
                }
                IPCCommand::BrightnessDown => {
                    if let Some(brightness_service) = brightness_service()
                        && let Some(primary) = brightness_service.primary.get()
                    {
                        let current_brightness = primary.percentage().value();
                        let min_brightness: f64 = 0.0;
                        let new_brightness = min_brightness.max(current_brightness - 5.0);
                        let _ = primary
                            .set_percentage(Percentage::new(new_brightness))
                            .await;
                    }
                }
                IPCCommand::SetWallpaper(path) => {
                    set_wallpaper(&path);
                }
                IPCCommand::Lock => {
                    lock_session();
                }
                IPCCommand::CheckLock(reply) => {
                    let _ = reply.send(session_locked());
                }
                IPCCommand::Screenshare(reply, payload) => {
                    app_sender.emit(ShellInput::ToggleScreenshareMenu(
                        active_monitor().await,
                        reply,
                        payload,
                    ));
                }
                IPCCommand::SelectRegion(reply) => {
                    // Bridge for mscreenshot CLI: open the in-shell
                    // area selector and reply with "X,Y WxH" (slurp
                    // format) when the user commits, empty string
                    // on cancel. Runs entirely on the GTK main loop
                    // because select_region builds layer-shell
                    // overlays — but the wait isn't blocking,
                    // select_screen's callback fires from the next
                    // event-loop tick after the user commits, and
                    // the reply channel delivers it back to the
                    // mshellctl client.
                    mshell_screenshot::select_screen(
                        mshell_screenshot::ScreenSelectAreaRequest::SelectRegion,
                        move |result| {
                            let geom = match result {
                                Ok(mshell_screenshot::ScreenSelection::Region(region)) => format!(
                                    "{},{} {}x{}",
                                    region.x, region.y, region.width, region.height
                                ),
                                _ => String::new(),
                            };
                            let _ = reply.send(geom);
                        },
                    );
                }
                IPCCommand::OpenSettings => {
                    open_settings();
                }
                IPCCommand::OpenWizard => {
                    open_wizard();
                }
                IPCCommand::CloseSettings => {
                    close_settings();
                }
                IPCCommand::Inspect => {
                    gtk::Window::set_interactive_debugging(true);
                }
                IPCCommand::BarToggleTop => {
                    app_sender.emit(ShellInput::BarToggleTop(active_monitor().await));
                }
                IPCCommand::BarToggleBottom => {
                    app_sender.emit(ShellInput::BarToggleBottom(active_monitor().await));
                }
                IPCCommand::BarToggleLeft => {
                    app_sender.emit(ShellInput::BarToggleLeft(active_monitor().await));
                }
                IPCCommand::BarToggleRight => {
                    app_sender.emit(ShellInput::BarToggleRight(active_monitor().await));
                }
                IPCCommand::BarToggleAll(exclude_hidden_by_default) => {
                    app_sender.emit(ShellInput::BarToggleAll(
                        active_monitor().await,
                        exclude_hidden_by_default,
                    ));
                }
                IPCCommand::BarRevealAll(exclude_hidden_by_default) => {
                    app_sender.emit(ShellInput::BarRevealAll(
                        active_monitor().await,
                        exclude_hidden_by_default,
                    ));
                }
                IPCCommand::BarHideAll(exclude_hidden_by_default) => {
                    app_sender.emit(ShellInput::BarHideAll(
                        active_monitor().await,
                        exclude_hidden_by_default,
                    ));
                }
            }
        }
    });
}

/// Tabs the app launcher renders in the category strip, in the
/// order the AppLauncherModel registers its providers. Kept here
/// — rather than asking the launcher runtime at IPC-query time
/// — because the runtime only exists while the launcher panel is
/// open; the wizard / CLI consumer wants the list any time
/// mshell is running.
///
/// If you add a new provider whose `category()` returns a fresh
/// string, append it here so `mshellctl menu app-launcher
/// --list-tabs` stays accurate.
pub const APP_LAUNCHER_TABS: &[&str] = &[
    "All",
    "Run",
    "System",
    "Insert",
    "Search",
    "Compositor",
    "Connect",
];

enum IPCCommand {
    Quit,
    AppLauncher,
    /// `mshellctl menu app-launcher --tab <name>` — open the
    /// launcher and pre-select the named category tab. Unknown
    /// names silently fall back to "All".
    AppLauncherTab(String),
    Clock,
    Clipboard,
    Notifications,
    NotificationsClearAll,
    NotificationsReadPopups,
    Session,
    SessionAction(SessionAction),
    Screenshot,
    /// Headless capture: `"<area> <target> <delay>"` (see
    /// `mshellctl screenshot`).
    ScreenshotCapture(String),
    /// Screen recording: `"<action> <area> <audio>"` (see
    /// `mshellctl screenrecord`).
    ScreenRecord(String),
    Wallpaper,
    Ufw,
    Privacy,
    Bluetooth,
    /// Drive the native auto-connect engine: "toggle" (smart) | "connect" |
    /// "disconnect". Backs `mshellctl bluetooth …` (bind to F10).
    BluetoothCtl(String),
    CpuDashboard,
    AudioDashboard,
    SystemUpdate,
    Valent,
    KeepAwake,
    Twilight,
    Weather,
    Keybinds,
    AlarmClock,
    ControlCenter,
    HiddenBar(mshell_common::hidden_bar::HiddenBarVerb),
    SshSessions,
    Dns,
    Podman,
    Notes,
    Ip,
    Network,
    Power,
    MediaPlayer,
    Dashboard,
    /// Toggle an installed plugin's panel/menu by key (generic — any plugin).
    PluginMenu(String),
    /// Force-reload an installed plugin's WASM panel — evict the cached
    /// instance so the next open instantiates from disk.
    PluginReload(String),
    /// Fire a registered plugin keybind: `(plugin-key, bind-id)`. Opens
    /// the plugin's panel and delivers a `Keybind` event with the id.
    PluginKeybind(String, String),
    CloseAllMenus,
    VolumeUp,
    VolumeDown,
    /// Set the default output volume to an absolute fraction (0.0–1.5).
    VolumeSet(f64),
    /// 0 = unmute, 1 = mute, anything else = toggle (default output).
    MuteSet(i32),
    MicUp,
    MicDown,
    /// Set the default input (mic) volume to an absolute fraction.
    MicVolumeSet(f64),
    /// 0 = unmute, 1 = mute, anything else = toggle (default input).
    MicMuteSet(i32),
    /// Make a different output the default: "next" | "prev" | an index | a
    /// case-insensitive name / description fragment.
    SwitchOutput(String),
    /// Same for the default input.
    SwitchInput(String),
    /// Media control. The `String` targets a player: empty = the active one,
    /// else a case-insensitive identity fragment (`spotify`, `browser`, …).
    MediaToggle(String),
    MediaNext(String),
    MediaPrev(String),
    Mute,
    MicMute,
    BrightnessUp,
    BrightnessDown,
    SetWallpaper(PathBuf),
    WallpaperCycle(mshell_cache::wallpaper::CycleDirection),
    Lock,
    CheckLock(tokio::sync::oneshot::Sender<bool>),
    Screenshare(tokio::sync::oneshot::Sender<String>, String),
    /// mscreenshot CLI bridge — opens the area selector and replies
    /// with "X,Y WxH" (slurp format) when the user commits, or an
    /// empty string when they cancel. Lets `mscreenshot area` use
    /// the rich in-shell selector (preview state, snap, aspect
    /// info) instead of the bare slurp overlay when mshell is up.
    SelectRegion(tokio::sync::oneshot::Sender<String>),
    /// Headless clipboard op driven by `mshellctl clipboard …`. Spec is
    /// `"copy <id>" | "pin <id>" | "unpin <id>" | "delete <id>" |
    /// "clear" | "wipe"`.
    ClipboardAction(String),
    /// `mshellctl clipboard list` — reply with one `id\tcategory\tpreview`
    /// line per entry (newest first).
    ClipboardList(tokio::sync::oneshot::Sender<String>),
    OpenSettings,
    OpenWizard,
    CloseSettings,
    Inspect,
    BarToggleTop,
    BarToggleBottom,
    BarToggleLeft,
    BarToggleRight,
    BarToggleAll(bool),
    BarRevealAll(bool),
    BarHideAll(bool),
}

/// A sink whose active port is actually connected — drops e.g. an HDMI /
/// DisplayPort output with nothing plugged in (its active port reports
/// `available = false`), so cycling never lands on a dead sink. Devices with
/// no port concept (virtual sinks) are kept.
fn output_connected(d: &OutputDevice) -> bool {
    match d.active_port.get() {
        None => true,
        Some(active) => d
            .ports
            .get()
            .iter()
            .find(|p| p.name == active)
            .map(|p| p.available)
            .unwrap_or(true),
    }
}

/// Real, switchable output sinks (skips unplugged HDMI/DP ports). Sorted by
/// the stable PipeWire device index so `list`, `status` and `next`/`prev` all
/// see the SAME order — `input_devices.get()` / `output_devices.get()` don't
/// guarantee a stable order between calls, which made cycling skip a device
/// and the switch notification name the wrong one.
///
/// When the `audio.hide_hdmi_outputs` config toggle is on, sinks whose node
/// name or description matches "hdmi"/"displayport" are also excluded.
fn usable_outputs() -> Vec<Arc<OutputDevice>> {
    let hide_hdmi = config_manager()
        .config()
        .audio()
        .hide_hdmi_outputs()
        .get_untracked();
    let mut v: Vec<_> = audio_service()
        .output_devices
        .get()
        .into_iter()
        .filter(|d| output_connected(d))
        .filter(|d| !(hide_hdmi && mshell_utils::audio::is_hdmi_output(d)))
        .collect();
    v.sort_by_key(|d| d.key.index);
    v
}

/// Real capture sources — drops PulseAudio monitor sources (the loopback
/// "Monitor of <sink>" entries), which aren't microphones. Same stable sort.
fn usable_inputs() -> Vec<Arc<InputDevice>> {
    let mut v: Vec<_> = audio_service()
        .input_devices
        .get()
        .into_iter()
        .filter(|d| !d.is_monitor.get())
        .collect();
    v.sort_by_key(|d| d.key.index);
    v
}

/// Fire-and-forget desktop notification (replaces the previous one via the
/// synchronous hint so rapid switches don't stack), mirroring osc-soundctl.
fn notify_audio(summary: &str, body: &str) {
    let summary = summary.to_string();
    let body = body.to_string();
    relm4::spawn(async move {
        let _ = tokio::process::Command::new("notify-send")
            .args([
                "-a",
                "mshell",
                "-i",
                "audio-volume-high-symbolic",
                "-h",
                "string:x-canonical-private-synchronous:mshell-audio",
                &summary,
                &body,
            ])
            .status()
            .await;
    });
}

/// Toast the player + current track after a media action (osc-media style).
/// Spawned with a short settle delay because MPRIS pushes the new track /
/// playback state asynchronously after `next` / `play_pause` returns — reading
/// immediately would name the *previous* track.
fn notify_media(player: Arc<Player>) {
    relm4::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        let glyph = match player.playback_state.get() {
            PlaybackState::Playing => "▶",
            PlaybackState::Paused => "⏸",
            PlaybackState::Stopped => "⏹",
        };
        let title = player.metadata.title.get();
        let artist = player.metadata.artist.get();
        let body = match (title.trim(), artist.trim()) {
            ("", "") => format!("{glyph} {}", playback_label(player.playback_state.get())),
            (t, "") => format!("{glyph} {t}"),
            (t, a) => format!("{glyph} {t} · {a}"),
        };
        // Album art from the MediaService's art cache when available, else a
        // generic player glyph.
        let icon = player
            .metadata
            .cover_art
            .get()
            .or_else(|| player.metadata.art_url.get())
            .map(|p| p.trim_start_matches("file://").to_string())
            .filter(|p| std::path::Path::new(p).is_file())
            .unwrap_or_else(|| "multimedia-player-symbolic".to_string());
        let _ = tokio::process::Command::new("notify-send")
            .args([
                "-a",
                "mshell",
                "-i",
                &icon,
                "-h",
                "string:x-canonical-private-synchronous:mshell-media",
                &player.identity.get(),
                &body,
            ])
            .status()
            .await;
    });
}

/// Resolve a switch target against a device list. `names` is `(node_name,
/// description)` per device, `current` the default's node name. Accepts
/// `next` / `prev` / `switch`, a numeric index, or a case-insensitive
/// fragment matched against the description first then the node name.
/// A parsed `mshellctl clipboard <verb> [id]` action.
#[derive(Debug, PartialEq, Eq)]
enum ClipboardAction {
    Copy(u64),
    TogglePin(u64),
    Delete(u64),
    Clear,
    Wipe,
}

/// Parse a `clipboard` IPC spec (`"<verb> [id]"`) into an action, or `None`
/// for an unknown verb / missing-or-malformed id where one is required.
fn parse_clipboard_action(spec: &str) -> Option<ClipboardAction> {
    let mut it = spec.split_whitespace();
    let verb = it.next().unwrap_or("");
    let id = it.next().and_then(|s| s.parse::<u64>().ok());
    match (verb, id) {
        ("copy", Some(id)) => Some(ClipboardAction::Copy(id)),
        ("pin", Some(id)) | ("unpin", Some(id)) => Some(ClipboardAction::TogglePin(id)),
        ("delete", Some(id)) => Some(ClipboardAction::Delete(id)),
        ("clear", _) => Some(ClipboardAction::Clear),
        ("wipe", _) => Some(ClipboardAction::Wipe),
        _ => None,
    }
}

fn pick_device(names: &[(String, String)], current: Option<&str>, target: &str) -> Option<usize> {
    if names.is_empty() {
        return None;
    }
    let cur = current.and_then(|c| names.iter().position(|(n, _)| n == c));
    let t = target.trim();
    match t.to_ascii_lowercase().as_str() {
        "next" | "switch" => return Some(cur.map(|c| (c + 1) % names.len()).unwrap_or(0)),
        "prev" | "previous" => {
            return Some(
                cur.map(|c| (c + names.len() - 1) % names.len())
                    .unwrap_or(0),
            );
        }
        _ => {}
    }
    if let Ok(i) = t.parse::<usize>()
        && i < names.len()
    {
        return Some(i);
    }
    let tl = t.to_lowercase();
    names
        .iter()
        .position(|(n, d)| d.to_lowercase().contains(&tl) || n.to_lowercase().contains(&tl))
}

#[derive(serde::Serialize)]
struct DeviceSnapshot {
    index: usize,
    /// Technical PipeWire node name (`alsa_output.pci-…`).
    name: String,
    /// Friendly label ("Logitech Z205 Analog Stereo").
    description: String,
    volume_percent: u32,
    muted: bool,
    /// Whether this is the current default device.
    active: bool,
}

#[derive(serde::Serialize)]
struct AudioSnapshot {
    outputs: Vec<DeviceSnapshot>,
    inputs: Vec<DeviceSnapshot>,
}

/// Read the live audio service into a serialisable snapshot (sync `.get()`s,
/// same direct-global pattern as `notification_count`).
fn audio_snapshot() -> AudioSnapshot {
    let svc = audio_service();
    let def_out = svc.default_output.get().map(|d| d.name.get());
    let def_in = svc.default_input.get().map(|d| d.name.get());
    let snap = |i: usize,
                name: String,
                description: String,
                vol: f64,
                muted: bool,
                def: &Option<String>| {
        DeviceSnapshot {
            index: i,
            active: def.as_deref() == Some(name.as_str()),
            volume_percent: (vol * 100.0).round() as u32,
            muted,
            description,
            name,
        }
    };
    let outputs = usable_outputs()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            snap(
                i,
                d.name.get(),
                d.description.get(),
                d.volume.get().average(),
                d.muted.get(),
                &def_out,
            )
        })
        .collect();
    let inputs = usable_inputs()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            snap(
                i,
                d.name.get(),
                d.description.get(),
                d.volume.get().average(),
                d.muted.get(),
                &def_in,
            )
        })
        .collect();
    AudioSnapshot { outputs, inputs }
}

fn fmt_device_line(d: &DeviceSnapshot, icon: &str) -> String {
    let mut tags = String::new();
    if d.active {
        tags.push_str("  [active]");
    }
    if d.muted {
        tags.push_str("  [muted]");
    }
    format!(
        "  {}: {} {:<42} {:>3}%{}",
        d.index, icon, d.description, d.volume_percent, tags
    )
}

/// `mshellctl audio list` body — human table or `--json`.
fn render_audio_list(as_json: bool) -> String {
    let snap = audio_snapshot();
    if as_json {
        return serde_json::to_string_pretty(&snap).unwrap_or_else(|_| "{}".into());
    }
    let mut out = String::from("Outputs:\n");
    if snap.outputs.is_empty() {
        out.push_str("  (none)\n");
    }
    for d in &snap.outputs {
        out.push_str(&fmt_device_line(d, "🔊"));
        out.push('\n');
    }
    out.push_str("\nInputs:\n");
    if snap.inputs.is_empty() {
        out.push_str("  (none)\n");
    }
    for d in &snap.inputs {
        out.push_str(&fmt_device_line(d, "🎤"));
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// `mshellctl audio status` body — the current default out/in.
fn render_audio_status(as_json: bool) -> String {
    let snap = audio_snapshot();
    let out = snap.outputs.into_iter().find(|d| d.active);
    let inp = snap.inputs.into_iter().find(|d| d.active);
    if as_json {
        let v = serde_json::json!({ "output": out, "input": inp });
        return serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into());
    }
    let line = |d: Option<&DeviceSnapshot>, label: &str| match d {
        Some(d) => format!(
            "{label}: {} — {}%{}",
            d.description,
            d.volume_percent,
            if d.muted { " (muted)" } else { "" }
        ),
        None => format!("{label}: (none)"),
    };
    format!(
        "{}\n{}",
        line(out.as_ref(), "Output"),
        line(inp.as_ref(), "Input")
    )
}

// ── Media players (MPRIS, via the shell's MediaService) ─────────────────────
fn playback_label(s: PlaybackState) -> &'static str {
    match s {
        PlaybackState::Playing => "Playing",
        PlaybackState::Paused => "Paused",
        PlaybackState::Stopped => "Stopped",
    }
}

/// Resolve a media target to a player. Empty = the active player (else the
/// first one); otherwise a case-insensitive match on the player identity,
/// with a `browser` alias and `mpd`/`mpc` → "Music Player Daemon".
///
/// When a fragment matches several players — e.g. `browser` with three
/// Chromium instances — prefer the one that's actually **Playing** (then
/// Paused, then Stopped), tie-breaking toward the active player. The MPRIS
/// player list is built from a `HashMap`, so its order isn't stable; a plain
/// "first match" would toggle an arbitrary (often silent) instance.
fn pick_player(target: &str) -> Option<Arc<Player>> {
    let svc = media_service();
    let t = target.trim().to_lowercase();
    if t.is_empty() {
        return svc
            .active_player()
            .or_else(|| svc.players().into_iter().next());
    }
    // Match the fragment against the identity, the D-Bus bus name, AND the
    // desktop entry. The bus name is the robust signal: a Chromium/Firefox
    // fork inherits the engine's MPRIS service name even when it rebrands
    // its Identity — e.g. Helium reports identity "Helium" but registers as
    // `org.mpris.MediaPlayer2.chromium.instance…`. So the `chrome` / `chromium`
    // / `firefox` tokens below catch essentially every mainstream browser and
    // fork via the bus prefix; the rest are branded-identity fallbacks. (A
    // browser that rebrands all three fields with no engine token won't hit
    // the `browser` alias — use its explicit name, e.g. `media toggle helium`.)
    // osc-media keys on `playerctl -l` bus names for the same reason.
    let matches = |p: &Arc<Player>| -> bool {
        let bus = p.id.bus_name();
        let bus = bus.strip_prefix("org.mpris.MediaPlayer2.").unwrap_or(bus);
        let desktop = p.desktop_entry.get().unwrap_or_default();
        let hay = format!("{} {} {}", p.identity.get(), bus, desktop).to_lowercase();
        hay.contains(&t)
            || (t == "browser"
                && [
                    "firefox",
                    "chrome",
                    "chromium",
                    "brave",
                    "edge",
                    "vivaldi",
                    "opera",
                    "webcord",
                    "zen",
                    "librewolf",
                    "waterfox",
                    "floorp",
                    "helium",
                    "thorium",
                    "ungoogled",
                    "palemoon",
                    "midori",
                    "epiphany",
                    "falkon",
                    "qutebrowser",
                ]
                .iter()
                .any(|b| hay.contains(b)))
            || ((t == "mpd" || t == "mpc") && hay.contains("music player daemon"))
    };
    let active = svc.active_player();
    svc.players().into_iter().filter(matches).max_by_key(|p| {
        let state = match p.playback_state.get() {
            PlaybackState::Playing => 2,
            PlaybackState::Paused => 1,
            PlaybackState::Stopped => 0,
        };
        let is_active = active.as_ref().map(|a| Arc::ptr_eq(a, p)).unwrap_or(false);
        (state, is_active)
    })
}

#[derive(serde::Serialize)]
struct PlayerSnapshot {
    identity: String,
    state: String,
    title: String,
    artist: String,
    active: bool,
}

fn media_snapshot() -> Vec<PlayerSnapshot> {
    let svc = media_service();
    let active = svc.active_player();
    svc.players()
        .into_iter()
        .map(|p| PlayerSnapshot {
            active: active.as_ref().map(|a| Arc::ptr_eq(a, &p)).unwrap_or(false),
            identity: p.identity.get(),
            state: playback_label(p.playback_state.get()).to_string(),
            title: p.metadata.title.get(),
            artist: p.metadata.artist.get(),
        })
        .collect()
}

/// `mshellctl media list` body.
fn render_media_list(as_json: bool) -> String {
    let snap = media_snapshot();
    if as_json {
        return serde_json::to_string_pretty(&snap).unwrap_or_else(|_| "[]".into());
    }
    if snap.is_empty() {
        return "No media players".to_string();
    }
    snap.iter()
        .map(|p| {
            let icon = if p.state == "Playing" { "▶" } else { "⏸" };
            let track = match (p.title.as_str(), p.artist.as_str()) {
                ("", "") => String::new(),
                (t, "") => format!("  — {t}"),
                (t, a) => format!("  — {t} · {a}"),
            };
            let tag = if p.active { "  [active]" } else { "" };
            format!("{icon} {} [{}]{track}{tag}", p.identity, p.state)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `mshellctl media status` body — the active player.
fn render_media_status(as_json: bool) -> String {
    let active = media_snapshot().into_iter().find(|p| p.active);
    if as_json {
        return serde_json::to_string_pretty(&active).unwrap_or_else(|_| "null".into());
    }
    match active {
        None => "No active media player".to_string(),
        Some(p) => {
            let track = match (p.title.as_str(), p.artist.as_str()) {
                ("", "") => String::new(),
                (t, "") => format!(" — {t}"),
                (t, a) => format!(" — {t} · {a}"),
            };
            format!("{} [{}]{track}", p.identity, p.state)
        }
    }
}

/// File-watcher hot reload for the WASM plugin tier. Watches every installed
/// plugin's directory; when its `plugin.wasm` changes (e.g. `cargo build`
/// drops a new build), we send `PluginReload(key)` so the cached panel is
/// evicted and the next open instantiates from disk — no mshell restart, no
/// manual `mshellctl plugin reload`.
fn spawn_plugin_watcher_task(tx: mpsc::UnboundedSender<IPCCommand>) {
    tokio::task::spawn_blocking(move || {
        use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;
        use std::sync::mpsc as sync_mpsc;
        use std::time::{Duration, Instant};

        let store = mshell_plugins::PluginStore::new();
        let installed = store.installed();
        if installed.is_empty() {
            return;
        }

        let (notify_tx, notify_rx) = sync_mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = match RecommendedWatcher::new(
            move |res| {
                let _ = notify_tx.send(res);
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("plugin watcher init failed: {e}");
                return;
            }
        };

        // Watch each installed plugin's directory; map paths back to their keys.
        let mut by_dir: HashMap<PathBuf, String> = HashMap::new();
        for p in installed {
            if let Err(e) = watcher.watch(&p.dir, RecursiveMode::NonRecursive) {
                tracing::warn!(plugin = %p.key, "watcher.watch failed: {e}");
                continue;
            }
            by_dir.insert(p.dir, p.key);
        }

        // Debounce: editors / cargo write the wasm in multiple steps (tmp + rename).
        // Collect events for 300 ms after the last one, then flush one reload per key.
        let debounce = Duration::from_millis(300);
        let mut pending: HashSet<String> = HashSet::new();
        let mut last_event: Option<Instant> = None;

        loop {
            let wait = match last_event {
                Some(t) => debounce.saturating_sub(t.elapsed()),
                None => Duration::from_secs(60),
            };
            match notify_rx.recv_timeout(wait) {
                Ok(Ok(ev)) => {
                    if !matches!(ev.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                        continue;
                    }
                    for path in &ev.paths {
                        if path.file_name().is_some_and(|n| n == "plugin.wasm")
                            && let Some(parent) = path.parent()
                            && let Some(key) = by_dir.get(parent)
                        {
                            pending.insert(key.clone());
                            last_event = Some(Instant::now());
                        }
                    }
                }
                Ok(Err(e)) => tracing::debug!("plugin watcher event err: {e}"),
                Err(sync_mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(t) = last_event
                        && t.elapsed() >= debounce
                    {
                        for key in pending.drain() {
                            tracing::info!(plugin = %key, "hot-reload: plugin.wasm changed");
                            let _ = tx.send(IPCCommand::PluginReload(key));
                        }
                        last_event = None;
                    }
                }
                Err(sync_mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

/// Daily-wallpaper auto-fetch (port of the noctalia `daily-wallpaper` plugin).
/// Main-thread glib timers do the *check* (reading the reactive config is only
/// safe on the main thread); the actual blocking download runs on a worker and
/// the resulting path is sent back as `SetWallpaper` so it's applied on main.
fn spawn_daily_wallpaper_task(tx: mpsc::UnboundedSender<IPCCommand>) {
    let last_applied: std::rc::Rc<std::cell::RefCell<String>> =
        std::rc::Rc::new(std::cell::RefCell::new(String::new()));

    // Apply shortly after login.
    {
        let tx = tx.clone();
        let last = last_applied.clone();
        glib::timeout_add_local_once(Duration::from_secs(15), move || run_daily_check(&tx, &last));
    }
    // Re-check every 30 min so a date rollover swaps to the new day's image.
    glib::timeout_add_seconds_local(30 * 60, move || {
        run_daily_check(&tx, &last_applied);
        glib::ControlFlow::Continue
    });
}

/// One daily-wallpaper check (main thread). Skips when disabled or already done
/// for today; otherwise kicks the blocking fetch on a worker and applies the
/// result via `SetWallpaper`.
fn run_daily_check(
    tx: &mpsc::UnboundedSender<IPCCommand>,
    last: &std::rc::Rc<std::cell::RefCell<String>>,
) {
    // Each reactive-store accessor consumes the subfield, so re-walk the chain.
    if !config_manager()
        .config()
        .wallpaper()
        .daily_wallpaper_enabled()
        .get_untracked()
    {
        return;
    }
    let Some(today) = glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|s| s.to_string())
    else {
        return;
    };
    if *last.borrow() == today {
        return;
    }
    *last.borrow_mut() = today;

    let source = config_manager()
        .config()
        .wallpaper()
        .daily_wallpaper_source()
        .get_untracked();
    let locale = config_manager()
        .config()
        .wallpaper()
        .daily_wallpaper_locale()
        .get_untracked();
    let tx = tx.clone();
    tokio_rt().spawn(async move {
        let fetched = tokio::task::spawn_blocking(move || {
            mshell_cache::wallpaper::fetch_daily_wallpaper(&source, &locale)
        })
        .await;
        match fetched {
            Ok(Ok(path)) => {
                let _ = tx.send(IPCCommand::SetWallpaper(path));
            }
            Ok(Err(e)) => tracing::warn!(error = %e, "daily wallpaper: fetch failed"),
            Err(e) => tracing::warn!(error = %e, "daily wallpaper: task join failed"),
        }
    });
}

// ── Alarm scheduler (port of the DMS alarmClock plugin) ─────────────────────
// A single main-thread glib timer ticks each second, reads the alarms from the
// config (reactive reads are main-thread-only), and fires matching alarms once
// per clock minute. The ring tone + the Stop/Snooze notification run on
// workers; Snooze re-fire times go on a thread-safe queue the tick drains.
static ALARM_SNOOZES: std::sync::Mutex<Vec<std::time::SystemTime>> =
    std::sync::Mutex::new(Vec::new());

fn spawn_alarm_scheduler() {
    let last_minute = std::rc::Rc::new(std::cell::Cell::new(i64::MIN));
    glib::timeout_add_seconds_local(1, move || {
        run_alarm_tick(&last_minute);
        glib::ControlFlow::Continue
    });
}

fn run_alarm_tick(last_minute: &std::rc::Rc<std::cell::Cell<i64>>) {
    let Some(now) = glib::DateTime::now_local().ok() else {
        return;
    };

    // Due snooze re-fires (every tick, independent of the per-minute gate).
    let sys_now = std::time::SystemTime::now();
    let due = {
        let mut q = ALARM_SNOOZES.lock().unwrap_or_else(|e| e.into_inner());
        let due = q.iter().filter(|t| **t <= sys_now).count();
        q.retain(|t| *t > sys_now);
        due
    };
    if due > 0 {
        fire_alarm("Snoozed alarm".to_string());
    }

    // Scheduled alarms — fire at most once per clock minute.
    let minute_key = now.to_unix() / 60;
    if minute_key == last_minute.get() {
        return;
    }
    last_minute.set(minute_key);

    let hour = now.hour() as u8;
    let minute = now.minute() as u8;
    let weekday = (now.day_of_week() % 7) as u8; // 0 = Sunday … 6 = Saturday

    let alarms = config_manager().config().alarm().alarms().get_untracked();
    for (i, alarm) in alarms.iter().enumerate() {
        if !alarm.enabled || alarm.hour != hour || alarm.minutes != minute {
            continue;
        }
        if alarm.repeat_mask != 0 && (alarm.repeat_mask & (1 << weekday)) == 0 {
            continue; // repeating, but not on today's weekday
        }
        let label = if alarm.name.trim().is_empty() {
            "Alarm".to_string()
        } else {
            alarm.name.clone()
        };
        fire_alarm(label);
        if alarm.repeat_mask == 0 {
            // One-shot: disable after it fires.
            config_manager().update_config(move |c| {
                if let Some(a) = c.alarm.alarms.get_mut(i) {
                    a.enabled = false;
                }
            });
        }
    }
}

/// Ring the tone + (optionally) pop a Stop/Snooze notification. The
/// notification blocks a worker thread until the user acts; the tone stops on
/// any outcome, and Snooze re-queues a fire `snooze_minutes` later.
fn fire_alarm(label: String) {
    play_alarm_loop();
    if !config_manager()
        .config()
        .alarm()
        .notifications()
        .get_untracked()
    {
        return;
    }
    let snooze_secs = (config_manager()
        .config()
        .alarm()
        .snooze_minutes()
        .get_untracked()
        .max(1) as u64)
        * 60;
    let urgency = config_manager().config().alarm().urgency().get_untracked();
    std::thread::spawn(move || {
        let action = std::process::Command::new("notify-send")
            .args([
                "-a",
                "Alarm Clock",
                "-i",
                "alarm-symbolic",
                "-u",
                &urgency,
                "-A",
                "stop=Stop",
                "-A",
                "snooze=Snooze",
                &label,
                "It's time.",
            ])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        stop_alarm(); // stop on every outcome (Stop / Snooze / dismiss)
        if action == "snooze"
            && let Ok(mut q) = ALARM_SNOOZES.lock()
        {
            q.push(std::time::SystemTime::now() + std::time::Duration::from_secs(snooze_secs));
        }
    });
}

struct IPCService {
    tx: mpsc::UnboundedSender<IPCCommand>,
}

impl IPCService {
    pub fn new(tx: mpsc::UnboundedSender<IPCCommand>) -> Self {
        Self { tx }
    }
}

#[interface(name = "com.mshell.Shell")]
impl IPCService {
    async fn quit(&self) {
        let _ = self.tx.send(IPCCommand::Quit);
    }
    async fn app_launcher(&self) {
        let _ = self.tx.send(IPCCommand::AppLauncher);
    }
    /// Open (or refocus) the app launcher and pre-select the
    /// named category tab. Unknown names silently fall back to
    /// "All" — the launcher's `select_category` is permissive.
    async fn app_launcher_tab(&self, tab: String) {
        let _ = self.tx.send(IPCCommand::AppLauncherTab(tab));
    }
    /// Return the known launcher category tab names so the CLI
    /// can offer `--list-tabs` without round-tripping through
    /// the live runtime (which only exists while the panel is
    /// open). See `APP_LAUNCHER_TABS` for the source list.
    async fn list_app_launcher_tabs(&self) -> Vec<String> {
        APP_LAUNCHER_TABS.iter().map(|s| (*s).to_string()).collect()
    }
    async fn clock(&self) {
        let _ = self.tx.send(IPCCommand::Clock);
    }
    async fn clipboard(&self) {
        let _ = self.tx.send(IPCCommand::Clipboard);
    }
    /// Headless clipboard op for `mshellctl clipboard copy|pin|unpin|delete|
    /// clear|wipe`. `spec` is `"<verb> [id]"`.
    async fn clipboard_action(&self, spec: String) {
        let _ = self.tx.send(IPCCommand::ClipboardAction(spec));
    }
    /// `mshellctl clipboard list` — `id\tcategory\tpreview` per line.
    async fn clipboard_list(&self) -> String {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.tx.send(IPCCommand::ClipboardList(tx));
        rx.await.unwrap_or_default()
    }
    async fn notifications(&self) {
        let _ = self.tx.send(IPCCommand::Notifications);
    }
    async fn notifications_clear_all(&self) {
        let _ = self.tx.send(IPCCommand::NotificationsClearAll);
    }
    async fn notifications_read_popups(&self) {
        let _ = self.tx.send(IPCCommand::NotificationsReadPopups);
    }
    /// Do Not Disturb — set/clear/toggle directly on the global
    /// notification service (the bar pill + popups subscribe to it).
    async fn notification_dnd_on(&self) {
        notification_service().set_dnd(true);
    }
    async fn notification_dnd_off(&self) {
        notification_service().set_dnd(false);
    }
    async fn notification_dnd_toggle(&self) {
        let service = notification_service();
        service.set_dnd(!service.dnd.get());
    }
    /// Number of notifications currently in history (for bars / scripts).
    async fn notification_count(&self) -> u32 {
        notification_service().notifications.get().len() as u32
    }
    async fn screenshot(&self) {
        let _ = self.tx.send(IPCCommand::Screenshot);
    }
    /// Headless capture driven by `mshellctl screenshot <area>`. `spec` is
    /// `"<area> <target> <delay>"`.
    async fn screenshot_capture(&self, spec: String) {
        let _ = self.tx.send(IPCCommand::ScreenshotCapture(spec));
    }
    /// Screen recording driven by `mshellctl screenrecord …`. `spec` is
    /// `"<action> <area> <audio>"`.
    async fn screen_record(&self, spec: String) {
        let _ = self.tx.send(IPCCommand::ScreenRecord(spec));
    }
    async fn wallpaper(&self) {
        let _ = self.tx.send(IPCCommand::Wallpaper);
    }
    async fn ufw(&self) {
        let _ = self.tx.send(IPCCommand::Ufw);
    }
    async fn privacy(&self) {
        let _ = self.tx.send(IPCCommand::Privacy);
    }
    async fn bluetooth(&self) {
        let _ = self.tx.send(IPCCommand::Bluetooth);
    }
    /// Native auto-connect engine: `toggle` | `connect` | `disconnect`.
    async fn bluetooth_ctl(&self, action: String) {
        let _ = self.tx.send(IPCCommand::BluetoothCtl(action));
    }
    async fn cpu_dashboard(&self) {
        let _ = self.tx.send(IPCCommand::CpuDashboard);
    }
    async fn audio_dashboard(&self) {
        let _ = self.tx.send(IPCCommand::AudioDashboard);
    }
    // ── Audio query / control (mshellctl audio …) ──────────────────────────
    // Queries read the live service directly (sync, like notification_count);
    // actions go through the command loop so the async PipeWire setters run on
    // the service's own context and the bar / volume-OSD react to them.
    async fn audio_list_text(&self) -> String {
        render_audio_list(false)
    }
    async fn audio_list_json(&self) -> String {
        render_audio_list(true)
    }
    async fn audio_status_text(&self) -> String {
        render_audio_status(false)
    }
    async fn audio_status_json(&self) -> String {
        render_audio_status(true)
    }
    /// Set the shell's file-log level live (error|warn|info|debug|trace).
    async fn log_level(&self, level: String) -> String {
        match mshell_logging::set_level(&level) {
            Ok(()) => format!("shell log level set to {level}"),
            Err(e) => format!("error: {e}"),
        }
    }
    /// Enable/disable the shell's file logging live.
    async fn log_enabled(&self, enabled: bool) -> String {
        match mshell_logging::set_enabled(enabled) {
            Ok(()) => format!(
                "shell file logging {}",
                if enabled { "enabled" } else { "disabled" }
            ),
            Err(e) => format!("error: {e}"),
        }
    }
    /// Absolute output volume as a percent (0–150).
    async fn audio_volume_set(&self, percent: f64) {
        let _ = self.tx.send(IPCCommand::VolumeSet(percent / 100.0));
    }
    /// Output mute: 0 = off, 1 = on, else toggle.
    async fn audio_mute_set(&self, mode: i32) {
        let _ = self.tx.send(IPCCommand::MuteSet(mode));
    }
    async fn audio_mic_volume_set(&self, percent: f64) {
        let _ = self.tx.send(IPCCommand::MicVolumeSet(percent / 100.0));
    }
    async fn audio_mic_mute_set(&self, mode: i32) {
        let _ = self.tx.send(IPCCommand::MicMuteSet(mode));
    }
    async fn audio_mic_up(&self) {
        let _ = self.tx.send(IPCCommand::MicUp);
    }
    async fn audio_mic_down(&self) {
        let _ = self.tx.send(IPCCommand::MicDown);
    }
    /// Switch the default output: "next" | "prev" | index | name fragment.
    async fn audio_output_switch(&self, target: String) {
        let _ = self.tx.send(IPCCommand::SwitchOutput(target));
    }
    async fn audio_input_switch(&self, target: String) {
        let _ = self.tx.send(IPCCommand::SwitchInput(target));
    }
    // ── Media (mshellctl media …) — empty target = the active player ───────
    async fn media_toggle(&self, target: String) {
        let _ = self.tx.send(IPCCommand::MediaToggle(target));
    }
    async fn media_next(&self, target: String) {
        let _ = self.tx.send(IPCCommand::MediaNext(target));
    }
    async fn media_prev(&self, target: String) {
        let _ = self.tx.send(IPCCommand::MediaPrev(target));
    }
    async fn media_list_text(&self) -> String {
        render_media_list(false)
    }
    async fn media_list_json(&self) -> String {
        render_media_list(true)
    }
    async fn media_status_text(&self) -> String {
        render_media_status(false)
    }
    async fn media_status_json(&self) -> String {
        render_media_status(true)
    }
    async fn system_update(&self) {
        let _ = self.tx.send(IPCCommand::SystemUpdate);
    }
    async fn valent(&self) {
        let _ = self.tx.send(IPCCommand::Valent);
    }
    async fn keep_awake(&self) {
        let _ = self.tx.send(IPCCommand::KeepAwake);
    }
    async fn twilight(&self) {
        let _ = self.tx.send(IPCCommand::Twilight);
    }
    async fn weather(&self) {
        let _ = self.tx.send(IPCCommand::Weather);
    }
    async fn keybinds(&self) {
        let _ = self.tx.send(IPCCommand::Keybinds);
    }
    async fn alarm_clock(&self) {
        let _ = self.tx.send(IPCCommand::AlarmClock);
    }
    async fn control_center(&self) {
        let _ = self.tx.send(IPCCommand::ControlCenter);
    }
    /// Control the Hidden Bar drawer: `toggle` / `expand` / `collapse` /
    /// `pin` / `unpin`. Unknown actions are ignored.
    async fn hidden_bar(&self, action: String) {
        if let Some(verb) = mshell_common::hidden_bar::HiddenBarVerb::from_action(&action) {
            let _ = self.tx.send(IPCCommand::HiddenBar(verb));
        }
    }
    async fn ssh_sessions(&self) {
        let _ = self.tx.send(IPCCommand::SshSessions);
    }
    async fn dns(&self) {
        let _ = self.tx.send(IPCCommand::Dns);
    }
    async fn podman(&self) {
        let _ = self.tx.send(IPCCommand::Podman);
    }
    async fn notes(&self) {
        let _ = self.tx.send(IPCCommand::Notes);
    }
    async fn ip(&self) {
        let _ = self.tx.send(IPCCommand::Ip);
    }
    async fn network(&self) {
        let _ = self.tx.send(IPCCommand::Network);
    }
    async fn power(&self) {
        let _ = self.tx.send(IPCCommand::Power);
    }
    async fn media_player(&self) {
        let _ = self.tx.send(IPCCommand::MediaPlayer);
    }
    async fn session(&self) {
        let _ = self.tx.send(IPCCommand::Session);
    }
    async fn dashboard(&self) {
        let _ = self.tx.send(IPCCommand::Dashboard);
    }
    async fn plugin_reload(&self, key: String) {
        let _ = self.tx.send(IPCCommand::PluginReload(key));
    }

    async fn plugin_keybind(&self, arg: String) {
        let (key, id) = arg.split_once('|').unwrap_or((arg.as_str(), ""));
        let _ = self
            .tx
            .send(IPCCommand::PluginKeybind(key.to_string(), id.to_string()));
    }

    async fn plugin_menu(&self, key: String) {
        let _ = self.tx.send(IPCCommand::PluginMenu(key));
    }
    async fn session_lock(&self) {
        let _ = self.tx.send(IPCCommand::SessionAction(SessionAction::Lock));
    }
    async fn session_logout(&self) {
        let _ = self
            .tx
            .send(IPCCommand::SessionAction(SessionAction::Logout));
    }
    async fn session_suspend(&self) {
        let _ = self
            .tx
            .send(IPCCommand::SessionAction(SessionAction::Suspend));
    }
    async fn session_reboot(&self) {
        let _ = self
            .tx
            .send(IPCCommand::SessionAction(SessionAction::Reboot));
    }
    async fn session_shutdown(&self) {
        let _ = self
            .tx
            .send(IPCCommand::SessionAction(SessionAction::Shutdown));
    }
    async fn close_all_menus(&self) {
        let _ = self.tx.send(IPCCommand::CloseAllMenus);
    }
    async fn volume_up(&self) {
        let _ = self.tx.send(IPCCommand::VolumeUp);
    }
    async fn volume_down(&self) {
        let _ = self.tx.send(IPCCommand::VolumeDown);
    }
    async fn mute(&self) {
        let _ = self.tx.send(IPCCommand::Mute);
    }
    async fn mic_mute(&self) {
        let _ = self.tx.send(IPCCommand::MicMute);
    }
    async fn brightness_up(&self) {
        let _ = self.tx.send(IPCCommand::BrightnessUp);
    }
    async fn brightness_down(&self) {
        let _ = self.tx.send(IPCCommand::BrightnessDown);
    }
    async fn set_wallpaper(&self, path: &str) {
        let _ = self.tx.send(IPCCommand::SetWallpaper(PathBuf::from(path)));
    }
    /// Step the wallpaper: `direction` is `next`, `previous` /
    /// `prev`, or `random`.
    async fn wallpaper_cycle(&self, direction: &str) {
        use mshell_cache::wallpaper::CycleDirection;
        let dir = match direction.to_ascii_lowercase().as_str() {
            "previous" | "prev" => CycleDirection::Previous,
            "random" => CycleDirection::Random,
            _ => CycleDirection::Next,
        };
        let _ = self.tx.send(IPCCommand::WallpaperCycle(dir));
    }
    async fn lock(&self) {
        let _ = self.tx.send(IPCCommand::Lock);
    }
    async fn check_lock(&self) -> bool {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.tx.send(IPCCommand::CheckLock(tx));
        rx.await.unwrap_or(false)
    }
    async fn screenshare(&self, payload: &str) -> String {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self
            .tx
            .send(IPCCommand::Screenshare(tx, payload.to_string()));
        rx.await.unwrap_or(String::new())
    }
    /// Open the in-shell area selector and block until the user
    /// commits a rect (or cancels). Returns the geometry in slurp
    /// format `"X,Y WxH"` on commit, empty string on cancel. Used
    /// by `mscreenshot` CLI's region capture path so the rich
    /// in-shell selector replaces `slurp` when mshell is running.
    async fn select_region(&self) -> String {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.tx.send(IPCCommand::SelectRegion(tx));
        rx.await.unwrap_or_default()
    }
    async fn open_settings(&self) {
        let _ = self.tx.send(IPCCommand::OpenSettings);
    }
    async fn open_wizard(&self) {
        let _ = self.tx.send(IPCCommand::OpenWizard);
    }
    async fn close_settings(&self) {
        let _ = self.tx.send(IPCCommand::CloseSettings);
    }
    async fn inspect(&self) {
        let _ = self.tx.send(IPCCommand::Inspect);
    }
    async fn bar_toggle_top(&self) {
        let _ = self.tx.send(IPCCommand::BarToggleTop);
    }
    async fn bar_toggle_bottom(&self) {
        let _ = self.tx.send(IPCCommand::BarToggleBottom);
    }
    async fn bar_toggle_left(&self) {
        let _ = self.tx.send(IPCCommand::BarToggleLeft);
    }
    async fn bar_toggle_right(&self) {
        let _ = self.tx.send(IPCCommand::BarToggleRight);
    }
    async fn bar_toggle_all(&self, exclude_hidden_by_default: bool) {
        let _ = self
            .tx
            .send(IPCCommand::BarToggleAll(exclude_hidden_by_default));
    }
    async fn bar_reveal_all(&self, exclude_hidden_by_default: bool) {
        let _ = self
            .tx
            .send(IPCCommand::BarRevealAll(exclude_hidden_by_default));
    }
    async fn bar_hide_all(&self, exclude_hidden_by_default: bool) {
        let _ = self
            .tx
            .send(IPCCommand::BarHideAll(exclude_hidden_by_default));
    }
}

async fn start_shell_service(tx: mpsc::UnboundedSender<IPCCommand>) -> zbus::Result<()> {
    let service = IPCService::new(tx);
    let _connection = connection::Builder::session()?
        .name("com.mshell.Shell")?
        .serve_at("/com/mshell/Shell", service)?
        .build()
        .await?;
    std::future::pending::<()>().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ClipboardAction, parse_clipboard_action, pick_device};

    #[test]
    fn clipboard_action_parses_known_verbs() {
        assert_eq!(
            parse_clipboard_action("copy 42"),
            Some(ClipboardAction::Copy(42))
        );
        assert_eq!(
            parse_clipboard_action("pin 7"),
            Some(ClipboardAction::TogglePin(7))
        );
        assert_eq!(
            parse_clipboard_action("unpin 7"),
            Some(ClipboardAction::TogglePin(7))
        );
        assert_eq!(
            parse_clipboard_action("delete 3"),
            Some(ClipboardAction::Delete(3))
        );
        // clear / wipe ignore any trailing id.
        assert_eq!(
            parse_clipboard_action("clear"),
            Some(ClipboardAction::Clear)
        );
        assert_eq!(
            parse_clipboard_action("wipe now"),
            Some(ClipboardAction::Wipe)
        );
        // Extra whitespace is tolerated.
        assert_eq!(
            parse_clipboard_action("  copy   9 "),
            Some(ClipboardAction::Copy(9))
        );
    }

    #[test]
    fn clipboard_action_rejects_unknown_or_malformed() {
        assert_eq!(parse_clipboard_action(""), None);
        assert_eq!(parse_clipboard_action("bogus 1"), None);
        // id-requiring verbs with a missing / non-numeric id → None.
        assert_eq!(parse_clipboard_action("copy"), None);
        assert_eq!(parse_clipboard_action("delete abc"), None);
    }

    fn names() -> Vec<(String, String)> {
        vec![
            ("alsa_output.hdmi".into(), "HDMI Audio".into()),
            (
                "alsa_output.pci".into(),
                "Logitech Z205 Analog Stereo".into(),
            ),
            ("bluez.headset".into(), "WH-1000XM4".into()),
        ]
    }

    #[test]
    fn empty_device_list_is_none() {
        assert_eq!(pick_device(&[], None, "next"), None);
    }

    #[test]
    fn next_and_prev_cycle_from_current() {
        let n = names();
        assert_eq!(pick_device(&n, Some("alsa_output.pci"), "next"), Some(2));
        assert_eq!(pick_device(&n, Some("bluez.headset"), "next"), Some(0)); // wraps
        assert_eq!(pick_device(&n, Some("alsa_output.hdmi"), "prev"), Some(2)); // wraps
        assert_eq!(pick_device(&n, Some("bluez.headset"), "previous"), Some(1));
    }

    #[test]
    fn next_without_current_starts_at_zero() {
        assert_eq!(pick_device(&names(), None, "next"), Some(0));
        assert_eq!(pick_device(&names(), Some("unknown"), "prev"), Some(0));
    }

    #[test]
    fn numeric_index_selects_directly_when_in_range() {
        assert_eq!(pick_device(&names(), None, "1"), Some(1));
        // Out of range falls through to the name/description match (no hit).
        assert_eq!(pick_device(&names(), None, "9"), None);
    }

    #[test]
    fn matches_by_name_or_description_case_insensitively() {
        assert_eq!(pick_device(&names(), None, "logitech"), Some(1));
        assert_eq!(pick_device(&names(), None, "HDMI"), Some(0));
        assert_eq!(pick_device(&names(), None, "bluez"), Some(2));
        assert_eq!(pick_device(&names(), None, "nonexistent"), None);
    }
}
