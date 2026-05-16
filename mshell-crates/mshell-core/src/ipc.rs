use crate::relm_app::{Shell, ShellInput};
use mshell_cache::wallpaper::set_wallpaper;
use mshell_services::{audio_service, brightness_service, margo_service};
use mshell_session::session_lock::session_lock;
use mshell_settings::{close_settings, open_settings};
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
                IPCCommand::QuickSettings => {
                    app_sender.emit(ShellInput::ToggleQuickSettings(active_monitor().await));
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
                IPCCommand::Nufw => {
                    app_sender.emit(ShellInput::ToggleNufwMenu(active_monitor().await));
                }
                IPCCommand::Ndns => {
                    app_sender.emit(ShellInput::ToggleNdnsMenu(active_monitor().await));
                }
                IPCCommand::Npodman => {
                    app_sender.emit(ShellInput::ToggleNpodmanMenu(active_monitor().await));
                }
                IPCCommand::Nnotes => {
                    app_sender.emit(ShellInput::ToggleNnotesMenu(active_monitor().await));
                }
                IPCCommand::Nip => {
                    app_sender.emit(ShellInput::ToggleNipMenu(active_monitor().await));
                }
                IPCCommand::Nnetwork => {
                    app_sender.emit(ShellInput::ToggleNnetworkMenu(active_monitor().await));
                }
                IPCCommand::Npower => {
                    app_sender.emit(ShellInput::ToggleNpowerMenu(active_monitor().await));
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
                    session_lock().lock();
                }
                IPCCommand::CheckLock(reply) => {
                    let _ = reply.send(session_lock().is_locked());
                }
                IPCCommand::Screenshare(reply, payload) => {
                    app_sender.emit(ShellInput::ToggleScreenshareMenu(
                        active_monitor().await,
                        reply,
                        payload,
                    ));
                }
                IPCCommand::OpenSettings => {
                    open_settings();
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

enum IPCCommand {
    Quit,
    QuickSettings,
    AppLauncher,
    Clock,
    Clipboard,
    Notifications,
    NotificationsClearAll,
    NotificationsReadPopups,
    Session,
    SessionAction(SessionAction),
    Screenshot,
    Wallpaper,
    Nufw,
    Ndns,
    Npodman,
    Nnotes,
    Nip,
    Nnetwork,
    Npower,
    MediaPlayer,
    Dashboard,
    CloseAllMenus,
    VolumeUp,
    VolumeDown,
    Mute,
    BrightnessUp,
    BrightnessDown,
    SetWallpaper(PathBuf),
    WallpaperCycle(mshell_cache::wallpaper::CycleDirection),
    Lock,
    CheckLock(tokio::sync::oneshot::Sender<bool>),
    Screenshare(tokio::sync::oneshot::Sender<String>, String),
    OpenSettings,
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
    async fn quick_settings(&self) {
        let _ = self.tx.send(IPCCommand::QuickSettings);
    }
    async fn app_launcher(&self) {
        let _ = self.tx.send(IPCCommand::AppLauncher);
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
    async fn screenshot(&self) {
        let _ = self.tx.send(IPCCommand::Screenshot);
    }
    async fn wallpaper(&self) {
        let _ = self.tx.send(IPCCommand::Wallpaper);
    }
    async fn nufw(&self) {
        let _ = self.tx.send(IPCCommand::Nufw);
    }
    async fn ndns(&self) {
        let _ = self.tx.send(IPCCommand::Ndns);
    }
    async fn npodman(&self) {
        let _ = self.tx.send(IPCCommand::Npodman);
    }
    async fn nnotes(&self) {
        let _ = self.tx.send(IPCCommand::Nnotes);
    }
    async fn nip(&self) {
        let _ = self.tx.send(IPCCommand::Nip);
    }
    async fn nnetwork(&self) {
        let _ = self.tx.send(IPCCommand::Nnetwork);
    }
    async fn npower(&self) {
        let _ = self.tx.send(IPCCommand::Npower);
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
    async fn open_settings(&self) {
        let _ = self.tx.send(IPCCommand::OpenSettings);
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
