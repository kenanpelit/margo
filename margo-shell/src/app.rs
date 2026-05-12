use crate::{
    components::{ButtonUIRef, Centerbox, menu::MenuType},
    config::{self, AppearanceStyle, Config, Modules, Position},
    get_log_spec,
    i18n::{Localizer, init_localizer},
    ipc::IpcCommand,
    modules::{
        self,
        custom_module::{self, Custom},
        keyboard_layout::KeyboardLayout,
        dns::Dns,
        media_player::MediaPlayer,
        network_speed::NetworkSpeed,
        notifications::Notifications,
        podman::Podman,
        power::Power,
        ufw::Ufw,
        privacy::Privacy,
        settings::{self, Settings, audio},
        system_info::SystemInfo,
        tempo::Tempo,
        tray::TrayModule,
        updates::Updates,
        window_title::WindowTitle,
        workspaces::Workspaces,
    },
    osd::{self, Osd, OsdKind},
    outputs::{HasOutput, Outputs},
    services::ReadOnlyService,
    theme::{MshellTheme, backdrop_color, darken_color, init_theme, use_theme},
};
use flexi_logger::LoggerHandle;
use iced::{
    Alignment, Color, Element, Gradient, Length, OutputEvent, Radians, Subscription, SurfaceId,
    Task, Theme,
    event::listen_with,
    gradient::Linear,
    keyboard, set_exclusive_zone,
    widget::{Row, container, mouse_area},
};
use log::{debug, info, warn};
use std::{collections::HashMap, f32::consts::PI, path::PathBuf};

const OSD_WIDTH: u32 = 250;
const OSD_HEIGHT: u32 = 64;

fn resolve_localizer(config: &Config) -> Localizer {
    Localizer::resolve(config.language.as_deref(), config.region.as_deref())
}

pub struct GeneralConfig {
    outputs: config::Outputs,
    pub modules: Modules,
    pub layer: config::Layer,
    enable_esc_key: bool,
    pub wallpaper: config::WallpaperConfig,
}

pub struct App {
    config_path: PathBuf,
    logger: LoggerHandle,
    pub general_config: GeneralConfig,
    pub outputs: Outputs,
    pub custom: HashMap<String, Custom>,
    pub updates: Option<Updates>,
    pub workspaces: Workspaces,
    pub window_title: WindowTitle,
    pub system_info: SystemInfo,
    pub network_speed: NetworkSpeed,
    pub dns: Dns,
    pub ufw: Ufw,
    pub power: Power,
    pub podman: Podman,
    pub keyboard_layout: KeyboardLayout,
    pub tray: TrayModule,
    pub tempo: Tempo,
    pub privacy: Privacy,
    pub settings: Settings,
    pub media_player: MediaPlayer,
    pub notifications: Notifications,
    pub osd: Osd,
    pub visible: bool,
    /// margo'nun aktif output'unun bare adı ("DP-3"). IPC-tetiklenmiş
    /// menü açılışlarında doğru bar surface'i seçmek için tutulur;
    /// `WallpaperRefresh` her geldiğinde güncellenir.
    pub active_output: Option<String>,
    /// Dedicated wallpaper renderer thread (separate Wayland
    /// connection, direct wl_shm buffer attach). iced's Image
    /// widget on a Background-layer surface silently refused to
    /// upload pixels; the renderer is the pandora/wpaperd pattern
    /// instead. Cheap to clone (wraps an mpsc::Sender).
    pub wallpaper_renderer: crate::wallpaper::WallpaperRenderer,
    /// Last path sent per output, so we don't re-decode on every
    /// state.json tick. Plain string map; the renderer thread also
    /// dedupes internally, this just avoids the channel churn.
    pub wallpaper_last_path: HashMap<String, String>,
    /// Shuffle assignments owned by the main thread (we still pick
    /// the path here so the renderer thread doesn't need to know
    /// about config; it just receives "set this path on this
    /// output").
    pub shuffle_assignments: HashMap<String, std::path::PathBuf>,
    /// Cached directory listing for shuffle mode.
    pub shuffle_pool: Vec<std::path::PathBuf>,
    /// Cursor for Sequential shuffle mode.
    pub shuffle_cursor: usize,
}

#[derive(Debug, Clone)]
pub enum Message {
    ConfigChanged(Box<Config>),
    ToggleMenu(MenuType, SurfaceId, ButtonUIRef),
    CloseMenu(SurfaceId),
    Custom(String, custom_module::Message),
    Updates(modules::updates::Message),
    Workspaces(modules::workspaces::Message),
    WindowTitle(modules::window_title::Message),
    SystemInfo(modules::system_info::Message),
    NetworkSpeed(modules::network_speed::Message),
    Dns(modules::dns::Message),
    Ufw(modules::ufw::Message),
    Power(modules::power::Message),
    Podman(modules::podman::Message),
    /// IPC tarafından tetiklenmiş menü açma — bar'ın ilk surface'inde
    /// sentetik bir ButtonUIRef ile ilgili MenuType'ı toggle eder.
    OpenIpcMenu(String),
    KeyboardLayout(modules::keyboard_layout::Message),
    Tray(modules::tray::Message),
    Tempo(modules::tempo::Message),
    Privacy(modules::privacy::Message),
    Settings(modules::settings::Message),
    MediaPlayer(modules::media_player::Message),
    Notifications(modules::notifications::Message),
    Osd(osd::Message),
    IpcOsdCommand(IpcCommand),
    OutputEvent(OutputEvent),
    CloseAllMenus,
    ResumeFromSleep,
    None,
    ToggleVisibility,
    /// CompositorService announced a new wallpaper map. Carries
    /// both the state.json-driven path map and the active tag id
    /// per output, so the handler can resolve through the
    /// precedence chain (shuffle → mshell.toml tags → state.json).
    WallpaperRefresh {
        wallpapers: std::collections::HashMap<String, String>,
        active_tags: std::collections::HashMap<String, u32>,
        /// margo'nun aktif output'unun bare adı ("DP-3"). IPC-tetiklenmiş
        /// menü açılışlarında doğru bar surface'i seçmek için saklanır.
        active_output: Option<String>,
    },
    /// Async image-decode result. `output_name` → `path` → handle.
    /// Periodic shuffle rotate (cadence from
    /// `[wallpaper.shuffle].interval_secs`). Carries no payload;
    /// the handler reshuffles the per-output assignments and
    /// pushes a fresh `WallpaperRefresh`.
    WallpaperShuffleTick,
}

/// Pick one image from the pool. `Random` mode draws uniformly;
/// `Sequential` advances `cursor` modulo the pool size.
/// `#RRGGBB` → `[r, g, b]`. Returns None on parse failure so the
/// caller can apply a hard-coded fallback.
fn parse_hex_rgb(s: &str) -> Option<[u8; 3]> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&s[0..2], 16).ok()?,
        u8::from_str_radix(&s[2..4], 16).ok()?,
        u8::from_str_radix(&s[4..6], 16).ok()?,
    ])
}

fn pick_from_pool(
    pool: &[std::path::PathBuf],
    cursor: &mut usize,
    mode: config::WallpaperShuffleMode,
) -> std::path::PathBuf {
    debug_assert!(!pool.is_empty(), "pool should be non-empty here");
    let pick = match mode {
        config::WallpaperShuffleMode::Random => {
            use rand::Rng;
            let idx = rand::thread_rng().gen_range(0..pool.len());
            pool[idx].clone()
        }
        config::WallpaperShuffleMode::Sequential => {
            let pick = pool[*cursor % pool.len()].clone();
            *cursor = (*cursor + 1) % pool.len();
            pick
        }
    };
    pick
}

/// Scan `directory` for image files. Recognises `.jpg`, `.jpeg`,
/// `.png`, `.webp` (case-insensitive). Non-recursive. Returns paths
/// sorted by name so Sequential mode is deterministic across runs.
fn scan_wallpaper_directory(directory: &str) -> Vec<std::path::PathBuf> {
    let expanded = match shellexpand::full(directory) {
        Ok(p) => p.to_string(),
        Err(e) => {
            log::warn!("wallpaper.shuffle.directory expand failed: {e}");
            return Vec::new();
        }
    };
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    let entries = match std::fs::read_dir(&expanded) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("wallpaper.shuffle.directory {expanded} read failed: {e}");
            return Vec::new();
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_image = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| {
                let lower = ext.to_ascii_lowercase();
                matches!(lower.as_str(), "jpg" | "jpeg" | "png" | "webp")
            })
            .unwrap_or(false);
        if is_image {
            paths.push(path);
        }
    }
    paths.sort();
    paths
}

impl App {
    pub fn new(
        (logger, config, config_path): (LoggerHandle, Config, PathBuf),
    ) -> impl FnOnce() -> (Self, Task<Message>) {
        move || {
            let outputs = Outputs::new(
                config.appearance.style,
                config.position,
                config.layer,
                config.appearance.scale_factor,
            );

            let custom = config
                .custom_modules
                .clone()
                .into_iter()
                .map(|o| (o.name.clone(), Custom::new(o)))
                .collect();

            init_theme(MshellTheme::new(
                config.position,
                &config.appearance,
                &config.animations,
            ));
            init_localizer(resolve_localizer(&config));

            let notifications = Notifications::new(config.notifications);

            (
                App {
                    config_path,
                    logger,
                    general_config: GeneralConfig {
                        outputs: config.outputs,
                        modules: config.modules,
                        layer: config.layer,
                        enable_esc_key: config.enable_esc_key,
                        wallpaper: config.wallpaper,
                    },
                    outputs,
                    custom,
                    updates: config.updates.map(Updates::new),
                    workspaces: Workspaces::new(config.workspaces),
                    window_title: WindowTitle::new(config.window_title),
                    system_info: SystemInfo::new(config.system_info),
                    network_speed: NetworkSpeed::new(config.network_speed),
                    dns: Dns::new(config.dns),
                    ufw: Ufw::new(config.ufw),
                    power: Power::new(config.power),
                    podman: Podman::new(config.podman),
                    keyboard_layout: KeyboardLayout::new(config.keyboard_layout),
                    tray: TrayModule::new(config.tray),
                    tempo: Tempo::new(config.tempo),
                    privacy: Privacy::default(),
                    settings: Settings::new(config.settings),
                    notifications,
                    media_player: MediaPlayer::new(config.media_player),
                    osd: Osd::new(config.osd),
                    visible: true,
                    active_output: None,
                    wallpaper_renderer: crate::wallpaper::WallpaperRenderer::spawn(),
                    wallpaper_last_path: HashMap::new(),
                    shuffle_assignments: HashMap::new(),
                    shuffle_pool: Vec::new(),
                    shuffle_cursor: 0,
                },
                Task::none(),
            )
        }
    }

    fn refresh_config(&mut self, config: Box<Config>) {
        init_theme(MshellTheme::new(
            config.position,
            &config.appearance,
            &config.animations,
        ));
        init_localizer(resolve_localizer(&config));
        self.general_config = GeneralConfig {
            outputs: config.outputs,
            modules: config.modules,
            wallpaper: config.wallpaper.clone(),
            layer: config.layer,
            enable_esc_key: config.enable_esc_key,
        };
        let custom = config
            .custom_modules
            .into_iter()
            .map(|o| (o.name.clone(), Custom::new(o)))
            .collect();

        self.custom = custom;
        self.updates = config.updates.map(Updates::new);

        // ignore task, since config change should not generate any
        let _ = self
            .workspaces
            .update(modules::workspaces::Message::ConfigReloaded(
                config.workspaces,
            ))
            .map(Message::Workspaces);

        self.window_title
            .update(modules::window_title::Message::ConfigReloaded(
                config.window_title,
            ));

        self.system_info = SystemInfo::new(config.system_info);
        self.network_speed = NetworkSpeed::new(config.network_speed);
        self.dns = Dns::new(config.dns);
        self.ufw = Ufw::new(config.ufw);
        self.power = Power::new(config.power);
        self.podman = Podman::new(config.podman);

        let _ = self
            .keyboard_layout
            .update(modules::keyboard_layout::Message::ConfigReloaded(
                config.keyboard_layout,
            ))
            .map(Message::KeyboardLayout);

        self.tempo
            .update(modules::tempo::Message::ConfigReloaded(config.tempo));
        self.settings
            .update(modules::settings::Message::ConfigReloaded(config.settings));
        self.media_player
            .update(modules::media_player::Message::ConfigReloaded(
                config.media_player,
            ));
        let _ = self
            .notifications
            .update(modules::notifications::Message::ConfigReloaded(
                config.notifications,
            ));
        self.osd.update(osd::Message::ConfigReloaded(config.osd));
    }

    pub fn theme(&self) -> Theme {
        use_theme(|t| t.iced_theme.clone())
    }

    pub fn scale_factor(&self) -> f64 {
        use_theme(|t| t.scale_factor)
    }

    /// Build OSD display info (kind, normalised value, muted) for the given
    /// IPC command, reading current state from the Settings services.
    fn osd_info_for(&self, cmd: &IpcCommand) -> Option<(OsdKind, f32, bool)> {
        fn normalise(cur: u32, max: u32) -> f32 {
            if max > 0 {
                cur as f32 / max as f32
            } else {
                0.0
            }
        }

        match cmd {
            IpcCommand::VolumeUp { .. } | IpcCommand::VolumeDown { .. } => {
                // Use slider value — it has the optimistic RequestAndTimeout update,
                // which was computed from real_sink_volume in volume_adjust().
                let vol = self.settings.audio().current_sink_volume().unwrap_or(0);
                let muted = self.settings.audio().is_sink_muted().unwrap_or(false);
                Some((
                    OsdKind::Volume,
                    normalise(vol, audio::AudioSettings::vol_max()),
                    muted,
                ))
            }
            IpcCommand::VolumeToggleMute { .. } => {
                let vol = self.settings.audio().real_sink_volume().unwrap_or(0);
                // Invert: the toggle was just sent but PulseAudio hasn't
                // round-tripped yet, so the current state is stale.
                let muted = !self.settings.audio().is_sink_muted().unwrap_or(false);
                Some((
                    OsdKind::Volume,
                    normalise(vol, audio::AudioSettings::vol_max()),
                    muted,
                ))
            }
            IpcCommand::MicrophoneUp { .. } | IpcCommand::MicrophoneDown { .. } => {
                // Use slider value — it has the optimistic RequestAndTimeout update,
                // which was computed from real_source_volume in microphone_adjust().
                let vol = self.settings.audio().current_source_volume().unwrap_or(0);
                let muted = self.settings.audio().is_source_muted().unwrap_or(false);
                Some((
                    OsdKind::Microphone,
                    normalise(vol, audio::AudioSettings::mic_max()),
                    muted,
                ))
            }
            IpcCommand::MicrophoneToggleMute { .. } => {
                let vol = self.settings.audio().real_source_volume().unwrap_or(0);
                // Invert: the toggle was just sent but PulseAudio hasn't
                // round-tripped yet, so the current state is stale.
                let muted = !self.settings.audio().is_source_muted().unwrap_or(false);
                Some((
                    OsdKind::Microphone,
                    normalise(vol, audio::AudioSettings::mic_max()),
                    muted,
                ))
            }
            IpcCommand::BrightnessUp { .. } | IpcCommand::BrightnessDown { .. } => self
                .settings
                .brightness()
                .current_brightness()
                .map(|(cur, max)| (OsdKind::Brightness, normalise(cur, max), false)),
            IpcCommand::ToggleAirplaneMode { .. } => {
                // After toggle: the new state is the opposite of current.
                // For toggles, `muted` carries the active/enabled state; `value` is unused.
                let active = !self.settings.network().is_airplane_mode().unwrap_or(false);
                Some((OsdKind::Airplane, 0.0, active))
            }
            IpcCommand::ToggleIdleInhibitor { .. } => {
                if let Some(idle_inhibitor) = self.settings.idle_inhibitor() {
                    let active = idle_inhibitor.is_inhibited();
                    Some((OsdKind::IdleInhibitor, 0.0, active))
                } else {
                    None
                }
            }
            IpcCommand::ToggleVisibility => None,
            // Menü açma komutları osd_info_for tarafından kullanılmıyor.
            IpcCommand::Dns
            | IpcCommand::Network
            | IpcCommand::System
            | IpcCommand::Media
            | IpcCommand::Settings
            | IpcCommand::Notifications
            | IpcCommand::Updates
            | IpcCommand::Tempo
            | IpcCommand::Ufw
            | IpcCommand::Power
            | IpcCommand::Podman => None,
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ConfigChanged(config) => {
                info!("New config: {config:?}");
                let mut tasks = Vec::new();
                info!(
                    "Current outputs: {:?}, new outputs: {:?}",
                    self.general_config.outputs, config.outputs
                );
                let (bar_position, bar_style, scale_factor) =
                    use_theme(|t| (t.bar_position, t.bar_style, t.scale_factor));
                if self.general_config.outputs != config.outputs
                    || bar_position != config.position
                    || bar_style != config.appearance.style
                    || scale_factor != config.appearance.scale_factor
                    || self.general_config.layer != config.layer
                {
                    warn!("Outputs changed, syncing");
                    tasks.push(self.outputs.sync(
                        config.appearance.style,
                        &config.outputs,
                        config.position,
                        config.layer,
                        config.appearance.scale_factor,
                    ));
                }

                self.logger.set_new_spec(get_log_spec(&config.log_level));
                self.refresh_config(config);

                Task::batch(tasks)
            }
            Message::ToggleMenu(menu_type, id, button_ui_ref) => {
                let mut cmd = vec![];
                match &menu_type {
                    MenuType::Updates => {
                        if let Some(updates) = self.updates.as_mut() {
                            updates.update(modules::updates::Message::MenuOpened);
                        }
                    }
                    MenuType::Tray(name) => {
                        self.tray
                            .update(modules::tray::Message::MenuOpened(name.clone()));
                    }
                    MenuType::Settings => {
                        cmd.push(
                            match self.settings.update(modules::settings::Message::MenuOpened) {
                                modules::settings::Action::Command(task) => {
                                    task.map(Message::Settings)
                                }
                                _ => Task::none(),
                            },
                        );
                    }
                    _ => {}
                };
                cmd.push(self.outputs.toggle_menu(
                    id,
                    menu_type,
                    button_ui_ref,
                    self.general_config.enable_esc_key,
                ));

                Task::batch(cmd)
            }
            Message::CloseMenu(id) => {
                self.outputs
                    .close_menu(id, None, self.general_config.enable_esc_key)
            }
            Message::Custom(name, msg) => {
                if let Some(custom) = self.custom.get_mut(&name) {
                    custom.update(msg);
                }

                Task::none()
            }
            Message::Updates(msg) => {
                if let Some(updates) = self.updates.as_mut() {
                    match updates.update(msg) {
                        modules::updates::Action::None => Task::none(),
                        modules::updates::Action::CheckForUpdates(task) => {
                            task.map(Message::Updates)
                        }
                        modules::updates::Action::CloseMenu(id, task) => Task::batch(vec![
                            task.map(Message::Updates),
                            self.outputs.close_menu(
                                id,
                                Some(MenuType::Updates),
                                self.general_config.enable_esc_key,
                            ),
                        ]),
                    }
                } else {
                    Task::none()
                }
            }
            Message::Workspaces(msg) => self.workspaces.update(msg).map(Message::Workspaces),
            Message::WindowTitle(msg) => {
                self.window_title.update(msg);
                Task::none()
            }
            Message::NetworkSpeed(msg) => {
                self.network_speed.update(msg);
                Task::none()
            }
            Message::Dns(msg) => self.dns.update(msg).map(Message::Dns),
            Message::Ufw(msg) => self.ufw.update(msg).map(Message::Ufw),
            Message::Power(msg) => self.power.update(msg).map(Message::Power),
            Message::Podman(msg) => self.podman.update(msg).map(Message::Podman),
            Message::OpenIpcMenu(name) => {
                let menu_type = match name.as_str() {
                    "dns" => Some(MenuType::Dns),
                    "network" => Some(MenuType::NetworkSpeed),
                    "system" => Some(MenuType::SystemInfo),
                    "media" => Some(MenuType::MediaPlayer),
                    "settings" => Some(MenuType::Settings),
                    "notifications" => Some(MenuType::Notifications),
                    "updates" => Some(MenuType::Updates),
                    "tempo" => Some(MenuType::Tempo),
                    "ufw" => Some(MenuType::Ufw),
                    "power" => Some(MenuType::Power),
                    "podman" => Some(MenuType::Podman),
                    _ => None,
                };
                if let Some(mt) = menu_type {
                    // Aktif output'un bar surface'ini bul; bulunmazsa
                    // ilk surface'e düş (örn. mshell başlangıçta state
                    // henüz okunmamışsa).
                    let active = self.active_output.as_deref();
                    let entry = self
                        .outputs
                        .iter()
                        .find_map(|(name, si, _)| {
                            let si = si.as_ref()?;
                            // outputs.rs'deki name çok-kelimeli olabilir
                            // ("DP-3 Acme XYZ"); margo'nun active_output'u
                            // ise bare ad. İlk kelimeyi karşılaştır.
                            let bare = name.split_whitespace().next().unwrap_or(name);
                            if active == Some(bare) {
                                Some((si.id, si.output_logical_size))
                            } else {
                                None
                            }
                        })
                        .or_else(|| {
                            self.outputs.iter().find_map(|(_, si, _)| {
                                si.as_ref().map(|s| (s.id, s.output_logical_size))
                            })
                        });
                    if let Some((id, size)) = entry {
                        let (w, h) = size.unwrap_or((1920, 1080));
                        let ui_ref = ButtonUIRef {
                            position: iced::Point::new(w as f32 / 2.0, 24.0),
                            viewport: (w as f32, h as f32),
                        };
                        return Task::done(Message::ToggleMenu(mt, id, ui_ref));
                    }
                }
                Task::none()
            }
            Message::SystemInfo(msg) => {
                self.system_info.update(msg);
                Task::none()
            }
            Message::KeyboardLayout(message) => self
                .keyboard_layout
                .update(message)
                .map(Message::KeyboardLayout),
            Message::Tray(msg) => match self.tray.update(msg) {
                modules::tray::Action::None => Task::none(),
                modules::tray::Action::ToggleMenu(name, id, button_ui_ref) => {
                    self.outputs.toggle_menu(
                        id,
                        MenuType::Tray(name),
                        button_ui_ref,
                        self.general_config.enable_esc_key,
                    )
                }
                modules::tray::Action::TrayMenuCommand(task) => Task::batch(vec![
                    self.outputs
                        .close_all_menus(self.general_config.enable_esc_key),
                    task.map(Message::Tray),
                ]),
                modules::tray::Action::TrayMenuCommandKeepOpen(task) => task.map(Message::Tray),
                modules::tray::Action::CloseTrayMenu(name) => self
                    .outputs
                    .close_all_menu_if(MenuType::Tray(name), self.general_config.enable_esc_key),
            },
            Message::Tempo(message) => match self.tempo.update(message) {
                modules::tempo::Action::None => Task::none(),
            },
            Message::Privacy(msg) => {
                self.privacy.update(msg);
                Task::none()
            }
            Message::Settings(message) => match self.settings.update(message) {
                modules::settings::Action::None => Task::none(),
                modules::settings::Action::Command(task) => task.map(Message::Settings),
                modules::settings::Action::CloseMenu(id) => {
                    self.outputs
                        .close_menu(id, None, self.general_config.enable_esc_key)
                }
                modules::settings::Action::RequestKeyboard(id) => self.outputs.request_keyboard(id),
                modules::settings::Action::ReleaseKeyboard(id) => self.outputs.release_keyboard(id),
                modules::settings::Action::ReleaseKeyboardWithCommand(id, task) => {
                    Task::batch(vec![
                        task.map(Message::Settings),
                        self.outputs.release_keyboard(id),
                    ])
                }
            },
            Message::OutputEvent(event) => match event {
                OutputEvent::Added(info) => {
                    info!("Output created: {info:?}");
                    let name = &format!("{} {} {}", info.name, info.make, info.model);

                    let (bar_style, bar_position, scale_factor) =
                        use_theme(|t| (t.bar_style, t.bar_position, t.scale_factor));
                    let tasks = vec![self.outputs.add(
                        bar_style,
                        &self.general_config.outputs,
                        bar_position,
                        self.general_config.layer,
                        name,
                        info.id,
                        scale_factor,
                    )];
                    // `Outputs::add` (re-)created the ShellInfo with
                    // None size. Stamp the real logical size in.
                    if let Some((w, h)) = info.logical_size {
                        self.outputs
                            .set_output_logical_size(info.id, w as u32, h as u32);
                    }
                    // Wallpaper layer surface is owned by the
                    // dedicated `crate::wallpaper` thread; nothing to
                    // do here.
                    Task::batch(tasks)
                }
                OutputEvent::Removed(output_id) => {
                    info!("Output destroyed");
                    let (bar_style, bar_position, scale_factor) =
                        use_theme(|t| (t.bar_style, t.bar_position, t.scale_factor));
                    self.outputs.remove(
                        bar_style,
                        bar_position,
                        self.general_config.layer,
                        output_id,
                        scale_factor,
                    )
                }
                OutputEvent::InfoChanged(_) => Task::none(),
            },
            Message::MediaPlayer(msg) => match self.media_player.update(msg) {
                modules::media_player::Action::None => Task::none(),
                modules::media_player::Action::Command(task) => task.map(Message::MediaPlayer),
            },
            Message::CloseAllMenus => {
                if self.outputs.menu_is_open() {
                    self.outputs
                        .close_all_menus(self.general_config.enable_esc_key)
                } else {
                    Task::none()
                }
            }
            Message::ResumeFromSleep => {
                let (bar_style, bar_position, scale_factor) =
                    use_theme(|t| (t.bar_style, t.bar_position, t.scale_factor));
                self.outputs.sync(
                    bar_style,
                    &self.general_config.outputs,
                    bar_position,
                    self.general_config.layer,
                    scale_factor,
                )
            }
            Message::Notifications(message) => match self.notifications.update(message) {
                modules::notifications::Action::None => Task::none(),
                modules::notifications::Action::Task(task) => task.map(Message::Notifications),
                modules::notifications::Action::Show(task) => {
                    let position = self.notifications.toast_position();
                    let width = self.notifications.toast_width();
                    // Initial surface yüksekliği config'in toast_max_height'i —
                    // tek toast tam fit eder, sensor daha büyükse grow-only.
                    let initial_h = self.notifications.toast_initial_height();
                    Task::batch(vec![
                        task.map(Message::Notifications),
                        self.outputs.show_toast_layer(width, position, initial_h),
                    ])
                }
                modules::notifications::Action::Hide(task) => Task::batch(vec![
                    task.map(Message::Notifications),
                    self.outputs.hide_toast_layer(),
                ]),
                modules::notifications::Action::UpdateToastInputRegion(content_size) => {
                    let position = self.notifications.toast_position();
                    self.outputs
                        .update_toast_input_region(content_size, position)
                }
            },
            Message::IpcOsdCommand(cmd) => {
                let mut tasks = vec![];

                // Execute the action via Settings.
                let action = match &cmd {
                    IpcCommand::VolumeUp { .. } => self.settings.volume_adjust(true),
                    IpcCommand::VolumeDown { .. } => self.settings.volume_adjust(false),
                    IpcCommand::VolumeToggleMute { .. } => self.settings.toggle_mute(),
                    IpcCommand::MicrophoneUp { .. } => self.settings.microphone_adjust(true),
                    IpcCommand::MicrophoneDown { .. } => self.settings.microphone_adjust(false),
                    IpcCommand::MicrophoneToggleMute { .. } => {
                        self.settings.microphone_toggle_mute()
                    }
                    IpcCommand::BrightnessUp { .. } => self.settings.brightness_adjust(true),
                    IpcCommand::BrightnessDown { .. } => self.settings.brightness_adjust(false),
                    IpcCommand::ToggleAirplaneMode { .. } => self.settings.toggle_airplane(),
                    IpcCommand::ToggleIdleInhibitor { .. } => self.settings.toggle_idle_inhibitor(),
                    IpcCommand::ToggleVisibility => unreachable!(),
                    // Menü açma komutları subscription mapping'inde
                    // OpenIpcMenu'ya yönlendirildiği için buraya hiç gelmez.
                    IpcCommand::Dns
                    | IpcCommand::Network
                    | IpcCommand::System
                    | IpcCommand::Media
                    | IpcCommand::Settings
                    | IpcCommand::Notifications
                    | IpcCommand::Updates
                    | IpcCommand::Tempo
                    | IpcCommand::Ufw
                    | IpcCommand::Power
                    | IpcCommand::Podman => unreachable!(),
                };
                if let settings::Action::Command(task) = action {
                    tasks.push(task.map(Message::Settings));
                }

                // Show OSD overlay if enabled.
                if self.osd.config().enabled && !cmd.no_osd() {
                    let osd_info = self.osd_info_for(&cmd);

                    if let Some((kind, value, muted)) = osd_info
                        && let osd::Action::Show(timer) =
                            self.osd.update(osd::Message::Show { kind, value, muted })
                    {
                        tasks.push(timer.map(Message::Osd));
                        tasks.push(self.outputs.show_osd_layer(OSD_WIDTH, OSD_HEIGHT));
                    }
                }

                Task::batch(tasks)
            }
            Message::Osd(msg) => match self.osd.update(msg) {
                osd::Action::Hide => self.outputs.hide_osd_layer(),
                _ => Task::none(),
            },
            Message::None => Task::none(),
            Message::WallpaperRefresh { wallpapers, active_tags, active_output } => {
                // active_output her zaman güncellenir — IPC menü açılışları
                // bunu kullanıyor (focused output'un bar'ında açar).
                if self.active_output != active_output {
                    self.active_output = active_output;
                }
                // [wallpaper].enabled = false → skip everything;
                // user wants to run swaybg/swww or no wallpaper at
                // all.
                if !self.general_config.wallpaper.enabled {
                    return Task::none();
                }
                info!(
                    "WallpaperRefresh: {} wallpapers, {} active tags",
                    wallpapers.len(),
                    active_tags.len()
                );
                // Precedence chain (resolved per output):
                //   1. shuffle.enabled → mshell-chosen pool pick
                //   2. [wallpaper.tags] in mshell.toml → tag-based
                //   3. state.json wallpapers map (margo fallback)
                let map = self.resolve_wallpaper_map(wallpapers, &active_tags);
                let fit = self.general_config.wallpaper.fit;
                let fallback = parse_hex_rgb(&self.general_config.wallpaper.fallback_color)
                    .unwrap_or([0x1e, 0x1e, 0x2e]);
                for (output, path) in map.iter() {
                    if self.wallpaper_last_path.get(output) == Some(path) {
                        continue;
                    }
                    info!("WallpaperRefresh: dispatch output={} path={}", output, path);
                    self.wallpaper_last_path
                        .insert(output.clone(), path.clone());
                    let path_arg = if path.is_empty() {
                        None
                    } else {
                        Some(std::path::PathBuf::from(path))
                    };
                    self.wallpaper_renderer.set(output.clone(), path_arg, fit, fallback);
                }
                // Drop entries for outputs no longer in the map
                // (unplugged) so a re-add re-dispatches.
                self.wallpaper_last_path
                    .retain(|k, _| map.contains_key(k));
                Task::none()
            }
            Message::WallpaperShuffleTick => {
                // Force-reshuffle on the next refresh.
                self.shuffle_assignments.clear();
                self.wallpaper_last_path.clear();
                Task::done(Message::WallpaperRefresh {
                    wallpapers: self.last_wallpaper_map(),
                    active_tags: HashMap::new(),
                    active_output: self.active_output.clone(),
                })
            }
            Message::ToggleVisibility => {
                self.visible = !self.visible;
                let (bar_style, scale_factor, bar_height) =
                    use_theme(|t| (t.bar_style, t.scale_factor, t.bar_height));
                let height = if self.visible {
                    (bar_height
                        - match bar_style {
                            AppearanceStyle::Solid | AppearanceStyle::Gradient => 8.,
                            AppearanceStyle::Islands => 0.,
                        })
                        * scale_factor
                } else {
                    0.0
                };

                Task::batch(
                    self.outputs
                        .iter()
                        .filter_map(|(_, shell_info, _)| {
                            shell_info
                                .as_ref()
                                .map(|info| set_exclusive_zone(info.id, height as i32))
                        })
                        .collect::<Vec<_>>(),
                )
            }
        }
    }

    pub fn view(&'_ self, id: SurfaceId) -> Element<'_, Message> {
        match self.outputs.has(id) {
            Some(HasOutput::Main) => {
                if !self.visible {
                    return Row::new().into();
                }

                let [left, center, right] = self.modules_section(id);

                let (space, bar_style, bar_position, opacity, menu, bar_height) = use_theme(|t| {
                    (
                        t.space,
                        t.bar_style,
                        t.bar_position,
                        t.opacity,
                        t.menu,
                        t.bar_height,
                    )
                });
                let centerbox = Centerbox::new([left, center, right])
                    .spacing(space.xxs)
                    .width(Length::Fill)
                    .align_items(Alignment::Center)
                    .height(if bar_style == AppearanceStyle::Islands {
                        bar_height
                    } else {
                        bar_height - space.xs as f64
                    } as f32)
                    .padding(if bar_style == AppearanceStyle::Islands {
                        [space.xxs, space.xxs]
                    } else {
                        [0.0, 0.0]
                    });

                let menu_is_open = self.outputs.menu_is_open();
                let status_bar = container(centerbox).style(move |t: &Theme| container::Style {
                    background: match bar_style {
                        AppearanceStyle::Gradient => Some({
                            let start_color = t.palette().background.scale_alpha(opacity);

                            let start_color = if menu_is_open {
                                darken_color(start_color, menu.backdrop)
                            } else {
                                start_color
                            };

                            let end_color = if menu_is_open {
                                backdrop_color(menu.backdrop)
                            } else {
                                Color::TRANSPARENT
                            };

                            Gradient::Linear(
                                Linear::new(Radians(PI))
                                    .add_stop(
                                        0.0,
                                        match bar_position {
                                            Position::Top => start_color,
                                            Position::Bottom => end_color,
                                        },
                                    )
                                    .add_stop(
                                        1.0,
                                        match bar_position {
                                            Position::Top => end_color,
                                            Position::Bottom => start_color,
                                        },
                                    ),
                            )
                            .into()
                        }),
                        AppearanceStyle::Solid => Some({
                            let bg = t.palette().background.scale_alpha(opacity);
                            if menu_is_open {
                                darken_color(bg, menu.backdrop)
                            } else {
                                bg
                            }
                            .into()
                        }),
                        AppearanceStyle::Islands => {
                            if menu_is_open {
                                Some(backdrop_color(menu.backdrop).into())
                            } else {
                                None
                            }
                        }
                    },
                    ..Default::default()
                });

                if self.outputs.menu_is_open() {
                    mouse_area(status_bar)
                        .on_release(Message::CloseMenu(id))
                        .into()
                } else {
                    status_bar.into()
                }
            }
            Some(HasOutput::Menu(Some(open_menu))) => {
                let ui_ref = open_menu.button_ui_ref;
                match &open_menu.menu_type {
                    MenuType::Updates => {
                        if let Some(updates) = self.updates.as_ref() {
                            self.menu_wrapper(
                                id,
                                updates.menu_view(id).map(Message::Updates),
                                ui_ref,
                            )
                        } else {
                            Row::new().into()
                        }
                    }
                    MenuType::Tray(name) => {
                        self.menu_wrapper(id, self.tray.menu_view(name).map(Message::Tray), ui_ref)
                    }
                    MenuType::Notifications => self.menu_wrapper(
                        id,
                        self.notifications.menu_view().map(Message::Notifications),
                        ui_ref,
                    ),
                    MenuType::Settings => self.menu_wrapper(
                        id,
                        self.settings
                            .menu_view(id, use_theme(|t| t.bar_position))
                            .map(Message::Settings),
                        ui_ref,
                    ),
                    MenuType::MediaPlayer => self.menu_wrapper(
                        id,
                        self.media_player.menu_view().map(Message::MediaPlayer),
                        ui_ref,
                    ),
                    MenuType::SystemInfo => self.menu_wrapper(
                        id,
                        self.system_info.menu_view().map(Message::SystemInfo),
                        ui_ref,
                    ),
                    MenuType::NetworkSpeed => self.menu_wrapper(
                        id,
                        self.network_speed.menu_view().map(Message::NetworkSpeed),
                        ui_ref,
                    ),
                    MenuType::Dns => self.menu_wrapper(
                        id,
                        self.dns.menu_view().map(Message::Dns),
                        ui_ref,
                    ),
                    MenuType::Ufw => self.menu_wrapper(
                        id,
                        self.ufw.menu_view().map(Message::Ufw),
                        ui_ref,
                    ),
                    MenuType::Power => self.menu_wrapper(
                        id,
                        self.power.menu_view().map(Message::Power),
                        ui_ref,
                    ),
                    MenuType::Podman => self.menu_wrapper(
                        id,
                        self.podman.menu_view().map(Message::Podman),
                        ui_ref,
                    ),
                    MenuType::Tempo => {
                        self.menu_wrapper(id, self.tempo.menu_view().map(Message::Tempo), ui_ref)
                    }
                }
            }
            Some(HasOutput::Menu(None)) => Row::new().into(),
            Some(HasOutput::Toast) => self.notifications.toast_view().map(Message::Notifications),
            Some(HasOutput::Osd) => self.osd.view().map(Message::Osd),
            None => Row::new().into(),
        }
    }

    /// Decide the final per-output wallpaper path map by walking the
    /// precedence chain: shuffle → mshell.toml `[wallpaper.tags]` →
    /// state.json (backward compat).
    fn resolve_wallpaper_map(
        &mut self,
        wallpapers: HashMap<String, String>,
        active_tags: &HashMap<String, u32>,
    ) -> HashMap<String, String> {
        // 1. Shuffle bypasses everything.
        if self.general_config.wallpaper.shuffle.enabled {
            return self.maybe_apply_shuffle(wallpapers);
        }

        // 2. mshell.toml [wallpaper.tags] — if present, look up the
        //    active tag per output. Each output sees the tag it's
        //    currently displaying.
        if !self.general_config.wallpaper.tags.is_empty() {
            let tag_map = self.general_config.wallpaper.tags.clone();
            let mut out = HashMap::with_capacity(wallpapers.len());
            for (output, fallback_path) in wallpapers.into_iter() {
                let resolved = active_tags
                    .get(&output)
                    .and_then(|tag_id| tag_map.get(&tag_id.to_string()))
                    .map(|s| {
                        // ~ expansion
                        shellexpand::full(s)
                            .map(|c| c.into_owned())
                            .unwrap_or_else(|_| s.clone())
                    })
                    .unwrap_or(fallback_path);
                out.insert(output, resolved);
            }
            return out;
        }

        // 3. state.json passthrough.
        wallpapers
    }

    /// If `[wallpaper.shuffle]` is enabled, override the incoming
    /// per-output path map with mshell-chosen picks. Otherwise pass
    /// the map through untouched.
    ///
    /// Pool is lazy-loaded on first call (and on `WallpaperShuffleTick`
    /// after the assignments map is cleared). Per-output assignments
    /// are sticky across ticks so tag swaps don't reshuffle.
    fn maybe_apply_shuffle(
        &mut self,
        incoming: HashMap<String, String>,
    ) -> HashMap<String, String> {
        let cfg = &self.general_config.wallpaper.shuffle;
        if !cfg.enabled {
            return incoming;
        }

        // Lazy pool scan. Empty directory → log warn, fall through to
        // the original map (better than rendering blank).
        if self.shuffle_pool.is_empty() {
            self.shuffle_pool =
                scan_wallpaper_directory(&self.general_config.wallpaper.shuffle.directory);
            log::info!(
                "wallpaper.shuffle: scanned {} images from {}",
                self.shuffle_pool.len(),
                self.general_config.wallpaper.shuffle.directory
            );
        }
        if self.shuffle_pool.is_empty() {
            return incoming;
        }

        let pool = self.shuffle_pool.clone();
        let mode = self.general_config.wallpaper.shuffle.mode;
        let per_output = self.general_config.wallpaper.shuffle.per_output;

        // Decide which outputs need a fresh pick — incoming map's
        // keys are the truth source (margo backend filled it from
        // state.json's outputs[]).
        let mut shared_pick: Option<std::path::PathBuf> = None;
        let mut out_map = HashMap::with_capacity(incoming.len());
        for output in incoming.keys() {
            let pick = if per_output {
                self.shuffle_assignments
                    .entry(output.clone())
                    .or_insert_with(|| pick_from_pool(&pool, &mut self.shuffle_cursor, mode))
                    .clone()
            } else {
                let pick = shared_pick.get_or_insert_with(|| {
                    pick_from_pool(&pool, &mut self.shuffle_cursor, mode)
                });
                self.shuffle_assignments
                    .insert(output.clone(), pick.clone());
                pick.clone()
            };
            out_map.insert(output.clone(), pick.to_string_lossy().into_owned());
        }
        // Drop assignments for outputs that vanished.
        self.shuffle_assignments
            .retain(|k, _| incoming.contains_key(k));
        out_map
    }

    /// Reconstruct the "incoming" map the way WallpaperRefresh expects
    /// it — used by the shuffle-tick path to re-run the override flow
    /// without waiting for the next state.json change.
    fn last_wallpaper_map(&self) -> HashMap<String, String> {
        // Keyed by output name. We don't have direct access to the
        // compositor's wallpaper map from here, but the outputs list
        // is authoritative for "which outputs exist". An empty string
        // value preserves the same call signature; shuffle override
        // ignores values anyway.
        self.outputs
            .iter()
            .map(|(name, _, _)| (name.clone(), String::new()))
            .collect()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            Subscription::batch(self.modules_subscriptions(&self.general_config.modules.left)),
            Subscription::batch(self.modules_subscriptions(&self.general_config.modules.center)),
            Subscription::batch(self.modules_subscriptions(&self.general_config.modules.right)),
            config::subscription(&self.config_path),
            crate::services::logind::LogindService::subscribe().map(|event| match event {
                crate::services::ServiceEvent::Update(_) => Message::ResumeFromSleep,
                _ => Message::None,
            }),
            // Compositor wallpapers — relay state.json's per-output
            // wallpaper map into Message::WallpaperRefresh. The
            // workspaces module already subscribes to the same
            // CompositorService; iced fans the broadcaster out to
            // each subscriber so this is cheap.
            crate::services::compositor::CompositorService::subscribe().map(|event| {
                use crate::services::compositor::CompositorEvent;
                use crate::services::ServiceEvent;
                match event {
                    ServiceEvent::Init(svc) => Message::WallpaperRefresh {
                        wallpapers: svc.state.wallpapers.clone(),
                        active_tags: svc
                            .state
                            .monitors
                            .iter()
                            .map(|m| (m.name.clone(), m.active_workspace_id as u32))
                            .collect(),
                        active_output: svc.state.active_output.clone(),
                    },
                    ServiceEvent::Update(CompositorEvent::StateChanged(state)) => {
                        Message::WallpaperRefresh {
                            wallpapers: state.wallpapers.clone(),
                            active_tags: state
                                .monitors
                                .iter()
                                .map(|m| (m.name.clone(), m.active_workspace_id as u32))
                                .collect(),
                            active_output: state.active_output.clone(),
                        }
                    }
                    _ => Message::None,
                }
            }),
            // Wallpaper shuffle timer — only emits when both
            // [wallpaper.shuffle].enabled and interval_secs > 0.
            // `iced::time::every` returns an empty Subscription
            // for zero-duration; we still guard explicitly so the
            // pulse only fires when shuffle is actually active.
            if self.general_config.wallpaper.shuffle.enabled
                && self.general_config.wallpaper.shuffle.interval_secs > 0
            {
                iced::time::every(std::time::Duration::from_secs(
                    self.general_config.wallpaper.shuffle.interval_secs,
                ))
                .map(|_| Message::WallpaperShuffleTick)
            } else {
                Subscription::none()
            },
            iced::output_events().map(Message::OutputEvent),
            listen_with(move |evt, _, _| match evt {
                iced::event::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                    debug!("Keyboard event received: {key:?}");
                    if matches!(key, keyboard::Key::Named(keyboard::key::Named::Escape)) {
                        debug!("ESC key pressed, closing all menus");
                        Some(Message::CloseAllMenus)
                    } else {
                        None
                    }
                }
                _ => None,
            }),
            Subscription::run(|| {
                use iced::futures::StreamExt;
                signal_hook_tokio::Signals::new([libc::SIGUSR1])
                    .expect("Failed to create signal stream")
                    .filter_map(|sig| {
                        if sig == libc::SIGUSR1 {
                            iced::futures::future::ready(Some(Message::ToggleVisibility))
                        } else {
                            iced::futures::future::ready(None)
                        }
                    })
            }),
            // Always subscribe to audio/brightness services so OSD works
            // even when the Settings module isn't in the module list.
            self.settings.subscription().map(Message::Settings),
            crate::ipc::subscription().map(|cmd| match cmd {
                IpcCommand::ToggleVisibility => Message::ToggleVisibility,
                ref c if c.menu_name().is_some() => {
                    Message::OpenIpcMenu(c.menu_name().unwrap().to_string())
                }
                other => Message::IpcOsdCommand(other),
            }),
        ])
    }
}
