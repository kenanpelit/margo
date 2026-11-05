//! The config effects that drive the frame itself: the frame toggle and
//! inset, the side-menu expansion types and every menu's position.

use super::*;

/// Push the frame's own config effects onto `effects`.
pub(super) fn push_frame_effects(
    effects: &mut EffectScope,
    base_config: &reactive_stores::ArcStore<Config>,
    sender: &ComponentSender<Frame>,
) {
    let config = base_config.clone();
    let sender_clone = sender.clone();
    effects.push(move |_| {
        let config = config.clone();
        let enable_frame = config.clone().bars().frame().enable_frame().get();
        // Subscribe to the inset too, so editing `frameless_gap` re-fires
        // this effect → `SetDrawFrame` re-reads the fresh gap and re-pushes
        // the inset to the bars (live slider, no shell restart).
        let _ = config.bars().frame().frameless_gap().get();
        sender_clone.input(FrameInput::SetDrawFrame(enable_frame));
    });

    let config = base_config.clone();
    let sender_clone = sender.clone();
    effects.push(move |_| {
        let config = config.clone();
        let expansion_type = config.menus().left_menu_expansion_type().get();
        sender_clone.input(FrameInput::SetLeftMenuExpansionType(expansion_type));
    });

    let config = base_config.clone();
    let sender_clone = sender.clone();
    effects.push(move |_| {
        let config = config.clone();
        let expansion_type = config.menus().right_menu_expansion_type().get();
        sender_clone.input(FrameInput::SetRightMenuExpansionType(expansion_type));
    });

    let menu_config = base_config.clone();
    let sender_clone = sender.clone();
    effects.push(move |_| {
        // Each StoreField accessor (`.menus()`) consumes its receiver, so
        // every position read needs a fresh clone of the config handle.
        // `pos!` packages that clone-then-read so the per-menu lines stay
        // one-liners. Reading a menu's position also subscribes this effect
        // to that menu — which is why the side-placed menus below are read
        // into `_` (purely to re-fire when their position changes).
        // Collect every menu's position into one ordered Vec. The
        // `.get()` both reads the value AND subscribes this effect to
        // that menu (so a position change re-fires it), and the
        // collected snapshot lets `RepositionMenus` skip the
        // destructive restack when nothing moved (the config store is
        // coarse: any write re-fires this effect).
        let mut positions: Vec<Position> = Vec::new();
        macro_rules! pos {
            ($menu:ident) => {
                positions.push(menu_config.clone().menus().$menu().position().get())
            };
        }
        pos!(clock_menu);
        pos!(clipboard_menu);
        pos!(notification_menu);
        pos!(screenshot_menu);
        pos!(app_launcher_menu);
        pos!(wallpaper_menu);
        pos!(screenshare_menu);
        pos!(ufw_menu);
        pos!(dns_menu);
        pos!(podman_menu);
        pos!(github_menu);
        pos!(agents_menu);
        pos!(notes_menu);
        pos!(ip_menu);
        pos!(vpn_indicator_menu);
        pos!(network_menu);
        pos!(power_menu);
        pos!(media_player_menu);
        pos!(mtune_menu);
        pos!(lyrics_menu);
        pos!(session_menu);
        pos!(settings_menu);
        pos!(cpu_dashboard_menu);
        pos!(audio_dashboard_menu);
        pos!(mdash_menu);
        pos!(bluetooth_menu);
        pos!(system_update_menu);
        pos!(valent_menu);
        pos!(weather_menu);
        pos!(keep_awake_menu);
        pos!(twilight_menu);
        pos!(keybinds_menu);
        pos!(alarmclock_menu);
        pos!(dock_menu);
        pos!(control_center_menu);
        pos!(ssh_menu);
        pos!(usb_menu);
        pos!(discord_voice_menu);
        pos!(crashes_menu);
        pos!(vpn_menu);
        pos!(ai_menu);
        pos!(margo_layout_menu);
        sender_clone.input(FrameInput::RepositionMenus(positions));
    });
}
