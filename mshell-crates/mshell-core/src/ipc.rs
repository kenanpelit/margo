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
