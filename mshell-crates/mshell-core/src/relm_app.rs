use crate::ipc::init_ipc_shell_service;
use crate::monitors;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use mshell_cache::wallpaper::{CycleDirection, cycle_wallpaper};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarsStoreFields, ConfigStoreFields, DockStoreFields, FrameStoreFields, GeneralStoreFields,
    IdleStoreFields, WallpaperRotationMode, WallpaperStoreFields,
};
use mshell_frame::frame::{Frame, FrameInit, FrameInput};
use mshell_frame::mdock_surface::{MdockSurface, MdockSurfaceInit, MdockSurfaceInput};
use mshell_idle::idle_manager::{self, IdleConfig, IdleStage};
use mshell_idle::inhibitor::IdleInhibitor;
use mshell_lockscreen::lock_screen_manager::{LockScreenManagerInit, LockScreenManagerModel};
use mshell_notification_popups::popup_notifications::{
    PopupNotificationsInit, PopupNotificationsModel,
};
use mshell_osd::brightness_osd::{BrightnessOsdInit, BrightnessOsdModel};
use mshell_osd::mic_osd::{MicOsdInit, MicOsdModel};
use mshell_osd::network_osd::{NetworkOsdInit, NetworkOsdModel};
use mshell_osd::sound_alerts::SoundAlertsModel;
use mshell_osd::volume_osd::{VolumeOsdInit, VolumeOsdModel};
use mshell_polkit::PolkitPromptModel;
use mshell_services::margo_service;
use mshell_services::notification_service;
use mshell_session::session_lock::{lock_session, session_locked};
use mshell_style::style_manager::{StyleManagerModel, StyleManagerOutput};
use mshell_wallpaper::wallpaper::{WallpaperInit, WallpaperModel};
use reactive_graph::effect::Effect;
use reactive_graph::prelude::ReadUntracked;
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::gdk::Monitor;
use relm4::{gtk::prelude::*, main_application, prelude::*};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use tracing::info;

pub(crate) struct WindowGroup {
    pub monitor: Monitor,
    pub frame: Option<Controller<Frame>>,
    /// Standalone mdock surface for this output (when `dock.standalone`).
    pub mdock: Option<Controller<MdockSurface>>,
    pub _wallpaper: Option<Controller<WallpaperModel>>,
    pub _popup_notifications: Option<Controller<PopupNotificationsModel>>,
    pub _volume_osd: Option<Controller<VolumeOsdModel>>,
    pub _mic_osd: Option<Controller<MicOsdModel>>,
    pub _brightness_osd: Option<Controller<BrightnessOsdModel>>,
    /// Network-state OSD — flashes on connect / disconnect.
    /// Gated by `general.network_osd_enabled`; the controller
    /// itself stays mounted in either case (cheap, idle) and
    /// just doesn't paint when the flag is off.
    pub _network_osd: Option<Controller<NetworkOsdModel>>,
    /// Per-monitor corner-overlay windows (`Vec` of four). Held
    /// for lifetime: dropping the `WindowGroup` closes them on
    /// monitor hot-unplug. Empty when `general.show_screen_corners`
    /// is off.
    pub _screen_corners: Vec<gtk::Window>,
}

/// Build the standalone mdock surface for an output, or `None` when
/// `dock.standalone` is off. Used at window-group creation and on a live
/// `dock.standalone/behavior/position` change (rebuild).
fn make_mdock(monitor: &Monitor) -> Option<Controller<MdockSurface>> {
    let dock = config_manager().config().dock().get_untracked();
    // The standalone surface is the *Popup* (independent) style only. In
    // LayerShell style the dock is a bar-attached Frame menu instead.
    if !dock.standalone || !matches!(dock.style, mshell_config::schema::config::DockStyle::Popup) {
        return None;
    }
    Some(
        MdockSurface::builder()
            .launch(MdockSurfaceInit {
                monitor: Some(monitor.clone()),
            })
            .detach(),
    )
}

/// Is the standalone dock the bar-attached (LayerShell) Frame menu?
fn dock_is_layer_shell() -> bool {
    let dock = config_manager().config().dock().get_untracked();
    dock.standalone
        && matches!(
            dock.style,
            mshell_config::schema::config::DockStyle::LayerShell
        )
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
    /// `dock.standalone/behavior/position` changed — tear down + recreate
    /// every output's standalone mdock surface.
    RebuildDocks,
    /// Standalone mdock: toggle / show / hide on every output.
    DockToggle,
    DockShow,
    DockHide,
    /// Activate the Nth (1-based) pinned dock app — focus its first window if
    /// running, else launch it. Bound to a hotkey (e.g. Super+Alt+N).
    DockActivate(u32),
    Quit,
    ToggleAppLauncher(Option<String>),
    /// Open the app launcher AND pre-select the named category
    /// tab. The String carries the tab label (e.g. "Actions",
    /// "Insert"). Unknown labels silently fall back to "All"
    /// once the launcher's `select_category` runs.
    ToggleAppLauncherWithTab(Option<String>, String),
    ToggleClipboard(Option<String>),
    ToggleClockMenu(Option<String>),
    /// Hidden Bar IPC verb — broadcast to every monitor's bars.
    HiddenBar(mshell_common::hidden_bar::HiddenBarVerb, Option<String>),
    ToggleNotifications(Option<String>),
    NotificationsClearAll,
    NotificationsReadPopups,
    ToggleScreenshotMenu(Option<String>),
    /// Headless screenshot capture from `mshellctl screenshot <area>`.
    /// Spec is `"<area> <target> <delay_secs>"` — area ∈ region/window/
    /// output/full, target ∈ default/copy/save/edit.
    CaptureScreenshot(String),
    /// Headless screen recording from `mshellctl screenrecord …`.
    /// Spec is `"<action> <area> <audio>"` — action ∈ start/stop/toggle,
    /// area ∈ region/window/output/full, audio = "-" (none) or a source.
    ScreenRecord(String),
    ToggleWallpaperMenu(Option<String>),
    CycleWallpaper(mshell_cache::wallpaper::CycleDirection),
    ToggleUfwMenu(Option<String>),
    TogglePrivacyMenu(Option<String>),
    ToggleBluetoothMenu(Option<String>),
    ToggleCpuDashboardMenu(Option<String>),
    ToggleAudioDashboardMenu(Option<String>),
    ToggleSystemUpdateMenu(Option<String>),
    ToggleValentMenu(Option<String>),
    ToggleKeepAwakeMenu(Option<String>),
    ToggleTwilightMenu(Option<String>),
    ToggleMargoLayoutMenu(Option<String>),
    ToggleWeatherMenu(Option<String>),
    ToggleKeybindsMenu(Option<String>),
    ToggleAlarmClockMenu(Option<String>),
    ToggleControlCenterMenu(Option<String>),
    ToggleSshSessionsMenu(Option<String>),
    ToggleVpnMenu(Option<String>),
    ToggleAiMenu(Option<String>),
    ToggleDnsMenu(Option<String>),
    TogglePodmanMenu(Option<String>),
    ToggleNotesMenu(Option<String>),
    /// Toggle an installed plugin's panel/menu by key (monitor, key). Generic
    /// — the frame resolves the key to the plugin's derived widget.
    TogglePluginMenu(Option<String>, String),
    /// Force-reload an installed plugin's WASM panel — evict the cached
    /// instance everywhere so the next open re-instantiates from disk.
    ReloadPlugin(String),
    /// `(monitor, plugin-key, bind-id)`: a global keybind fired; open the
    /// plugin's panel and deliver a `Keybind` event with the bind id.
    FirePluginKeybind(Option<String>, String, String),
    ToggleIpMenu(Option<String>),
    ToggleNetworkMenu(Option<String>),
    TogglePowerMenu(Option<String>),
    ToggleMediaPlayerMenu(Option<String>),
    ToggleSessionMenu(Option<String>),
    ToggleSettingsMenu(Option<String>),
    /// Toggle the in-shell setup wizard menu on the active monitor.
    ToggleWizardMenu(Option<String>),
    /// Jump to a specific Settings sidebar section. Routes
    /// through the per-frame settings widget after ensuring it's
    /// visible — same monitor-resolution dance as
    /// `ToggleSettingsMenu`. Emitted by the launcher's Settings
    /// provider through the `SECTION_BACKEND` bridge.
    OpenSettingsAtSection(Option<String>, String),
    ToggleMdashMenu(Option<String>),
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
        icon_theme.add_search_path(dirs::home_dir().unwrap().join(".config/margo/mshell/icons"));

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

        // Rebuild the standalone mdock surfaces when their shape changes.
        let sender_clone = sender.clone();
        Effect::new(move |_| {
            let _ = config_manager().config().dock().standalone().get();
            let _ = config_manager().config().dock().style().get();
            let _ = config_manager().config().dock().behavior().get();
            let _ = config_manager().config().dock().position().get();
            sender_clone.input(ShellInput::RebuildDocks);
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

        // Wire mshell-settings' `open_settings()` so it routes
        // through the shell-level dispatcher instead of pinning
        // to whichever Frame won the `OnceLock` race at boot.
        // Settings used to always open on the first monitor
        // (eDP-1) on multi-output setups because every Frame
        // would call `set_toggle_backend`, but `OnceLock` keeps
        // only the first registration.
        //
        // Threading: ShellInput contains `Monitor` (a gtk-rs
        // raw-ptr wrapper) so its Sender is `!Send`. The
        // `set_toggle_backend` closure must be `Send + Sync`
        // though, since the caller (gtk click handler) can be on
        // any glib context. We use a unit-typed tokio channel as
        // a thread-safe bridge: the closure fires a `()` ping,
        // and a glib-main-thread task drains the pings, queries
        // active monitor via `margo_service`, and emits
        // `ShellInput::ToggleSettingsMenu(Some(monitor))` to the
        // shell.
        let (toggle_tx, mut toggle_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        mshell_settings::set_toggle_backend(move || {
            let _ = toggle_tx.send(());
        });

        // Section-navigation bridge — mirrors the toggle bridge
        // above. Launcher → `mshell_settings::open_settings_at_section`
        // → registered closure → channel → main-loop task →
        // ShellInput::OpenSettingsAtSection(monitor, section).
        let (section_tx, mut section_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        mshell_settings::set_section_backend(move |section: &str| {
            let _ = section_tx.send(section.to_string());
        });
        let section_app_sender = sender.input_sender().clone();
        relm4::gtk::glib::spawn_future_local(async move {
            while let Some(section) = section_rx.recv().await {
                let monitor = margo_service().active_monitor_name().await;
                section_app_sender.emit(ShellInput::OpenSettingsAtSection(monitor, section));
            }
        });

        let toggle_app_sender = sender.input_sender().clone();
        relm4::gtk::glib::spawn_future_local(async move {
            while toggle_rx.recv().await.is_some() {
                // Use `active_monitor_name()` (focused-client first,
                // state.active_output fallback) — same path the rest
                // of the IPC arms switched to in 60078ec. The old
                // `active_workspace().monitor` returns None until the
                // workspaces cache populates after boot, which sent
                // Settings to whichever Frame iterated first
                // (typically eDP-1). The focused-client signal is
                // live the moment a window has focus, so this
                // resolves immediately even on a fresh session.
                let monitor = margo_service().active_monitor_name().await;
                toggle_app_sender.emit(ShellInput::ToggleSettingsMenu(monitor));
            }
        });

        // Wizard-menu toggle — same !Send bridge as the settings toggle.
        let (wizard_tx, mut wizard_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        mshell_settings::set_wizard_backend(move || {
            let _ = wizard_tx.send(());
        });
        let wizard_app_sender = sender.input_sender().clone();
        relm4::gtk::glib::spawn_future_local(async move {
            while wizard_rx.recv().await.is_some() {
                let monitor = margo_service().active_monitor_name().await;
                wizard_app_sender.emit(ShellInput::ToggleWizardMenu(monitor));
            }
        });

        // Lock-before-sleep: hold a logind delay inhibitor and lock with
        // mlock on any suspend/hibernate (lid, power menu, `systemctl
        // suspend`, logind idle) before the system actually sleeps.
        crate::sleep_lock::spawn();

        // Keep the lock-screen info sidecar (notifications / weather /
        // now-playing) fresh for mlock.
        crate::lock_info::start();

        // First launch — no shell profile saved yet. Open the setup
        // wizard menu once the frames exist (a short delay lets
        // `sync_monitors` build them first). This is the same in-shell
        // layer-shell menu as `mshellctl wizard`; the compositor no
        // longer launches any floating wizard window.
        if mshell_config::config_utils::list_available_profiles().is_empty()
            && !mshell_config::config_utils::wizard_completed()
        {
            relm4::gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(2), || {
                mshell_settings::open_wizard();
            });
        }

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
            ShellInput::RebuildDocks => {
                for group in self.window_groups.values_mut() {
                    if let Some(old) = group.mdock.take() {
                        old.widget().close();
                    }
                    group.mdock = make_mdock(&group.monitor);
                }
            }
            ShellInput::DockToggle | ShellInput::DockShow | ShellInput::DockHide => {
                // LayerShell style = a bar-attached Frame menu (toggle-only, so
                // show/hide also toggle). Popup style = the independent surface,
                // which honours show/hide/toggle distinctly.
                let layer_shell = dock_is_layer_shell();
                for group in self.window_groups.values() {
                    if layer_shell {
                        if let Some(f) = &group.frame {
                            f.emit(FrameInput::ToggleDockMenu);
                        }
                    } else if let Some(m) = &group.mdock {
                        m.emit(match message {
                            ShellInput::DockShow => MdockSurfaceInput::Show,
                            ShellInput::DockHide => MdockSurfaceInput::Hide,
                            _ => MdockSurfaceInput::Toggle,
                        });
                    }
                }
            }
            ShellInput::DockActivate(n) => {
                // Nth (1-based) pinned app: focus its first running window, else
                // launch it. Runs once (single app component) — no per-output
                // broadcast, so launch can't fire N times.
                let apps = mshell_cache::pinned_apps::pinned_apps_store()
                    .read_untracked()
                    .apps
                    .clone();
                if let Some(app) = apps.get((n.max(1) - 1) as usize) {
                    let class = app.app_id.clone();
                    let mut matching: Vec<_> = margo_service()
                        .clients
                        .get()
                        .into_iter()
                        .filter(|c| c.class.get() == class)
                        .collect();
                    matching.sort_by_key(|c| c.address.get());
                    if let Some(idx) = matching.first().and_then(|c| c.address.get().margo_idx()) {
                        mshell_services::tokio_rt().spawn(async move {
                            let _ = margo_service()
                                .dispatch(&format!("dispatch focuswindow {idx}"))
                                .await;
                        });
                    } else if let Some(info) = mshell_utils::app_info::find_app_info(&class) {
                        mshell_utils::launch::launch_detached(&info);
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

                let mic_osd = Some(
                    MicOsdModel::builder()
                        .launch(MicOsdInit {
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

                let network_osd = Some(
                    NetworkOsdModel::builder()
                        .launch(NetworkOsdInit {
                            monitor: monitor.clone(),
                        })
                        .detach(),
                );

                // Rounded screen corners — one tiny overlay per
                // corner. Reads config once at monitor-add time;
                // live toggling needs a reload (or a future
                // reactive effect plumbing). Empty `Vec` when
                // the user has turned the corners off.
                let show_corners = config_manager()
                    .config()
                    .general()
                    .show_screen_corners()
                    .get_untracked();
                let corner_radius = config_manager()
                    .config()
                    .general()
                    .screen_corner_radius()
                    .get_untracked();
                let screen_corners = if show_corners {
                    mshell_frame::screen_corners::spawn(&monitor, corner_radius)
                } else {
                    Vec::new()
                };

                let mdock = make_mdock(&monitor);

                let window_group = WindowGroup {
                    monitor: monitor.clone(),
                    frame,
                    mdock,
                    _wallpaper: wallpaper,
                    _popup_notifications: popup_notifications,
                    _volume_osd: volume_osd,
                    _mic_osd: mic_osd,
                    _brightness_osd: brightness_osd,
                    _network_osd: network_osd,
                    _screen_corners: screen_corners,
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
                    if let Some(mic) = &group._mic_osd {
                        mic.widget().close();
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
            ShellInput::ToggleAppLauncherWithTab(monitor_name, tab) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleAppLauncherMenuWithTab(tab));
                }
            }
            ShellInput::ToggleScreenshotMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleScreenshotMenu);
                }
            }
            ShellInput::CaptureScreenshot(spec) => {
                use mshell_screenshot::{CaptureArea, OutputTarget};
                let mut parts = spec.split_whitespace();
                let area = match parts.next() {
                    Some("window") => CaptureArea::SelectWindow,
                    Some("output") => CaptureArea::SelectMonitor,
                    Some("full") => CaptureArea::All,
                    _ => CaptureArea::SelectRegion, // "region" / default
                };
                let target_tok = parts.next();
                let delay = parts
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(std::time::Duration::from_secs)
                    .unwrap_or_default();
                // Optional 4th field: explicit editor name (or "-").
                // Naming an editor implies edit mode even without the
                // "edit" target token (`mshellctl screenshot region satty`).
                let editor = parts
                    .next()
                    .filter(|s| *s != "-" && !s.is_empty())
                    .map(str::to_string);
                let target = match target_tok {
                    Some("copy") => OutputTarget::Clipboard,
                    Some("save") => OutputTarget::File,
                    Some("edit") => OutputTarget::EditAndSave(editor),
                    _ if editor.is_some() => OutputTarget::EditAndSave(editor),
                    _ => OutputTarget::FileAndClipboard, // "default"
                };
                mshell_frame::capture_screenshot(area, target, delay);
            }
            ShellInput::ScreenRecord(spec) => {
                use mshell_screenshot::CaptureArea;
                let mut parts = spec.split_whitespace();
                let action = parts.next().unwrap_or("toggle").to_string();
                let area = match parts.next() {
                    Some("window") => CaptureArea::SelectWindow,
                    Some("output") => CaptureArea::SelectMonitor,
                    Some("region") => CaptureArea::SelectRegion,
                    _ => CaptureArea::All, // "full" / default
                };
                let audio = match parts.next() {
                    Some(a) if a != "-" && !a.is_empty() => Some(a.to_string()),
                    _ => None,
                };
                mshell_frame::screen_record(&action, area, audio);
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
            ShellInput::HiddenBar(verb, target) => {
                // Global — every monitor's bars react together.
                for group in self.window_groups.values() {
                    if let Some(frame) = group.frame.as_ref() {
                        frame.emit(FrameInput::HiddenBar(verb, target.clone()));
                    }
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
            ShellInput::ToggleUfwMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleUfwMenu);
                }
            }
            ShellInput::TogglePrivacyMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::TogglePrivacyMenu);
                }
            }
            ShellInput::ToggleBluetoothMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleBluetoothMenu);
                }
            }
            ShellInput::ToggleCpuDashboardMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleCpuDashboardMenu);
                }
            }
            ShellInput::ToggleAudioDashboardMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleAudioDashboardMenu);
                }
            }
            ShellInput::ToggleSystemUpdateMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleSystemUpdateMenu);
                }
            }
            ShellInput::ToggleValentMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleValentMenu);
                }
            }
            ShellInput::ToggleKeepAwakeMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleKeepAwakeMenu);
                }
            }
            ShellInput::ToggleTwilightMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleTwilightMenu);
                }
            }
            ShellInput::ToggleMargoLayoutMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleMargoLayoutMenu);
                }
            }
            ShellInput::ToggleWeatherMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleWeatherMenu);
                }
            }
            ShellInput::ToggleKeybindsMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleKeybindsMenu);
                }
            }
            ShellInput::ToggleAlarmClockMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleAlarmClockMenu);
                }
            }
            ShellInput::ToggleControlCenterMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleControlCenterMenu);
                }
            }
            ShellInput::ToggleSshSessionsMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleSshSessionsMenu);
                }
            }
            ShellInput::ToggleVpnMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleVpnMenu);
                }
            }
            ShellInput::ToggleAiMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleAiMenu);
                }
            }
            ShellInput::ToggleDnsMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleDnsMenu);
                }
            }
            ShellInput::TogglePodmanMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::TogglePodmanMenu);
                }
            }
            ShellInput::ToggleNotesMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleNotesMenu);
                }
            }
            ShellInput::TogglePluginMenu(monitor_name, key) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::TogglePluginByKey(key));
                }
            }
            ShellInput::ReloadPlugin(key) => {
                // Each frame caches its own PluginPanel — broadcast so every
                // monitor's panel drops the stale instance.
                for group in self.window_groups.values() {
                    if let Some(frame) = &group.frame {
                        frame.emit(FrameInput::ReloadPlugin(key.clone()));
                    }
                }
            }
            ShellInput::FirePluginKeybind(monitor_name, key, id) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::FirePluginKeybind(key, id));
                }
            }
            ShellInput::ToggleIpMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleIpMenu);
                }
            }
            ShellInput::ToggleNetworkMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleNetworkMenu);
                }
            }
            ShellInput::TogglePowerMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::TogglePowerMenu);
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
            ShellInput::ToggleSettingsMenu(monitor_name) => {
                // Settings is a single-monitor panel — if a previous
                // session left it open on another frame, close that
                // copy first so we don't end up with two parallel
                // Settings windows the user can't tell apart.
                // Without this, switching monitors and reopening
                // Settings opens a fresh copy on the focused monitor
                // while the old copy lingers on whichever monitor
                // the user was on last.
                let target = resolve_frame(&self.window_groups, &monitor_name);
                let target_ptr = target.map(|f| f as *const _);
                for group in self.window_groups.values() {
                    if let Some(frame) = group.frame.as_ref()
                        && Some(frame as *const _) != target_ptr
                    {
                        frame.emit(FrameInput::CloseSettingsMenu);
                    }
                }
                if let Some(frame) = target {
                    frame.emit(FrameInput::ToggleSettingsMenu);
                }
            }
            ShellInput::ToggleWizardMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleWizardMenu);
                }
            }
            ShellInput::OpenSettingsAtSection(monitor_name, section) => {
                // Same monitor-routing as ToggleSettingsMenu —
                // close any non-target instances first so the
                // panel is unambiguously on the focused monitor.
                let target = resolve_frame(&self.window_groups, &monitor_name);
                let target_ptr = target.map(|f| f as *const _);
                for group in self.window_groups.values() {
                    if let Some(frame) = group.frame.as_ref()
                        && Some(frame as *const _) != target_ptr
                    {
                        frame.emit(FrameInput::CloseSettingsMenu);
                    }
                }
                if let Some(frame) = target {
                    frame.emit(FrameInput::OpenSettingsAtSection(section));
                }
            }
            ShellInput::ToggleMdashMenu(monitor_name) => {
                if let Some(frame) = resolve_frame(&self.window_groups, &monitor_name) {
                    frame.emit(FrameInput::ToggleMdashMenu);
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
            let due =
                last_rotation.get().elapsed() >= Duration::from_secs(u64::from(interval_min) * 60);
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
        .then(|| {
            config_manager()
                .config()
                .idle()
                .lock_timeout_minutes()
                .get()
        });
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
                        if !session_locked() {
                            lock_session();
                        }
                    }
                }
                IdleStage::Suspend => {
                    if !inhibited {
                        if !session_locked() {
                            lock_session();
                        }
                        if let Err(e) = std::process::Command::new("systemctl")
                            .arg("suspend")
                            .spawn()
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
