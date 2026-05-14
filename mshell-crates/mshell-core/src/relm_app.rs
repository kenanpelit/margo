use crate::ipc::init_ipc_shell_service;
use crate::monitors;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use mshell_cache::wallpaper::{CycleDirection, cycle_wallpaper};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarsStoreFields, ConfigStoreFields, FrameStoreFields, IdleStoreFields,
    WallpaperRotationMode, WallpaperStoreFields,
};
use mshell_idle::idle_manager::{self, IdleConfig, IdleStage};
use mshell_idle::inhibitor::IdleInhibitor;
use mshell_session::session_lock::session_lock;
use mshell_frame::frame::{Frame, FrameInit, FrameInput};
use mshell_lockscreen::lock_screen_manager::{LockScreenManagerInit, LockScreenManagerModel};
use mshell_notification_popups::popup_notifications::{
    PopupNotificationsInit, PopupNotificationsModel,
};
use mshell_services::notification_service;
use mshell_osd::brightness_osd::{BrightnessOsdInit, BrightnessOsdModel};
use mshell_osd::sound_alerts::SoundAlertsModel;
use mshell_osd::volume_osd::{VolumeOsdInit, VolumeOsdModel};
use mshell_polkit::PolkitPromptModel;
use mshell_style::style_manager::{StyleManagerModel, StyleManagerOutput};
use mshell_wallpaper::wallpaper::{WallpaperInit, WallpaperModel};
use reactive_graph::effect::Effect;
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::gdk::Monitor;
use relm4::{gtk::prelude::*, main_application, prelude::*};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use tracing::info;

pub(crate) struct WindowGroup {
    pub monitor: Monitor,
    pub frame: Option<Controller<Frame>>,
    pub _wallpaper: Option<Controller<WallpaperModel>>,
    pub _popup_notifications: Option<Controller<PopupNotificationsModel>>,
    pub _volume_osd: Option<Controller<VolumeOsdModel>>,
    pub _brightness_osd: Option<Controller<BrightnessOsdModel>>,
}

pub(crate) struct Shell {
    window_groups: HashMap<String, WindowGroup>,
    _lock_screen_manager: Controller<LockScreenManagerModel>,
    _polkit: Controller<PolkitPromptModel>,
    _sound_alerts: Controller<SoundAlertsModel>,
    _style_manager: Controller<StyleManagerModel>,
    monitor_filter: Vec<String>,
    /// Keeps the idle-manager timeout channel alive for the
    /// shell's lifetime.
    _idle_config_tx: tokio::sync::watch::Sender<IdleConfig>,
}

pub(crate) struct ShellInit {}

#[derive(Debug)]
pub(crate) enum ShellInput {
    SyncMonitors,
    MonitorFilterUpdated(Vec<String>),
    AddWindowGroup(String, Monitor),
    RemoveWindowGroup(String),
    Quit,
    ToggleQuickSettings(Option<String>),
    ToggleAppLauncher(Option<String>),
    ToggleClipboard(Option<String>),
    ToggleClockMenu(Option<String>),
    ToggleNotifications(Option<String>),
    NotificationsClearAll,
    NotificationsReadPopups,
    ToggleScreenshotMenu(Option<String>),
    ToggleWallpaperMenu(Option<String>),
    CycleWallpaper(mshell_cache::wallpaper::CycleDirection),
    ToggleNufwMenu(Option<String>),
    ToggleNdnsMenu(Option<String>),
    ToggleNpodmanMenu(Option<String>),
    ToggleNnotesMenu(Option<String>),
    ToggleNipMenu(Option<String>),
    ToggleNnetworkMenu(Option<String>),
    ToggleNpowerMenu(Option<String>),
    ToggleMediaPlayerMenu(Option<String>),
    ToggleSessionMenu(Option<String>),
    RunSessionAction(mshell_utils::session::SessionAction),
    CloseAllMenus,
    ToggleScreenshareMenu(Option<String>, tokio::sync::oneshot::Sender<String>, String),
    QueueFrameRedraw,
    BarToggleTop(Option<String>),
    BarToggleBottom(Option<String>),
    BarToggleLeft(Option<String>),
    BarToggleRight(Option<String>),
    BarToggleAll(Option<String>, bool),
    BarRevealAll(Option<String>, bool),
    BarHideAll(Option<String>, bool),
}

#[derive(Debug)]
pub enum ShellCommandOutput {}

#[relm4::component(pub(crate))]
impl Component for Shell {
    type CommandOutput = ShellCommandOutput;
    type Input = ShellInput;
    type Output = ();
    type Init = ShellInit;

    view! {
        #[root]
        gtk::Window {
            set_decorated: false,
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let icon_theme = gtk::IconTheme::for_display(&gtk::gdk::Display::default().unwrap());
        icon_theme.add_search_path(
            dirs::home_dir()
                .unwrap()
                .join(".config/margo/mshell/icons"),
        );

        root.init_layer_shell();
        root.set_layer(Layer::Background);
        root.set_default_size(1, 1);
        root.set_visible(false);
        root.set_namespace(Some("mshell-invisible-root"));

        let widgets = view_output!();

        let window_groups = HashMap::new();

        let lock_screen_manager = LockScreenManagerModel::builder()
            .launch(LockScreenManagerInit {})
            .detach();

        let polkit = PolkitPromptModel::builder().launch(()).detach();

        let sound_alerts = SoundAlertsModel::builder().launch(()).detach();

        let style_manager = StyleManagerModel::builder().launch(()).forward(
            sender.input_sender(),
            |msg| match msg {
                StyleManagerOutput::QueueFrameRedraw => ShellInput::QueueFrameRedraw,
            },
        );

        let sender_clone = sender.clone();
        Effect::new(move |_| {
            let monitors = config_manager()
                .config()
                .bars()
                .frame()
                .monitor_filter()
                .get();
            sender_clone.input(ShellInput::MonitorFilterUpdated(monitors));
        });

        let monitor_filter = config_manager()
            .config()
            .bars()
            .frame()
            .monitor_filter()
            .get_untracked();

        init_ipc_shell_service(&sender);
        spawn_wallpaper_rotation_timer();
        let idle_config_tx = spawn_idle_manager();

        let model = Shell {
            window_groups,
            _lock_screen_manager: lock_screen_manager,
            _polkit: polkit,
            _sound_alerts: sound_alerts,
            _style_manager: style_manager,
            monitor_filter,
            _idle_config_tx: idle_config_tx,
        };

        monitors::setup_monitor_watcher(&sender);
        monitors::sync_monitors(&model.window_groups, &sender);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ShellInput::SyncMonitors => {
                monitors::sync_monitors(&self.window_groups, &sender);
            }
            ShellInput::MonitorFilterUpdated(monitors) => {
                self.monitor_filter = monitors;

                for (name, group) in self.window_groups.iter_mut() {
                    let should_have_frame =
                        self.monitor_filter.is_empty() || self.monitor_filter.contains(name);

                    if should_have_frame && group.frame.is_none() {
                        group.frame = Some(
                            Frame::builder()
                                .launch(FrameInit {
                                    monitor: group.monitor.clone(),
                                })
                                .detach(),
                        );
                    } else if !should_have_frame && let Some(frame) = group.frame.take() {
                        frame.widget().close();
                    }
                }
            }
            ShellInput::AddWindowGroup(name, monitor) => {
                info!("Creating new window group");
                let wallpaper = Some(
                    WallpaperModel::builder()
                        .launch(WallpaperInit {
                            monitor: monitor.clone(),
                        })
                        .detach(),
                );

                let create_frame =
                    self.monitor_filter.is_empty() || self.monitor_filter.contains(&name);

                let frame = if create_frame {
                    Some(
                        Frame::builder()
                            .launch(FrameInit {
                                monitor: monitor.clone(),
                            })
                            .detach(),
                    )
                } else {
                    None
                };

                let popup_notifications = Some(
                    PopupNotificationsModel::builder()
                        .launch(PopupNotificationsInit {
                            monitor: monitor.clone(),
                        })
                        .detach(),
                );

                let volume_osd = Some(
                    VolumeOsdModel::builder()
                        .launch(VolumeOsdInit {
                            monitor: monitor.clone(),
                        })
                        .detach(),
                );

                let brightness_osd = Some(
                    BrightnessOsdModel::builder()
                        .launch(BrightnessOsdInit {
                            monitor: monitor.clone(),
                        })
                        .detach(),
                );

                let window_group = WindowGroup {
                    monitor: monitor.clone(),
                    frame,
                    _wallpaper: wallpaper,
                    _popup_notifications: popup_notifications,
                    _volume_osd: volume_osd,
                    _brightness_osd: brightness_osd,
                };

                self.window_groups.insert(name, window_group);
            }
            ShellInput::RemoveWindowGroup(name) => {
                if let Some(group) = self.window_groups.remove(&name) {
                    if let Some(frame) = &group.frame {
                        frame.widget().close();
                    }
                    if let Some(wallpaper) = &group._wallpaper {
                        wallpaper.widget().close();
                    }
                    if let Some(popup) = &group._popup_notifications {
                        popup.widget().close();
                    }
                    if let Some(vol) = &group._volume_osd {
                        vol.widget().close();
                    }
                    if let Some(bright) = &group._brightness_osd {
                        bright.widget().close();
                    }
                }
            }
            ShellInput::Quit => {
                main_application().quit();
            }
            ShellInput::ToggleAppLauncher(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleAppLauncherMenu);
                }
            }
            ShellInput::ToggleScreenshotMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleScreenshotMenu);
                }
            }
            ShellInput::ToggleNotifications(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleNotificationMenu);
                }
            }
            ShellInput::NotificationsClearAll => {
                // Same effect as the menu's "Clear All" — drops the
                // whole history (and with it any live popups).
                tokio::spawn(async move {
                    let _ = notification_service().dismiss_all().await;
                });
            }
            ShellInput::NotificationsReadPopups => {
                // Dismiss only the on-screen popups; they stay in the
                // notification history.
                let service = notification_service();
                for popup in service.popups.get() {
                    service.dismiss_popup(popup.id);
                }
            }
            ShellInput::ToggleClipboard(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleClipboardMenu);
                }
            }
            ShellInput::ToggleClockMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleClockMenu);
                }
            }
            ShellInput::ToggleQuickSettings(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleQuickSettingsMenu);
                }
            }
            ShellInput::ToggleWallpaperMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleWallpaperMenu);
                }
            }
            ShellInput::CycleWallpaper(direction) => {
                // Runs on the relm4 main thread — `set_wallpaper`
                // (called by `cycle_wallpaper`) kicks glib work.
                mshell_cache::wallpaper::cycle_wallpaper(direction);
            }
            ShellInput::ToggleNufwMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleNufwMenu);
                }
            }
            ShellInput::ToggleNdnsMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleNdnsMenu);
                }
            }
            ShellInput::ToggleNpodmanMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleNpodmanMenu);
                }
            }
            ShellInput::ToggleNnotesMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleNnotesMenu);
                }
            }
            ShellInput::ToggleNipMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleNipMenu);
                }
            }
            ShellInput::ToggleNnetworkMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleNnetworkMenu);
                }
            }
            ShellInput::ToggleNpowerMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleNpowerMenu);
                }
            }
            ShellInput::ToggleMediaPlayerMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleMediaPlayerMenu);
                }
            }
            ShellInput::ToggleSessionMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleSessionMenu);
                }
            }
            ShellInput::RunSessionAction(action) => {
                mshell_utils::session::run_session_action(action);
            }
            ShellInput::CloseAllMenus => {
                self.window_groups.iter().for_each(|(_, wg)| {
                    if let Some(frame) = &wg.frame {
                        frame.emit(FrameInput::CloseMenus);
                    }
                });
            }
            ShellInput::ToggleScreenshareMenu(monitor_name, reply, payload) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleScreenshareMenu(reply, payload));
                }
            }
            ShellInput::QueueFrameRedraw => {
                self.window_groups.iter().for_each(|(_, wg)| {
                    if let Some(frame) = &wg.frame {
                        frame.emit(FrameInput::QueueFrameRedraw);
                    }
                });
            }
            ShellInput::BarToggleTop(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::BarToggleTop);
                }
            }
            ShellInput::BarToggleBottom(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::BarToggleBottom);
                }
            }
            ShellInput::BarToggleLeft(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::BarToggleLeft);
                }
            }
            ShellInput::BarToggleRight(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::BarToggleRight);
                }
            }
            ShellInput::BarToggleAll(monitor_name, exclude_hidden_by_default) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::BarToggleAll(exclude_hidden_by_default));
                }
            }
            ShellInput::BarRevealAll(monitor_name, exclude_hidden_by_default) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::BarRevealAll(exclude_hidden_by_default));
                }
            }
            ShellInput::BarHideAll(monitor_name, exclude_hidden_by_default) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::BarHideAll(exclude_hidden_by_default));
                }
            }
        }
    }
}

/// Drive the automatic wallpaper rotation.
///
/// Runs on the GTK main loop (not a tokio task) so config reads
/// and `cycle_wallpaper` — which kicks glib work — happen on the
/// right thread. Wakes every 30 s and rotates once the configured
/// interval has elapsed; while rotation is disabled it keeps the
/// elapsed clock fresh so re-enabling doesn't fire immediately.
fn spawn_wallpaper_rotation_timer() {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    let last_rotation = Rc::new(Cell::new(Instant::now()));
    relm4::gtk::glib::timeout_add_seconds_local(30, move || {
        // The Store subfield accessors consume `self`, so re-walk
        // the config chain for each read.
        let enabled = config_manager()
            .config()
            .wallpaper()
            .rotation_enabled()
            .get_untracked();
        if enabled {
            let interval_min = config_manager()
                .config()
                .wallpaper()
                .rotation_interval_minutes()
                .get_untracked()
                .max(1);
            let due = last_rotation.get().elapsed()
                >= Duration::from_secs(u64::from(interval_min) * 60);
            if due {
                let direction = match config_manager()
                    .config()
                    .wallpaper()
                    .rotation_mode()
                    .get_untracked()
                {
                    WallpaperRotationMode::Random => CycleDirection::Random,
                    WallpaperRotationMode::Sequential => CycleDirection::Next,
                };
                cycle_wallpaper(direction);
                last_rotation.set(Instant::now());
            }
        } else {
            last_rotation.set(Instant::now());
        }
        relm4::gtk::glib::ControlFlow::Continue
    });
}

/// Read the idle-stage timeouts from config into an `IdleConfig`
/// (a disabled stage maps to `None`).
fn read_idle_config() -> IdleConfig {
    let dim_minutes = config_manager()
        .config()
        .idle()
        .dim_enabled()
        .get()
        .then(|| config_manager().config().idle().dim_timeout_minutes().get());
    let lock_minutes = config_manager()
        .config()
        .idle()
        .lock_enabled()
        .get()
        .then(|| config_manager().config().idle().lock_timeout_minutes().get());
    let suspend_minutes = config_manager()
        .config()
        .idle()
        .suspend_enabled()
        .get()
        .then(|| {
            config_manager()
                .config()
                .idle()
                .suspend_timeout_minutes()
                .get()
        });
    IdleConfig {
        dim_minutes,
        lock_minutes,
        suspend_minutes,
    }
}

/// A full-screen, input-transparent translucent overlay shown
/// while the session is idle-dimmed.
fn build_idle_dim_overlay() -> gtk::Window {
    let window = gtk::Window::new();
    window.add_css_class("idle-dim-overlay");
    window.set_decorated(false);
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("mshell-idle-dim"));
    // Visual only — no exclusive zone, no keyboard focus, so input
    // still reaches the windows below and wakes the session.
    window.set_exclusive_zone(-1);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.set_visible(false);
    window
}

/// Start the idle manager: wire config → timeouts, and the idle
/// stage → screen-dim overlay / lock / suspend. Returns the
/// config sender, which the caller keeps alive.
fn spawn_idle_manager() -> tokio::sync::watch::Sender<IdleConfig> {
    let (config_tx, mut stage_rx) = match idle_manager::start(read_idle_config()) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!("idle manager unavailable: {e:#}");
            // Hand back a live-but-unused sender so the caller's
            // struct field stays valid.
            return tokio::sync::watch::channel(IdleConfig::default()).0;
        }
    };

    // Push timeout changes from config into the manager.
    let config_tx_effect = config_tx.clone();
    Effect::new(move |_| {
        let _ = config_tx_effect.send(read_idle_config());
    });

    // React to idle-stage changes on the GTK main loop.
    let dim_overlay = build_idle_dim_overlay();
    relm4::gtk::glib::spawn_future_local(async move {
        let mut previous = IdleStage::Active;
        while stage_rx.changed().await.is_ok() {
            let stage = *stage_rx.borrow_and_update();
            if stage == previous {
                continue;
            }
            previous = stage;

            // The manual idle inhibitor vetoes everything but the
            // return-to-active (un-dim) transition.
            let inhibited = IdleInhibitor::global().get();

            match stage {
                IdleStage::Active => {
                    dim_overlay.set_visible(false);
                }
                IdleStage::Dim => {
                    if !inhibited {
                        dim_overlay.set_visible(true);
                    }
                }
                IdleStage::Lock => {
                    if !inhibited {
                        dim_overlay.set_visible(true);
                        if !session_lock().is_locked() {
                            session_lock().lock();
                        }
                    }
                }
                IdleStage::Suspend => {
                    if !inhibited {
                        if !session_lock().is_locked() {
                            session_lock().lock();
                        }
                        if let Err(e) =
                            std::process::Command::new("systemctl").arg("suspend").spawn()
                        {
                            tracing::warn!("idle suspend failed: {e}");
                        }
                    }
                }
            }
        }
    });

    config_tx
}

/// Get the frame for the monitor name.  If it doesn't exist, get the first frame available
fn resolve_frame<'a>(
    window_groups: &'a HashMap<String, WindowGroup>,
    monitor_name: &Option<String>,
) -> Option<&'a Controller<Frame>> {
    if let Some(name) = monitor_name
        && let Some(frame) = window_groups.get(name).and_then(|g| g.frame.as_ref())
    {
        return Some(frame);
    }
    window_groups.values().find_map(|g| g.frame.as_ref())
}

impl Debug for WindowGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowGroup").finish()
    }
}
