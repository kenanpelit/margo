use crate::media_picker;
use crate::relm_app::{Shell, ShellInput};
use mshell_cache::wallpaper::set_wallpaper;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    AlarmConfigStoreFields, AudioConfigStoreFields, ConfigStoreFields, WallpaperStoreFields,
};
use mshell_osd::toast::{ToastEvent, ToastSeverity, push_toast};
use mshell_services::mpd::mpd_service;
use mshell_services::{
    audio_service, brightness_service, margo_service, media_service, notification_service, tokio_rt,
};
use mshell_session::session_lock::{lock_session, session_locked};
use mshell_settings::{close_settings, open_settings, open_wizard};
use mshell_sounds::{play_alarm_loop, play_audio_volume_change, stop_alarm};
use mshell_utils::session::SessionAction;
use reactive_graph::effect::Effect;
use reactive_graph::prelude::{Get, GetUntracked};
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
use zbus::connection;
use zbus::fdo;
use zbus::interface;

mod alarm;
mod audio;
mod daily_wallpaper;
mod media;
mod plugin_watch;
mod service;
use alarm::*;
use audio::*;
use daily_wallpaper::*;
use media::*;
use plugin_watch::*;
use service::*;

pub fn init_ipc_shell_service(sender: &ComponentSender<Shell>) {
    let (shell_tx, mut shell_rx) = mpsc::unbounded_channel();

    spawn_daily_wallpaper_task(shell_tx.clone());
    spawn_alarm_scheduler();
    spawn_plugin_watcher_task(shell_tx.clone());
    tokio::spawn(async move {
        if let Err(e) = start_shell_service(shell_tx).await {
            // Without the bus name every `mshellctl` call (keybinds, scripts)
            // fails — make it visible, not a silently dropped Result.
            tracing::error!(error = %e, "ipc: failed to start the com.mshell.Shell D-Bus service");
            push_toast(ToastEvent {
                icon: "dialog-error-symbolic".to_string(),
                title: "mshellctl unavailable".to_string(),
                body: Some(format!("D-Bus service failed to start: {e}")),
                severity: ToastSeverity::Danger,
            });
        }
    });

    let app_sender = sender.input_sender().clone();
    glib::spawn_future_local(async move {
        // Resolve the monitor a toggled menu opens on via
        // `MargoService::active_monitor_name`: margo's recency-based
        // `active_output` as mirrored into the reactive store by the
        // `watch state` subscription (a zero-I/O read), with a one-shot
        // socket read only before the first state push. We used to chain
        // through `active_workspace().monitor`, which could return None
        // right after boot and send menus to whichever Frame iterated
        // first.
        async fn active_monitor() -> Option<String> {
            margo_service().active_monitor_name().await
        }
        while let Some(cmd) = shell_rx.recv().await {
            match cmd {
                IPCCommand::Quit => app_sender.emit(ShellInput::Quit),
                IPCCommand::DockToggle => app_sender.emit(ShellInput::DockToggle),
                IPCCommand::DockShow => app_sender.emit(ShellInput::DockShow),
                IPCCommand::DockHide => app_sender.emit(ShellInput::DockHide),
                IPCCommand::DockActivate(n) => app_sender.emit(ShellInput::DockActivate(n)),
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
                IPCCommand::ClipboardAction(action) => {
                    // Handled directly against the clipboard service
                    // singleton (no UI round-trip needed). The verb→op parse
                    // (a pure, unit-tested function) ran in the bus method,
                    // so an unknown verb was already rejected to the caller.
                    let svc = mshell_clipboard::clipboard_service();
                    match action {
                        ClipboardAction::Copy(id) => svc.copy_entry(id),
                        ClipboardAction::TogglePin(id) => svc.toggle_pin(id),
                        ClipboardAction::Delete(id) => {
                            svc.history().remove(id);
                        }
                        ClipboardAction::Clear => svc.clear_unpinned(),
                        ClipboardAction::Wipe => svc.clear_history(),
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
                                mshell_clipboard::EntryPreview::ImagePending { mime_type } => {
                                    format!("[image {mime_type}]")
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
                    // No usable adapter (e.g. a VM): surface a toast here, where
                    // we can reach the toast surface — the services layer can't.
                    let usable = mshell_services::bluetooth_service()
                        .map(|bt| bt.available.get())
                        .unwrap_or(false);
                    if usable {
                        // Spawn so a 10–12s connect wait never blocks the IPC loop.
                        tokio_rt().spawn(async move {
                            if let Some(number) = action
                                .strip_prefix("connect:")
                                .and_then(|n| n.parse::<u8>().ok())
                            {
                                let mac = mshell_utils::audio_prefs::lock_prefs()
                                    .mac_for_bt_number(number);
                                if let Some(mac) = mac {
                                    mshell_services::bluetooth::connect_by_mac(&mac).await;
                                } else {
                                    push_toast(ToastEvent {
                                        icon: "bluetooth-disabled-symbolic".to_string(),
                                        title: "Bluetooth".to_string(),
                                        body: Some(format!("No device assigned to #{number}")),
                                        severity: ToastSeverity::Warn,
                                    });
                                }
                                return;
                            }
                            match action.as_str() {
                                "connect" => {
                                    mshell_services::bluetooth::connect_configured().await;
                                }
                                "disconnect" => {
                                    mshell_services::bluetooth::disconnect_configured().await;
                                }
                                // Validated by `parse_bluetooth_action`: the
                                // only remaining verb is `toggle`.
                                _ => mshell_services::bluetooth::toggle().await,
                            }
                        });
                    } else {
                        push_toast(ToastEvent {
                            icon: "bluetooth-disabled-symbolic".to_string(),
                            title: "Bluetooth".to_string(),
                            body: Some("No adapter".to_string()),
                            severity: ToastSeverity::Warn,
                        });
                    }
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
                IPCCommand::MargoLayoutMenu => {
                    app_sender.emit(ShellInput::ToggleMargoLayoutMenu(active_monitor().await));
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
                IPCCommand::HiddenBar(verb, target) => {
                    app_sender.emit(ShellInput::HiddenBar(verb, target));
                }
                IPCCommand::Crashes => {
                    app_sender.emit(ShellInput::ToggleCrashesMenu(active_monitor().await));
                }
                IPCCommand::SshSessions => {
                    app_sender.emit(ShellInput::ToggleSshSessionsMenu(active_monitor().await));
                }
                IPCCommand::Usb => {
                    app_sender.emit(ShellInput::ToggleUsbMenu(active_monitor().await));
                }
                IPCCommand::DiscordVoice => {
                    app_sender.emit(ShellInput::ToggleDiscordVoiceMenu(active_monitor().await));
                }
                IPCCommand::Vpn => {
                    app_sender.emit(ShellInput::ToggleVpnMenu(active_monitor().await));
                }
                IPCCommand::Ai => {
                    app_sender.emit(ShellInput::ToggleAiMenu(active_monitor().await));
                }
                IPCCommand::Dns => {
                    app_sender.emit(ShellInput::ToggleDnsMenu(active_monitor().await));
                }
                IPCCommand::Translate => {
                    app_sender.emit(ShellInput::ToggleTranslateMenu(active_monitor().await));
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
                IPCCommand::VpnIndicator => {
                    app_sender.emit(ShellInput::ToggleVpnIndicatorMenu(active_monitor().await));
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
                IPCCommand::Mtune => {
                    app_sender.emit(ShellInput::ToggleMtuneMenu(active_monitor().await));
                }
                IPCCommand::Lyrics => {
                    app_sender.emit(ShellInput::ToggleLyricsMenu(active_monitor().await));
                }
                IPCCommand::Session => {
                    app_sender.emit(ShellInput::ToggleSessionMenu(active_monitor().await));
                }
                IPCCommand::Mdash => {
                    app_sender.emit(ShellInput::ToggleMdashMenu(active_monitor().await));
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
                    if let Some(output) = audio_service().and_then(|a| a.default_output.get()) {
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
                    if let Some(output) = audio_service().and_then(|a| a.default_output.get()) {
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
                    if let Some(output) = audio_service().and_then(|a| a.default_output.get()) {
                        let _ = output.set_mute(!output.muted.get()).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MicMute => {
                    // Toggle the default *source* (microphone). The mic
                    // OSD watches `default_input.muted` and pops the
                    // bottom-centre pill on the change — no explicit
                    // show needed here, same as the volume path.
                    if let Some(input) = audio_service().and_then(|a| a.default_input.get()) {
                        let _ = input.set_mute(!input.muted.get()).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::VolumeSet(frac) => {
                    if let Some(output) = audio_service().and_then(|a| a.default_output.get()) {
                        let v = frac.clamp(0.0, 1.5);
                        let _ = output.set_volume(Volume::stereo(v, v)).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MuteSet(mode) => {
                    if let Some(output) = audio_service().and_then(|a| a.default_output.get()) {
                        let target = mute_target(mode, output.muted.get());
                        let _ = output.set_mute(target).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MicUp => {
                    if let Some(input) = audio_service().and_then(|a| a.default_input.get()) {
                        let nv = 1.0_f64.min(input.volume.get().average() + 0.05);
                        let _ = input.set_volume(Volume::stereo(nv, nv)).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MicDown => {
                    if let Some(input) = audio_service().and_then(|a| a.default_input.get()) {
                        let nv = 0.0_f64.max(input.volume.get().average() - 0.05);
                        let _ = input.set_volume(Volume::stereo(nv, nv)).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MicVolumeSet(frac) => {
                    if let Some(input) = audio_service().and_then(|a| a.default_input.get()) {
                        let v = frac.clamp(0.0, 1.5);
                        let _ = input.set_volume(Volume::stereo(v, v)).await;
                    }
                    play_audio_volume_change();
                }
                IPCCommand::MicMuteSet(mode) => {
                    if let Some(input) = audio_service().and_then(|a| a.default_input.get()) {
                        let target = mute_target(mode, input.muted.get());
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
                    let cur = audio_service()
                        .and_then(|a| a.default_output.get())
                        .map(|d| d.name.get());
                    if let Some(i) = pick_device(&names, cur.as_deref(), &target)
                        && devs[i].set_as_default().await.is_ok()
                    {
                        // Re-pin the configured default level on the device we
                        // just promoted (same opt-in as the startup restore).
                        // WirePlumber carries each device's own last volume, so
                        // without this a switch can land on e.g. a 100% mic
                        // even though Settings → Sound pins 50%.
                        let audio_cfg = config_manager().config().audio().get_untracked();
                        if audio_cfg.restore_volume_on_start {
                            let v = (audio_cfg.default_output_volume.clamp(0, 100) as f64) / 100.0;
                            let _ = devs[i].set_volume(Volume::stereo(v, v)).await;
                        }
                        // Follow whatever's currently playing to the new
                        // device — PipeWire's own default-sink change only
                        // affects streams opened afterwards.
                        mshell_utils::audio::migrate_playback_streams_to(&devs[i]).await;
                        // No notify-send here: the OSD toast producer already
                        // toasts "Audio output" on the default-device change.
                    }
                }
                IPCCommand::AudioRouteCycle => {
                    // Cycle to the next routable output (HDMI/DP skipped,
                    // name-sorted, wrapping) and — when configured — follow the
                    // mic across the headset boundary. Drives `mshellctl audio
                    // route-next` (bind it to a key); shares the same helpers as
                    // the Settings → Sound device switcher.
                    let outs = mshell_utils::audio::routable_outputs();
                    if outs.len() >= 2 {
                        let names: Vec<String> = outs.iter().map(|d| d.name.get()).collect();
                        let cur = audio_service()
                            .and_then(|a| a.default_output.get())
                            .map(|d| d.name.get());
                        let idx = mshell_utils::audio::next_index(&names, cur.as_deref());
                        let target = outs[idx].clone();
                        let to_headset = mshell_utils::audio::out_is_headset(&target);
                        if target.set_as_default().await.is_ok() {
                            // Re-pin the configured default level, same opt-in as
                            // SwitchOutput (WirePlumber remembers per-device volume).
                            let audio_cfg = config_manager().config().audio().get_untracked();
                            if audio_cfg.restore_volume_on_start {
                                let v =
                                    (audio_cfg.default_output_volume.clamp(0, 100) as f64) / 100.0;
                                let _ = target.set_volume(Volume::stereo(v, v)).await;
                            }
                            mshell_utils::audio::migrate_playback_streams_to(&target).await;
                            if audio_cfg.route_switch_microphone
                                && let Some(mic) =
                                    mshell_utils::audio::mic_follow_target(to_headset)
                            {
                                let _ = mic.set_as_default().await;
                            }
                        }
                    }
                }
                IPCCommand::SwitchInput(target) => {
                    let devs = usable_inputs();
                    let names: Vec<(String, String)> = devs
                        .iter()
                        .map(|d| (d.name.get(), d.description.get()))
                        .collect();
                    let cur = audio_service()
                        .and_then(|a| a.default_input.get())
                        .map(|d| d.name.get());
                    if let Some(i) = pick_device(&names, cur.as_deref(), &target)
                        && devs[i].set_as_default().await.is_ok()
                    {
                        // Same re-pin as SwitchOutput — this is the path where
                        // the "mic jumps to 100% after switch-mic" bug lived.
                        let audio_cfg = config_manager().config().audio().get_untracked();
                        if audio_cfg.restore_volume_on_start {
                            let v = (audio_cfg.default_input_volume.clamp(0, 100) as f64) / 100.0;
                            let _ = devs[i].set_volume(Volume::stereo(v, v)).await;
                        }
                        // No notify-send here: the OSD toast producer already
                        // toasts "Audio input" on the default-device change.
                    }
                }
                // MPRIS control on its own main-thread task, NOT inline:
                // a wedged player (a SIGSTOP'd app not answering D-Bus, up
                // to the ~25 s zbus timeout) would otherwise block this
                // serial loop and freeze every command queued behind it —
                // volume keys, menu toggles. `spawn_future_local` keeps the
                // same GTK-main thread, so pick_player / notify_media's
                // reactive reads stay sound; only the *ordering* dependency
                // is dropped, which is fine for fire-and-forget media keys.
                IPCCommand::MediaToggle(target) => {
                    glib::spawn_future_local(async move {
                        if let Some(p) = media_picker::pick_active(&target) {
                            media_picker::toggle(p).await;
                        }
                    });
                }
                IPCCommand::MediaNext(target) => {
                    glib::spawn_future_local(async move {
                        if let Some(p) = media_picker::pick_active(&target) {
                            media_picker::next(p).await;
                        }
                    });
                }
                IPCCommand::MediaPrev(target) => {
                    glib::spawn_future_local(async move {
                        if let Some(p) = media_picker::pick_active(&target) {
                            media_picker::previous(p).await;
                        }
                    });
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
                    // Global layout coordinates — grim takes those; the
                    // selector's own are output-local.
                    mshell_screenshot::select_region_geometry(move |geom| {
                        let _ = reply.send(geom);
                    });
                }
                IPCCommand::ThemeSet(theme) => {
                    // Identical to the GUI's ThemeSelected handler: write the
                    // scheme into the reactive config store and let the style
                    // manager re-theme. No `mctl reload`, no restart.
                    config_manager().update_config(|config| {
                        config.theme.theme = theme;
                    });
                }
                IPCCommand::ThemeGet(reply) => {
                    let current = config_manager()
                        .config()
                        .theme()
                        .get_untracked()
                        .theme
                        .ident()
                        .to_string();
                    let _ = reply.send(current);
                }
                IPCCommand::GameMode(target) => {
                    // Just flip the persisted `active` flag; the reactive
                    // reconcile effect (mshell-core `run`) does the apply, so
                    // the CLI, the Settings toggle, and startup share one path.
                    let active = config_manager().config().game_mode().get_untracked().active;
                    let target = target.unwrap_or(!active); // None = toggle
                    config_manager().update_config(|c| c.game_mode.active = target);
                }
                IPCCommand::GameModeStatus(reply) => {
                    let on = config_manager().config().game_mode().get_untracked().active;
                    let _ = reply.send(if on { "on" } else { "off" }.to_string());
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
/// order the launcher runtime exposes display categories. Kept here
/// — rather than asking the launcher runtime at IPC-query time
/// — because the runtime only exists while the launcher panel is
/// open; the wizard / CLI consumer wants the list any time
/// mshell is running.
///
/// If you add a new display category in `mshell-launcher::runtime`,
/// append it here so `mshellctl menu app-launcher --list-tabs` stays
/// accurate.
pub const APP_LAUNCHER_TABS: &[&str] = &["All", "Apps", "Actions", "Search", "Help"];

enum IPCCommand {
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
    /// `mshellctl gamemode on|off|toggle` — engage/release Game Mode (drops
    /// compositor effects, DND, idle inhibitor). `None` = toggle.
    GameMode(Option<bool>),
    /// `mshellctl gamemode status` — reply "on" / "off".
    GameModeStatus(tokio::sync::oneshot::Sender<String>),
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

#[cfg(test)]
mod tests {
    use super::{
        ClipboardAction, mute_target, parse_bluetooth_action, parse_clipboard_action,
        parse_cycle_direction, parse_game_mode, parse_launcher_tab, pick_device,
        validate_wallpaper_path,
    };
    use mshell_cache::wallpaper::CycleDirection;

    /// D-Bus type code for the Rust types the proxy trait uses.
    fn dbus_sig(rust: &str) -> &'static str {
        match rust.trim() {
            "&str" | "String" => "s",
            "u32" => "u",
            "i32" => "i",
            "f64" => "d",
            "bool" => "b",
            "Vec<String>" => "as",
            "()" => "",
            other => panic!("map Rust type `{other}` to a D-Bus signature"),
        }
    }

    fn pascal(snake: &str) -> String {
        snake
            .split('_')
            .map(|w| {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// The typed client (`mshell-ipc`'s `ShellProxy`, used by mshellctl and
    /// mshellshare) must describe exactly the methods this server exports,
    /// with the same in/out signatures — the old hand-typed method strings
    /// drifted silently.
    #[test]
    fn proxy_matches_server_interface() {
        use std::collections::BTreeMap;
        use zbus::object_server::Interface;

        // Server: introspection XML of the real interface.
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut xml = String::new();
        super::IPCService::new(tx).introspect_to_writer(&mut xml, 0);
        let mut server: BTreeMap<String, (String, String)> = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in xml.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("<method name=\"") {
                let name = rest.split('"').next().unwrap().to_string();
                server.insert(name.clone(), Default::default());
                current = (!line.ends_with("/>")).then_some(name);
            } else if line.starts_with("</method>") {
                current = None;
            } else if let (Some(m), true) = (&current, line.starts_with("<arg ")) {
                let attr = |k: &str| {
                    line.split(&format!("{k}=\""))
                        .nth(1)
                        .and_then(|r| r.split('"').next())
                        .unwrap_or("")
                        .to_string()
                };
                let entry = server.get_mut(m).unwrap();
                if attr("direction") == "out" {
                    entry.1 += &attr("type");
                } else {
                    entry.0 += &attr("type");
                }
            }
        }

        // Client: the proxy trait's declarations.
        let src = include_str!("../../mshell-ipc/src/lib.rs");
        let mut client: BTreeMap<String, (String, String)> = BTreeMap::new();
        for line in src.lines().map(str::trim) {
            let Some(rest) = line.strip_prefix("fn ") else {
                continue;
            };
            let Some(sig) = rest.strip_suffix(';') else {
                continue;
            };
            let (name, rest) = sig.split_once('(').unwrap();
            let (args, ret) = rest.split_once(") -> zbus::Result<").unwrap();
            let ret = ret.strip_suffix('>').unwrap();
            let ins: String = args
                .split(',')
                .skip(1) // &self
                .filter(|a| !a.trim().is_empty())
                .map(|a| dbus_sig(a.split_once(':').unwrap().1))
                .collect();
            client.insert(pascal(name), (ins, dbus_sig(ret).to_string()));
        }

        assert!(client.len() > 100, "parsed the proxy trait");
        for (name, sig) in &server {
            if name.starts_with("org.") {
                continue;
            }
            assert_eq!(
                client.get(name),
                Some(sig),
                "{name}: proxy (left) vs server (right)"
            );
        }
        for name in client.keys() {
            assert!(
                server.contains_key(name),
                "proxy method {name} has no server side"
            );
        }
    }

    #[test]
    fn game_mode_accepts_on_off_toggle_and_rejects_the_rest() {
        assert_eq!(parse_game_mode("on"), Ok(Some(true)));
        assert_eq!(parse_game_mode(" Enable "), Ok(Some(true)));
        assert_eq!(parse_game_mode("off"), Ok(Some(false)));
        assert_eq!(parse_game_mode("0"), Ok(Some(false)));
        assert_eq!(parse_game_mode("toggle"), Ok(None));
        // A typo used to silently toggle.
        assert!(parse_game_mode("of").is_err());
        assert!(parse_game_mode("").is_err());
    }

    #[test]
    fn cycle_direction_rejects_unknown() {
        assert_eq!(parse_cycle_direction("next"), Ok(CycleDirection::Next));
        assert_eq!(parse_cycle_direction("PREV"), Ok(CycleDirection::Previous));
        assert_eq!(
            parse_cycle_direction("previous"),
            Ok(CycleDirection::Previous)
        );
        assert_eq!(parse_cycle_direction("random"), Ok(CycleDirection::Random));
        assert!(parse_cycle_direction("sideways").is_err());
    }

    #[test]
    fn bluetooth_action_rejects_unknown() {
        for ok in ["toggle", "connect", "disconnect", "connect:3"] {
            assert_eq!(parse_bluetooth_action(ok), Ok(ok.to_string()), "{ok}");
        }
        assert!(parse_bluetooth_action("conect").is_err());
        assert!(parse_bluetooth_action("connect:x").is_err());
        assert!(parse_bluetooth_action("connect:999").is_err());
    }

    #[test]
    fn launcher_tab_is_canonicalised_or_rejected() {
        assert_eq!(parse_launcher_tab("apps"), Ok("Apps".to_string()));
        assert_eq!(parse_launcher_tab("All"), Ok("All".to_string()));
        assert!(parse_launcher_tab("Games").is_err());
    }

    #[test]
    fn wallpaper_path_must_be_an_existing_file() {
        let dir = std::env::temp_dir();
        assert!(validate_wallpaper_path("/nonexistent/wall.png").is_err());
        assert!(
            validate_wallpaper_path(dir.to_str().unwrap()).is_err(),
            "a directory is not a wallpaper"
        );
        assert!(validate_wallpaper_path("").is_err());
        let file = dir.join(format!("mshell-wall-test-{}.png", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            validate_wallpaper_path(file.to_str().unwrap()),
            Ok(file.clone())
        );
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn mute_target_encodes_off_on_and_toggle() {
        // 0 = unmute, regardless of current state.
        assert!(!mute_target(0, true));
        assert!(!mute_target(0, false));
        // 1 = mute, regardless of current state.
        assert!(mute_target(1, false));
        assert!(mute_target(1, true));
        // Anything else toggles the current state.
        assert!(mute_target(2, false));
        assert!(!mute_target(2, true));
        assert!(mute_target(-1, false));
    }

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
