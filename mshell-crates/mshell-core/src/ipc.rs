use crate::relm_app::{Shell, ShellInput};
use mshell_cache::wallpaper::set_wallpaper;
use mshell_services::{audio_service, brightness_service, margo_service, notification_service};
use mshell_session::session_lock::{lock_session, session_locked};
use mshell_settings::{close_settings, open_settings, open_wizard};
use mshell_utils::session::SessionAction;
use mshell_sounds::play_audio_volume_change;
use relm4::gtk::glib;
use relm4::{ComponentSender, gtk};
use std::path::PathBuf;
use tokio::sync::mpsc;
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::volume::types::Volume;
use wayle_brightness::Percentage;
use zbus::connection;
use zbus::interface;

pub fn init_ipc_shell_service(sender: &ComponentSender<Shell>) {
    let (shell_tx, mut shell_rx) = mpsc::unbounded_channel();

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
                IPCCommand::Wallpaper => {
                    app_sender.emit(ShellInput::ToggleWallpaperMenu(active_monitor().await));
                }
                IPCCommand::WallpaperCycle(direction) => {
                    app_sender.emit(ShellInput::CycleWallpaper(direction));
                }
                IPCCommand::Ufw => {
                    app_sender.emit(ShellInput::ToggleUfwMenu(active_monitor().await));
                }
                IPCCommand::Bluetooth => {
                    app_sender.emit(ShellInput::ToggleBluetoothMenu(active_monitor().await));
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
                IPCCommand::MShellDash(tab) => {
                    app_sender
                        .emit(ShellInput::ToggleMShellDashMenu(active_monitor().await, tab));
                }
                IPCCommand::SessionAction(action) => {
                    app_sender.emit(ShellInput::RunSessionAction(action));
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
                    let names: Vec<(String, String)> =
                        devs.iter().map(|d| (d.name.get(), d.description.get())).collect();
                    let cur = audio_service().default_output.get().map(|d| d.name.get());
                    if let Some(i) = pick_device(&names, cur.as_deref(), &target)
                        && devs[i].set_as_default().await.is_ok()
                    {
                        notify_audio("Audio output", &devs[i].description.get());
                    }
                }
                IPCCommand::SwitchInput(target) => {
                    let devs = usable_inputs();
                    let names: Vec<(String, String)> =
                        devs.iter().map(|d| (d.name.get(), d.description.get())).collect();
                    let cur = audio_service().default_input.get().map(|d| d.name.get());
                    if let Some(i) = pick_device(&names, cur.as_deref(), &target)
                        && devs[i].set_as_default().await.is_ok()
                    {
                        notify_audio("Audio input", &devs[i].description.get());
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
    Wallpaper,
    Ufw,
    Bluetooth,
    CpuDashboard,
    AudioDashboard,
    SystemUpdate,
    Valent,
    KeepAwake,
    Twilight,
    Weather,
    Keybinds,
    SshSessions,
    Dns,
    Podman,
    Notes,
    Ip,
    Network,
    Power,
    MediaPlayer,
    Dashboard,
    MShellDash(String),
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
fn usable_outputs() -> Vec<Arc<OutputDevice>> {
    let mut v: Vec<_> =
        audio_service().output_devices.get().into_iter().filter(|d| output_connected(d)).collect();
    v.sort_by_key(|d| d.key.index);
    v
}

/// Real capture sources — drops PulseAudio monitor sources (the loopback
/// "Monitor of <sink>" entries), which aren't microphones. Same stable sort.
fn usable_inputs() -> Vec<Arc<InputDevice>> {
    let mut v: Vec<_> =
        audio_service().input_devices.get().into_iter().filter(|d| !d.is_monitor.get()).collect();
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

/// Resolve a switch target against a device list. `names` is `(node_name,
/// description)` per device, `current` the default's node name. Accepts
/// `next` / `prev` / `switch`, a numeric index, or a case-insensitive
/// fragment matched against the description first then the node name.
fn pick_device(names: &[(String, String)], current: Option<&str>, target: &str) -> Option<usize> {
    if names.is_empty() {
        return None;
    }
    let cur = current.and_then(|c| names.iter().position(|(n, _)| n == c));
    let t = target.trim();
    match t.to_ascii_lowercase().as_str() {
        "next" | "switch" => return Some(cur.map(|c| (c + 1) % names.len()).unwrap_or(0)),
        "prev" | "previous" => {
            return Some(cur.map(|c| (c + names.len() - 1) % names.len()).unwrap_or(0));
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
    let snap = |i: usize, name: String, description: String, vol: f64, muted: bool, def: &Option<String>| {
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
        .map(|(i, d)| snap(i, d.name.get(), d.description.get(), d.volume.get().average(), d.muted.get(), &def_out))
        .collect();
    let inputs = usable_inputs()
        .iter()
        .enumerate()
        .map(|(i, d)| snap(i, d.name.get(), d.description.get(), d.volume.get().average(), d.muted.get(), &def_in))
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
    format!("  {}: {} {:<42} {:>3}%{}", d.index, icon, d.description, d.volume_percent, tags)
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
    format!("{}\n{}", line(out.as_ref(), "Output"), line(inp.as_ref(), "Input"))
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
    async fn wallpaper(&self) {
        let _ = self.tx.send(IPCCommand::Wallpaper);
    }
    async fn ufw(&self) {
        let _ = self.tx.send(IPCCommand::Ufw);
    }
    async fn bluetooth(&self) {
        let _ = self.tx.send(IPCCommand::Bluetooth);
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
    async fn mshelldash(&self, tab: String) {
        let _ = self.tx.send(IPCCommand::MShellDash(tab));
    }
    async fn session_lock(&self) {
        let _ = self.tx.send(IPCCommand::SessionAction(SessionAction::Lock));
    }
    async fn session_logout(&self) {
        let _ = self.tx.send(IPCCommand::SessionAction(SessionAction::Logout));
    }
    async fn session_suspend(&self) {
        let _ = self.tx.send(IPCCommand::SessionAction(SessionAction::Suspend));
    }
    async fn session_reboot(&self) {
        let _ = self.tx.send(IPCCommand::SessionAction(SessionAction::Reboot));
    }
    async fn session_shutdown(&self) {
        let _ = self.tx.send(IPCCommand::SessionAction(SessionAction::Shutdown));
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
