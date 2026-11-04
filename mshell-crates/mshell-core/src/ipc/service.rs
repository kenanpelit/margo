//! The `com.mshell.Shell` D-Bus interface: validates each call and forwards it
//! to the GTK thread as an `IPCCommand`.

use super::*;

pub(super) struct IPCService {
    pub(super) tx: mpsc::UnboundedSender<IPCCommand>,
}

impl IPCService {
    pub fn new(tx: mpsc::UnboundedSender<IPCCommand>) -> Self {
        Self { tx }
    }

    /// Queue a command for the GTK-side dispatch loop. Fails only once that
    /// loop is gone (the shell is shutting down).
    pub(super) fn send(&self, cmd: IPCCommand) -> fdo::Result<()> {
        self.tx
            .send(cmd)
            .map_err(|_| fdo::Error::Failed("mshell is shutting down".into()))
    }
}

/// The dispatch loop dropped a reply channel without answering.
pub(super) fn no_reply() -> fdo::Error {
    fdo::Error::Failed("mshell dropped the request without replying".into())
}

pub(super) fn no_notification_service() -> fdo::Error {
    fdo::Error::Failed("notification service is not running".into())
}

/// CLI percent (0–150) → the fraction the audio setters take.
pub(super) fn volume_fraction(percent: f64) -> fdo::Result<f64> {
    if percent.is_finite() && (0.0..=150.0).contains(&percent) {
        Ok(percent / 100.0)
    } else {
        Err(fdo::Error::InvalidArgs(format!(
            "volume {percent} is out of range (0–150)"
        )))
    }
}

/// `mshellctl gamemode <action>` → the target state; `None` = toggle.
pub(super) fn parse_game_mode(action: &str) -> Result<Option<bool>, String> {
    match action.trim().to_ascii_lowercase().as_str() {
        "on" | "1" | "true" | "enable" => Ok(Some(true)),
        "off" | "0" | "false" | "disable" => Ok(Some(false)),
        "toggle" => Ok(None),
        other => Err(format!(
            "unknown game mode action '{other}' (on|off|toggle)"
        )),
    }
}

/// `mshellctl wallpaper next|previous|prev|random`.
pub(super) fn parse_cycle_direction(
    direction: &str,
) -> Result<mshell_cache::wallpaper::CycleDirection, String> {
    use mshell_cache::wallpaper::CycleDirection;
    match direction.trim().to_ascii_lowercase().as_str() {
        "next" => Ok(CycleDirection::Next),
        "previous" | "prev" => Ok(CycleDirection::Previous),
        "random" => Ok(CycleDirection::Random),
        other => Err(format!(
            "unknown wallpaper direction '{other}' (next|previous|random)"
        )),
    }
}

/// `mshellctl bluetooth …` verbs: `toggle` | `connect` | `disconnect` |
/// `connect:<n>` (a numbered device slot).
pub(super) fn parse_bluetooth_action(action: &str) -> Result<String, String> {
    let a = action.trim();
    let valid = matches!(a, "toggle" | "connect" | "disconnect")
        || a.strip_prefix("connect:")
            .is_some_and(|n| n.parse::<u8>().is_ok());
    if valid {
        Ok(a.to_string())
    } else {
        Err(format!(
            "unknown bluetooth action '{a}' (toggle|connect|disconnect|connect:<n>)"
        ))
    }
}

/// Match a launcher tab name case-insensitively against `APP_LAUNCHER_TABS`,
/// returning its canonical spelling (the launcher's lookup is exact).
pub(super) fn parse_launcher_tab(tab: &str) -> Result<String, String> {
    APP_LAUNCHER_TABS
        .iter()
        .find(|t| t.eq_ignore_ascii_case(tab.trim()))
        .map(|t| (*t).to_string())
        .ok_or_else(|| {
            format!(
                "unknown launcher tab '{tab}' ({})",
                APP_LAUNCHER_TABS.join("|")
            )
        })
}

/// A wallpaper path must name an existing regular file.
pub(super) fn validate_wallpaper_path(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if p.is_file() {
        Ok(p)
    } else {
        Err(format!("'{path}' is not an existing file"))
    }
}

#[interface(name = "com.mshell.Shell")]
impl IPCService {
    async fn quit(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Quit)
    }
    /// `mshellctl toast <title> [body] [--icon NAME] [--severity …]` — show a
    /// transient state-change toast. The `notify-send` equivalent for the
    /// toast surface: ephemeral (no notification history), usable from scripts
    /// and startup services. `push_toast` only fans out to the per-output
    /// surfaces (a thread-safe broadcast), so this is safe to call straight
    /// from the bus task. Explicit intent — it bypasses the per-event toggles.
    async fn toast(
        &self,
        title: String,
        body: String,
        icon: String,
        severity: String,
    ) -> fdo::Result<()> {
        let severity: ToastSeverity = severity.parse().map_err(fdo::Error::InvalidArgs)?;
        if title.trim().is_empty() {
            return Err(fdo::Error::InvalidArgs("toast title is empty".into()));
        }
        let icon = if icon.trim().is_empty() {
            "dialog-information-symbolic".to_string()
        } else {
            icon
        };
        push_toast(ToastEvent {
            icon,
            title,
            body: (!body.trim().is_empty()).then_some(body),
            severity,
        });
        Ok(())
    }
    /// `mshellctl gamemode on|off|toggle` — engage/release Game Mode.
    async fn game_mode(&self, action: String) -> fdo::Result<()> {
        let target = parse_game_mode(&action).map_err(fdo::Error::InvalidArgs)?;
        self.send(IPCCommand::GameMode(target))
    }
    /// `mshellctl gamemode status` — "on" / "off".
    async fn game_mode_status(&self) -> fdo::Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.send(IPCCommand::GameModeStatus(tx))?;
        rx.await.map_err(|_| no_reply())
    }
    /// Standalone mdock surface controls.
    async fn dock_toggle(&self) -> fdo::Result<()> {
        self.send(IPCCommand::DockToggle)
    }
    async fn dock_show(&self) -> fdo::Result<()> {
        self.send(IPCCommand::DockShow)
    }
    async fn dock_hide(&self) -> fdo::Result<()> {
        self.send(IPCCommand::DockHide)
    }
    /// Activate the Nth (1-based) pinned dock app — focus or launch.
    async fn dock_activate(&self, index: u32) -> fdo::Result<()> {
        if index == 0 {
            return Err(fdo::Error::InvalidArgs("dock index is 1-based".into()));
        }
        self.send(IPCCommand::DockActivate(index))
    }
    async fn app_launcher(&self) -> fdo::Result<()> {
        self.send(IPCCommand::AppLauncher)
    }
    /// Open (or refocus) the app launcher and pre-select the
    /// named category tab (case-insensitive; see `APP_LAUNCHER_TABS`).
    /// An unknown name is rejected.
    async fn app_launcher_tab(&self, tab: String) -> fdo::Result<()> {
        let tab = parse_launcher_tab(&tab).map_err(fdo::Error::InvalidArgs)?;
        self.send(IPCCommand::AppLauncherTab(tab))
    }
    /// Return the known launcher category tab names so the CLI
    /// can offer `--list-tabs` without round-tripping through
    /// the live runtime (which only exists while the panel is
    /// open). See `APP_LAUNCHER_TABS` for the source list.
    async fn list_app_launcher_tabs(&self) -> Vec<String> {
        APP_LAUNCHER_TABS.iter().map(|s| (*s).to_string()).collect()
    }
    async fn clock(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Clock)
    }
    async fn clipboard(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Clipboard)
    }
    /// Headless clipboard op for `mshellctl clipboard copy|pin|unpin|delete|
    /// clear|wipe`. `spec` is `"<verb> [id]"`.
    async fn clipboard_action(&self, spec: String) -> fdo::Result<()> {
        let action = parse_clipboard_action(&spec).ok_or_else(|| {
            fdo::Error::InvalidArgs(format!(
                "unknown clipboard action '{spec}' (copy|pin|unpin|delete <id>, clear, wipe)"
            ))
        })?;
        self.send(IPCCommand::ClipboardAction(action))
    }
    /// `mshellctl clipboard list` — `id\tcategory\tpreview` per line.
    async fn clipboard_list(&self) -> fdo::Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.send(IPCCommand::ClipboardList(tx))?;
        rx.await.map_err(|_| no_reply())
    }
    /// `mshellctl theme list` — one `name\tlabel` line per built-in colour
    /// scheme. Static (sourced from `Themes::all()`), so it answers without
    /// bouncing through the command loop.
    async fn theme_list(&self) -> String {
        use mshell_config::schema::themes::Themes;
        Themes::all()
            .iter()
            .map(|t| format!("{}\t{}", t.ident(), t.label()))
            .collect::<Vec<_>>()
            .join("\n")
    }
    /// `mshellctl theme get` — the active scheme's canonical name.
    async fn theme_get(&self) -> fdo::Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.send(IPCCommand::ThemeGet(tx))?;
        rx.await.map_err(|_| no_reply())
    }
    /// `mshellctl theme set <name>` — switch the colour scheme live. An
    /// unknown name is an `InvalidArgs` error, so the CLI fails loudly.
    async fn theme_set(&self, name: String) -> fdo::Result<()> {
        use mshell_config::schema::themes::Themes;
        let theme = Themes::from_cli(&name).ok_or_else(|| {
            fdo::Error::InvalidArgs(format!(
                "unknown theme '{name}' — run `mshellctl theme list`"
            ))
        })?;
        self.send(IPCCommand::ThemeSet(theme))
    }
    /// `mshellctl doctor` — the running shell's version (the workspace
    /// version baked in at build time). Lets the doctor flag a shell that
    /// is older than the `mshellctl` talking to it (restart needed).
    async fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
    async fn notifications(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Notifications)
    }
    async fn notifications_clear_all(&self) -> fdo::Result<()> {
        self.send(IPCCommand::NotificationsClearAll)
    }
    async fn notifications_read_popups(&self) -> fdo::Result<()> {
        self.send(IPCCommand::NotificationsReadPopups)
    }
    /// Do Not Disturb — set/clear/toggle directly on the global
    /// notification service (the bar pill + popups subscribe to it).
    async fn notification_dnd_on(&self) -> fdo::Result<()> {
        notification_service()
            .ok_or_else(no_notification_service)?
            .set_dnd(true);
        Ok(())
    }
    async fn notification_dnd_off(&self) -> fdo::Result<()> {
        notification_service()
            .ok_or_else(no_notification_service)?
            .set_dnd(false);
        Ok(())
    }
    async fn notification_dnd_toggle(&self) -> fdo::Result<()> {
        let service = notification_service().ok_or_else(no_notification_service)?;
        service.set_dnd(!service.dnd.get());
        Ok(())
    }
    /// Number of notifications currently in history (for bars / scripts).
    async fn notification_count(&self) -> fdo::Result<u32> {
        Ok(notification_service()
            .ok_or_else(no_notification_service)?
            .notifications
            .get()
            .len() as u32)
    }
    async fn screenshot(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Screenshot)
    }
    /// Headless capture driven by `mshellctl screenshot <area>`. `spec` is
    /// `"<area> <target> <delay>"`.
    async fn screenshot_capture(&self, spec: String) -> fdo::Result<()> {
        self.send(IPCCommand::ScreenshotCapture(spec))
    }
    /// Screen recording driven by `mshellctl screenrecord …`. `spec` is
    /// `"<action> <area> <audio>"`.
    async fn screen_record(&self, spec: String) -> fdo::Result<()> {
        self.send(IPCCommand::ScreenRecord(spec))
    }
    async fn wallpaper(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Wallpaper)
    }
    async fn ufw(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Ufw)
    }
    async fn privacy(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Privacy)
    }
    async fn bluetooth(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Bluetooth)
    }
    /// Native auto-connect engine: `toggle` | `connect` | `disconnect`.
    async fn bluetooth_ctl(&self, action: String) -> fdo::Result<()> {
        let action = parse_bluetooth_action(&action).map_err(fdo::Error::InvalidArgs)?;
        self.send(IPCCommand::BluetoothCtl(action))
    }
    async fn cpu_dashboard(&self) -> fdo::Result<()> {
        self.send(IPCCommand::CpuDashboard)
    }
    async fn audio_dashboard(&self) -> fdo::Result<()> {
        self.send(IPCCommand::AudioDashboard)
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
    async fn log_level(&self, level: String) -> fdo::Result<String> {
        mshell_logging::set_level(&level)
            .map(|()| format!("shell log level set to {level}"))
            .map_err(|e| fdo::Error::InvalidArgs(e.to_string()))
    }
    /// Enable/disable the shell's file logging live.
    async fn log_enabled(&self, enabled: bool) -> fdo::Result<String> {
        mshell_logging::set_enabled(enabled)
            .map(|()| {
                format!(
                    "shell file logging {}",
                    if enabled { "enabled" } else { "disabled" }
                )
            })
            .map_err(|e| fdo::Error::Failed(e.to_string()))
    }
    /// Absolute output volume as a percent (0–150).
    async fn audio_volume_set(&self, percent: f64) -> fdo::Result<()> {
        self.send(IPCCommand::VolumeSet(volume_fraction(percent)?))
    }
    /// Output mute: 0 = off, 1 = on, else toggle.
    async fn audio_mute_set(&self, mode: i32) -> fdo::Result<()> {
        self.send(IPCCommand::MuteSet(mode))
    }
    async fn audio_mic_volume_set(&self, percent: f64) -> fdo::Result<()> {
        self.send(IPCCommand::MicVolumeSet(volume_fraction(percent)?))
    }
    async fn audio_mic_mute_set(&self, mode: i32) -> fdo::Result<()> {
        self.send(IPCCommand::MicMuteSet(mode))
    }
    async fn audio_mic_up(&self) -> fdo::Result<()> {
        self.send(IPCCommand::MicUp)
    }
    async fn audio_mic_down(&self) -> fdo::Result<()> {
        self.send(IPCCommand::MicDown)
    }
    /// Switch the default output: "next" | "prev" | index | name fragment.
    async fn audio_output_switch(&self, target: String) -> fdo::Result<()> {
        self.send(IPCCommand::SwitchOutput(target))
    }
    /// Cycle the default output to the next routable sink: skips HDMI/DP,
    /// name-sorted + wrapping, and follows the mic across the headset boundary
    /// when `audio.route_switch_microphone` is on.
    async fn audio_route_cycle(&self) -> fdo::Result<()> {
        self.send(IPCCommand::AudioRouteCycle)
    }
    async fn audio_input_switch(&self, target: String) -> fdo::Result<()> {
        self.send(IPCCommand::SwitchInput(target))
    }
    // ── Media (mshellctl media …) — empty target = the active player ───────
    async fn media_toggle(&self, target: String) -> fdo::Result<()> {
        self.send(IPCCommand::MediaToggle(target))
    }
    async fn media_next(&self, target: String) -> fdo::Result<()> {
        self.send(IPCCommand::MediaNext(target))
    }
    async fn media_prev(&self, target: String) -> fdo::Result<()> {
        self.send(IPCCommand::MediaPrev(target))
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
    async fn system_update(&self) -> fdo::Result<()> {
        self.send(IPCCommand::SystemUpdate)
    }
    async fn valent(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Valent)
    }
    async fn keep_awake(&self) -> fdo::Result<()> {
        self.send(IPCCommand::KeepAwake)
    }
    async fn twilight(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Twilight)
    }
    async fn margo_layout_menu(&self) -> fdo::Result<()> {
        self.send(IPCCommand::MargoLayoutMenu)
    }
    async fn weather(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Weather)
    }
    async fn keybinds(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Keybinds)
    }
    async fn alarm_clock(&self) -> fdo::Result<()> {
        self.send(IPCCommand::AlarmClock)
    }
    async fn control_center(&self) -> fdo::Result<()> {
        self.send(IPCCommand::ControlCenter)
    }
    /// Control a Hidden Bar drawer: `toggle` / `expand` / `collapse` /
    /// `pin` / `unpin`. `name` targets a single named drawer; an empty
    /// `name` reaches every drawer. An unknown action is rejected.
    async fn hidden_bar(&self, action: String, name: String) -> fdo::Result<()> {
        let verb =
            mshell_common::hidden_bar::HiddenBarVerb::from_action(&action).ok_or_else(|| {
                fdo::Error::InvalidArgs(format!(
                    "unknown hidden-bar action '{action}' (toggle|expand|collapse|pin|unpin)"
                ))
            })?;
        let target = if name.trim().is_empty() {
            None
        } else {
            Some(name)
        };
        self.send(IPCCommand::HiddenBar(verb, target))
    }
    async fn ssh_sessions(&self) -> fdo::Result<()> {
        self.send(IPCCommand::SshSessions)
    }
    async fn usb(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Usb)
    }
    async fn discord_voice(&self) -> fdo::Result<()> {
        self.send(IPCCommand::DiscordVoice)
    }
    async fn vpn(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Vpn)
    }
    async fn ai(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Ai)
    }
    async fn dns(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Dns)
    }
    async fn translate_menu(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Translate)
    }
    async fn podman(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Podman)
    }
    async fn notes(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Notes)
    }
    async fn ip(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Ip)
    }
    async fn vpn_indicator(&self) -> fdo::Result<()> {
        self.send(IPCCommand::VpnIndicator)
    }
    async fn network(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Network)
    }
    async fn power(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Power)
    }
    async fn media_player(&self) -> fdo::Result<()> {
        self.send(IPCCommand::MediaPlayer)
    }
    async fn mtune(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Mtune)
    }
    async fn lyrics(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Lyrics)
    }
    async fn session(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Session)
    }
    async fn mdash(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Mdash)
    }
    async fn plugin_reload(&self, key: String) -> fdo::Result<()> {
        self.send(IPCCommand::PluginReload(key))
    }

    async fn plugin_keybind(&self, arg: String) -> fdo::Result<()> {
        let (key, id) = arg.split_once('|').unwrap_or((arg.as_str(), ""));
        self.send(IPCCommand::PluginKeybind(key.to_string(), id.to_string()))
    }

    async fn plugin_menu(&self, key: String) -> fdo::Result<()> {
        self.send(IPCCommand::PluginMenu(key))
    }
    async fn session_lock(&self) -> fdo::Result<()> {
        self.send(IPCCommand::SessionAction(SessionAction::Lock))
    }
    async fn session_logout(&self) -> fdo::Result<()> {
        self.send(IPCCommand::SessionAction(SessionAction::Logout))
    }
    async fn session_suspend(&self) -> fdo::Result<()> {
        self.send(IPCCommand::SessionAction(SessionAction::Suspend))
    }
    async fn session_reboot(&self) -> fdo::Result<()> {
        self.send(IPCCommand::SessionAction(SessionAction::Reboot))
    }
    async fn session_shutdown(&self) -> fdo::Result<()> {
        self.send(IPCCommand::SessionAction(SessionAction::Shutdown))
    }
    async fn close_all_menus(&self) -> fdo::Result<()> {
        self.send(IPCCommand::CloseAllMenus)
    }
    async fn volume_up(&self) -> fdo::Result<()> {
        self.send(IPCCommand::VolumeUp)
    }
    async fn volume_down(&self) -> fdo::Result<()> {
        self.send(IPCCommand::VolumeDown)
    }
    async fn mute(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Mute)
    }
    async fn mic_mute(&self) -> fdo::Result<()> {
        self.send(IPCCommand::MicMute)
    }
    async fn brightness_up(&self) -> fdo::Result<()> {
        self.send(IPCCommand::BrightnessUp)
    }
    async fn brightness_down(&self) -> fdo::Result<()> {
        self.send(IPCCommand::BrightnessDown)
    }
    /// `path` must name an existing file (relative paths resolve against
    /// the shell's cwd, so the CLI sends an absolute one).
    async fn set_wallpaper(&self, path: &str) -> fdo::Result<()> {
        let path = validate_wallpaper_path(path).map_err(fdo::Error::InvalidArgs)?;
        self.send(IPCCommand::SetWallpaper(path))
    }
    /// Step the wallpaper: `direction` is `next`, `previous` /
    /// `prev`, or `random`.
    async fn wallpaper_cycle(&self, direction: &str) -> fdo::Result<()> {
        let dir = parse_cycle_direction(direction).map_err(fdo::Error::InvalidArgs)?;
        self.send(IPCCommand::WallpaperCycle(dir))
    }
    async fn lock(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Lock)
    }
    async fn check_lock(&self) -> fdo::Result<bool> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.send(IPCCommand::CheckLock(tx))?;
        rx.await.map_err(|_| no_reply())
    }
    async fn screenshare(&self, payload: &str) -> fdo::Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.send(IPCCommand::Screenshare(tx, payload.to_string()))?;
        rx.await.map_err(|_| no_reply())
    }
    /// Open the in-shell area selector and block until the user
    /// commits a rect (or cancels). Returns the geometry in slurp
    /// format `"X,Y WxH"` on commit, empty string on cancel. Used
    /// by `mscreenshot` CLI's region capture path so the rich
    /// in-shell selector replaces `slurp` when mshell is running.
    async fn select_region(&self) -> fdo::Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.send(IPCCommand::SelectRegion(tx))?;
        rx.await.map_err(|_| no_reply())
    }
    async fn open_settings(&self) -> fdo::Result<()> {
        self.send(IPCCommand::OpenSettings)
    }
    async fn open_wizard(&self) -> fdo::Result<()> {
        self.send(IPCCommand::OpenWizard)
    }
    async fn close_settings(&self) -> fdo::Result<()> {
        self.send(IPCCommand::CloseSettings)
    }
    async fn inspect(&self) -> fdo::Result<()> {
        self.send(IPCCommand::Inspect)
    }
    async fn bar_toggle_top(&self) -> fdo::Result<()> {
        self.send(IPCCommand::BarToggleTop)
    }
    async fn bar_toggle_bottom(&self) -> fdo::Result<()> {
        self.send(IPCCommand::BarToggleBottom)
    }
    async fn bar_toggle_all(&self, exclude_hidden_by_default: bool) -> fdo::Result<()> {
        self.send(IPCCommand::BarToggleAll(exclude_hidden_by_default))
    }
    async fn bar_reveal_all(&self, exclude_hidden_by_default: bool) -> fdo::Result<()> {
        self.send(IPCCommand::BarRevealAll(exclude_hidden_by_default))
    }
    async fn bar_hide_all(&self, exclude_hidden_by_default: bool) -> fdo::Result<()> {
        self.send(IPCCommand::BarHideAll(exclude_hidden_by_default))
    }
}

pub(super) async fn start_shell_service(tx: mpsc::UnboundedSender<IPCCommand>) -> zbus::Result<()> {
    let service = IPCService::new(tx);
    let _connection = connection::Builder::session()?
        .name("com.mshell.Shell")?
        .serve_at("/com/mshell/Shell", service)?
        .build()
        .await?;
    std::future::pending::<()>().await;
    Ok(())
}
