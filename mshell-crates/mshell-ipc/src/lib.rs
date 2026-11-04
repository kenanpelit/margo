//! Typed client for mshell's `com.mshell.Shell` D-Bus interface.
//!
//! The server is `IPCService` in `mshell-core/src/ipc.rs`; `mshellctl` and
//! `mshellshare` talk to it through [`ShellProxy`] instead of hand-typed
//! method-name strings, so a renamed or re-typed method is a compile error on
//! the client side. `mshell-core`'s `proxy_matches_server_interface` test
//! checks the two stay in sync (names and signatures).
//!
//! Methods return a D-Bus error (`zbus::Error::MethodError`) when the shell
//! rejects an argument (an unknown verb, a missing file) — callers propagate
//! it so the CLI exits non-zero.

/// Well-known bus name the shell owns.
pub const BUS_NAME: &str = "com.mshell.Shell";
/// Object path of the shell interface.
pub const OBJECT_PATH: &str = "/com/mshell/Shell";

#[zbus::proxy(
    interface = "com.mshell.Shell",
    default_service = "com.mshell.Shell",
    default_path = "/com/mshell/Shell"
)]
pub trait Shell {
    // Per-method docs live on the server (`IPCService` in mshell-core).
    fn quit(&self) -> zbus::Result<()>;
    fn toast(&self, title: &str, body: &str, icon: &str, severity: &str) -> zbus::Result<()>;
    fn game_mode(&self, action: &str) -> zbus::Result<()>;
    fn game_mode_status(&self) -> zbus::Result<String>;
    fn dock_toggle(&self) -> zbus::Result<()>;
    fn dock_show(&self) -> zbus::Result<()>;
    fn dock_hide(&self) -> zbus::Result<()>;
    fn dock_activate(&self, index: u32) -> zbus::Result<()>;
    fn app_launcher(&self) -> zbus::Result<()>;
    fn app_launcher_tab(&self, tab: &str) -> zbus::Result<()>;
    fn list_app_launcher_tabs(&self) -> zbus::Result<Vec<String>>;
    fn clock(&self) -> zbus::Result<()>;
    fn clipboard(&self) -> zbus::Result<()>;
    fn clipboard_action(&self, spec: &str) -> zbus::Result<()>;
    fn clipboard_list(&self) -> zbus::Result<String>;
    fn theme_list(&self) -> zbus::Result<String>;
    fn theme_get(&self) -> zbus::Result<String>;
    fn theme_set(&self, name: &str) -> zbus::Result<()>;
    fn version(&self) -> zbus::Result<String>;
    fn notifications(&self) -> zbus::Result<()>;
    fn notifications_clear_all(&self) -> zbus::Result<()>;
    fn notifications_read_popups(&self) -> zbus::Result<()>;
    fn notification_dnd_on(&self) -> zbus::Result<()>;
    fn notification_dnd_off(&self) -> zbus::Result<()>;
    fn notification_dnd_toggle(&self) -> zbus::Result<()>;
    fn notification_count(&self) -> zbus::Result<u32>;
    fn screenshot(&self) -> zbus::Result<()>;
    fn screenshot_capture(&self, spec: &str) -> zbus::Result<()>;
    fn screen_record(&self, spec: &str) -> zbus::Result<()>;
    fn wallpaper(&self) -> zbus::Result<()>;
    fn ufw(&self) -> zbus::Result<()>;
    fn privacy(&self) -> zbus::Result<()>;
    fn bluetooth(&self) -> zbus::Result<()>;
    fn bluetooth_ctl(&self, action: &str) -> zbus::Result<()>;
    fn cpu_dashboard(&self) -> zbus::Result<()>;
    fn audio_dashboard(&self) -> zbus::Result<()>;
    fn audio_list_text(&self) -> zbus::Result<String>;
    fn audio_list_json(&self) -> zbus::Result<String>;
    fn audio_status_text(&self) -> zbus::Result<String>;
    fn audio_status_json(&self) -> zbus::Result<String>;
    fn log_level(&self, level: &str) -> zbus::Result<String>;
    fn log_enabled(&self, enabled: bool) -> zbus::Result<String>;
    fn audio_volume_set(&self, percent: f64) -> zbus::Result<()>;
    fn audio_mute_set(&self, mode: i32) -> zbus::Result<()>;
    fn audio_mic_volume_set(&self, percent: f64) -> zbus::Result<()>;
    fn audio_mic_mute_set(&self, mode: i32) -> zbus::Result<()>;
    fn audio_mic_up(&self) -> zbus::Result<()>;
    fn audio_mic_down(&self) -> zbus::Result<()>;
    fn audio_output_switch(&self, target: &str) -> zbus::Result<()>;
    fn audio_route_cycle(&self) -> zbus::Result<()>;
    fn audio_input_switch(&self, target: &str) -> zbus::Result<()>;
    fn media_toggle(&self, target: &str) -> zbus::Result<()>;
    fn media_next(&self, target: &str) -> zbus::Result<()>;
    fn media_prev(&self, target: &str) -> zbus::Result<()>;
    fn media_list_text(&self) -> zbus::Result<String>;
    fn media_list_json(&self) -> zbus::Result<String>;
    fn media_status_text(&self) -> zbus::Result<String>;
    fn media_status_json(&self) -> zbus::Result<String>;
    fn system_update(&self) -> zbus::Result<()>;
    fn valent(&self) -> zbus::Result<()>;
    fn keep_awake(&self) -> zbus::Result<()>;
    fn twilight(&self) -> zbus::Result<()>;
    fn margo_layout_menu(&self) -> zbus::Result<()>;
    fn weather(&self) -> zbus::Result<()>;
    fn keybinds(&self) -> zbus::Result<()>;
    fn alarm_clock(&self) -> zbus::Result<()>;
    fn control_center(&self) -> zbus::Result<()>;
    fn hidden_bar(&self, action: &str, name: &str) -> zbus::Result<()>;
    fn custom_refresh(&self, name: &str) -> zbus::Result<u32>;
    fn ssh_sessions(&self) -> zbus::Result<()>;
    fn usb(&self) -> zbus::Result<()>;
    fn discord_voice(&self) -> zbus::Result<()>;
    fn crashes(&self) -> zbus::Result<()>;
    fn vpn(&self) -> zbus::Result<()>;
    fn ai(&self) -> zbus::Result<()>;
    fn dns(&self) -> zbus::Result<()>;
    fn translate_menu(&self) -> zbus::Result<()>;
    fn podman(&self) -> zbus::Result<()>;
    fn github(&self) -> zbus::Result<()>;
    fn agents(&self) -> zbus::Result<()>;
    fn notes(&self) -> zbus::Result<()>;
    fn ip(&self) -> zbus::Result<()>;
    fn vpn_indicator(&self) -> zbus::Result<()>;
    fn network(&self) -> zbus::Result<()>;
    fn power(&self) -> zbus::Result<()>;
    fn media_player(&self) -> zbus::Result<()>;
    fn mtune(&self) -> zbus::Result<()>;
    fn lyrics(&self) -> zbus::Result<()>;
    fn session(&self) -> zbus::Result<()>;
    fn mdash(&self) -> zbus::Result<()>;
    fn plugin_reload(&self, key: &str) -> zbus::Result<()>;
    fn plugin_keybind(&self, arg: &str) -> zbus::Result<()>;
    fn plugin_menu(&self, key: &str) -> zbus::Result<()>;
    fn session_lock(&self) -> zbus::Result<()>;
    fn session_logout(&self) -> zbus::Result<()>;
    fn session_suspend(&self) -> zbus::Result<()>;
    fn session_reboot(&self) -> zbus::Result<()>;
    fn session_shutdown(&self) -> zbus::Result<()>;
    fn close_all_menus(&self) -> zbus::Result<()>;
    fn volume_up(&self) -> zbus::Result<()>;
    fn volume_down(&self) -> zbus::Result<()>;
    fn mute(&self) -> zbus::Result<()>;
    fn mic_mute(&self) -> zbus::Result<()>;
    fn brightness_up(&self) -> zbus::Result<()>;
    fn brightness_down(&self) -> zbus::Result<()>;
    fn set_wallpaper(&self, path: &str) -> zbus::Result<()>;
    fn wallpaper_cycle(&self, direction: &str) -> zbus::Result<()>;
    fn lock(&self) -> zbus::Result<()>;
    fn check_lock(&self) -> zbus::Result<bool>;
    fn screenshare(&self, payload: &str) -> zbus::Result<String>;
    fn select_region(&self) -> zbus::Result<String>;
    fn open_settings(&self) -> zbus::Result<()>;
    fn open_wizard(&self) -> zbus::Result<()>;
    fn close_settings(&self) -> zbus::Result<()>;
    fn inspect(&self) -> zbus::Result<()>;
    fn bar_toggle_top(&self) -> zbus::Result<()>;
    fn bar_toggle_bottom(&self) -> zbus::Result<()>;
    fn bar_toggle_all(&self, exclude_hidden_by_default: bool) -> zbus::Result<()>;
    fn bar_reveal_all(&self, exclude_hidden_by_default: bool) -> zbus::Result<()>;
    fn bar_hide_all(&self, exclude_hidden_by_default: bool) -> zbus::Result<()>;
}

/// Connect to the session bus and build a proxy for the running shell.
pub async fn connect() -> zbus::Result<ShellProxy<'static>> {
    let connection = zbus::Connection::session().await?;
    ShellProxy::new(&connection).await
}
