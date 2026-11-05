//! `IPCCommand`: what the D-Bus service (`service.rs`) and the background
//! tasks send to the GTK-thread dispatch loop in `ipc.rs`.

use super::*;

/// One request for the GTK-thread dispatch loop. Each D-Bus method or
/// background task maps onto exactly one variant.
pub(super) enum IPCCommand {
    Quit,
    DockToggle,
    DockShow,
    DockHide,
    DockActivate(u32),
    AppLauncher,
    /// `mshellctl menu app-launcher --tab <name>` — open the
    /// launcher and pre-select the named category tab (canonical
    /// spelling; unknown names are rejected at the bus).
    AppLauncherTab(String),
    Clock,
    Clipboard,
    Notifications,
    NotificationsClearAll,
    NotificationsReadPopups,
    Session,
    SessionAction(SessionAction),
    /// `mshellctl session reboot-into <entry>`: set `BootNext` to this
    /// (already resolved) entry through the root helper, then reboot.
    SessionRebootInto(u16),
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
    /// Cycle the default output to the next routable sink, HDMI/DP skipped
    /// (`mshellctl audio route-next` — bind to a key).
    AudioRouteCycle,
    SystemUpdate,
    Valent,
    KeepAwake,
    /// Game Mode / Pomodoro / … — answered by `local::dispatch` with no UI
    /// round-trip.
    Local(LocalCommand),
    Twilight,
    MargoLayoutMenu,
    Weather,
    Keybinds,
    AlarmClock,
    ControlCenter,
    HiddenBar(mshell_common::hidden_bar::HiddenBarVerb, Option<String>),
    SshSessions,
    Usb,
    DiscordVoice,
    Crashes,
    Vpn,
    Ai,
    Dns,
    Translate,
    Podman,
    /// A uniform menu by id (the generic form newer menus use instead of a
    /// variant each): toggle it, or with `true` open it — re-shown when it
    /// is already open, so a request queued for it (`mshellctl qr`) lands.
    Menu(mshell_frame::frame::MenuId, bool),
    Notes,
    Ip,
    VpnIndicator,
    Network,
    Power,
    MediaPlayer,
    Mtune,
    Lyrics,
    Mdash,
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
    /// Headless clipboard op driven by `mshellctl clipboard …` (parsed from
    /// `"copy <id>" | "pin <id>" | "unpin <id>" | "delete <id>" | "clear" |
    /// "wipe"` in the bus method).
    ClipboardAction(ClipboardAction),
    /// `mshellctl clipboard list` — reply with one `id\tcategory\tpreview`
    /// line per entry (newest first).
    ClipboardList(tokio::sync::oneshot::Sender<String>),
    /// `mshellctl theme set <name>` — apply a colour scheme live, the
    /// same write Settings → Theme → Color Scheme performs. Runs on the
    /// GTK main thread (where the dispatch loop lives), so the reactive
    /// store + style manager pick it up exactly like the GUI path.
    ThemeSet(mshell_config::schema::themes::Themes),
    /// `mshellctl theme get` — reply with the active scheme's canonical name.
    ThemeGet(tokio::sync::oneshot::Sender<String>),
    /// `mshellctl theme bundle list` (`None`: reply with the list) /
    /// `apply <name>` (`Some`: apply it; reply `Err` for an unknown name).
    ThemeBundle(
        Option<String>,
        tokio::sync::oneshot::Sender<Result<String, String>>,
    ),
    OpenSettings,
    OpenWizard,
    CloseSettings,
    Inspect,
    BarToggleTop,
    BarToggleBottom,
    BarToggleAll(bool),
    BarRevealAll(bool),
    BarHideAll(bool),
}

/// Commands that drive shell services rather than a surface
/// (`ipc/local.rs`).
#[derive(Debug)]
pub(super) enum LocalCommand {
    /// `mshellctl gamemode on|off|toggle` — engage/release Game Mode (drops
    /// compositor effects, DND, idle inhibitor). `None` = toggle.
    GameMode(Option<bool>),
    /// `mshellctl gamemode status` — reply "on" / "off".
    GameModeStatus(tokio::sync::oneshot::Sender<String>),
    /// `mshellctl pomodoro <verb>` — drive the Pomodoro timer; reply with
    /// the resulting status line.
    Pomodoro(
        mshell_frame::pomodoro::Command,
        tokio::sync::oneshot::Sender<String>,
    ),
    /// `mshellctl eyecare <verb>` — drive the 20-20-20 reminder; reply with
    /// the resulting status line.
    EyeCare(
        mshell_frame::eyecare::Command,
        tokio::sync::oneshot::Sender<String>,
    ),
}
