//! Where each menu is anchored: the one table mapping a menu's stack name to
//! its `menus.<field>.position` config key.
//!
//! Both readers derive from [`positioned_menus!`]: [`menu_position`] (where a
//! menu is attached on first build and on every restack) and the frame's
//! position effect (which re-fires the restack when a position changes). They
//! used to be two hand-kept lists, and menus missing from one of them ignored
//! their configured position (Translate) or only moved after an unrelated
//! menu was repositioned (Privacy, Translate, the plugin panel).

use super::*;

/// Invoke `$callback! { NAME_CONST => menus_field, ... }` with every menu that
/// has a configured position. The wizard is not listed: it shares the
/// Settings slot (see [`menu_position_in`]).
macro_rules! positioned_menus {
    ($callback:ident) => {
        $callback! {
            CLOCK_MENU => clock_menu,
            CLIPBOARD_MENU => clipboard_menu,
            NOTIFICATION_MENU => notification_menu,
            SCREENSHOT_MENU => screenshot_menu,
            APP_LAUNCHER_MENU => app_launcher_menu,
            WALLPAPER_MENU => wallpaper_menu,
            SCREENSHARE_MENU => screenshare_menu,
            SETTINGS_MENU => settings_menu,
            NUFW_MENU => ufw_menu,
            PRIVACY_MENU => privacy_menu,
            BLUETOOTH_MENU => bluetooth_menu,
            CPU_DASHBOARD_MENU => cpu_dashboard_menu,
            AUDIO_DASHBOARD_MENU => audio_dashboard_menu,
            SYSTEM_UPDATE_MENU => system_update_menu,
            VALENT_MENU => valent_menu,
            WEATHER_MENU => weather_menu,
            KEEP_AWAKE_MENU => keep_awake_menu,
            TWILIGHT_MENU => twilight_menu,
            KEYBINDS_MENU => keybinds_menu,
            ALARMCLOCK_MENU => alarmclock_menu,
            DOCK_MENU => dock_menu,
            CONTROL_CENTER_MENU => control_center_menu,
            SSH_MENU => ssh_menu,
            USB_MENU => usb_menu,
            DISCORD_MENU => discord_voice_menu,
            CRASHES_MENU => crashes_menu,
            NDNS_MENU => dns_menu,
            NVPN_MENU => vpn_menu,
            NAI_MENU => ai_menu,
            NPODMAN_MENU => podman_menu,
            NGITHUB_MENU => github_menu,
            NQR_MENU => qr_menu,
            NAGENTS_MENU => agents_menu,
            NNOTES_MENU => notes_menu,
            NIP_MENU => ip_menu,
            NVPN_INDICATOR_MENU => vpn_indicator_menu,
            NNETWORK_MENU => network_menu,
            NPOWER_MENU => power_menu,
            MEDIA_PLAYER_MENU => media_player_menu,
            MTUNE_MENU => mtune_menu,
            LYRICS_MENU => lyrics_menu,
            SESSION_MENU => session_menu,
            MDASH_MENU => mdash_menu,
            MARGO_LAYOUT_MENU => margo_layout_menu,
            TRANSLATE_MENU => translate_menu,
            COLOR_PICKER_MENU => color_picker_menu,
            NPLUGIN_PANEL_MENU => plugin_panel_menu,
        }
    };
}
pub(in crate::frame) use positioned_menus;

/// The stack name whose position `name` uses: the wizard shares the Settings
/// slot (both are mutually-exclusive centre panels).
fn position_slot(name: &str) -> &str {
    if name == WIZARD_MENU {
        SETTINGS_MENU
    } else {
        name
    }
}

/// `name`'s configured anchor in `menus`, or `None` for an unknown name.
pub(in crate::frame) fn menu_position_in(menus: &Menus, name: &str) -> Option<Position> {
    macro_rules! lookup {
        ($($menu:ident => $field:ident),* $(,)?) => {
            match position_slot(name) {
                $($menu => Some(menus.$field.position.clone()),)*
                _ => None,
            }
        };
    }
    positioned_menus!(lookup)
}

/// A menu's configured screen anchor, read fresh from the config store by its
/// stable menu-name. The single source of truth for where
/// [`Frame::ensure_menu_built`] first attaches a menu and where
/// [`Frame::apply_left_and_right_side_children`] re-attaches it on a
/// reposition. Unknown names fall back to top-right (callers only pass known
/// names; a test pins that every [`MenuId`] resolves).
pub(in crate::frame) fn menu_position(name: &str) -> Position {
    config_manager()
        .config()
        .menus()
        .with_untracked(|menus| menu_position_in(menus, name))
        .unwrap_or(Position::TopRight)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every menu reachable through `MenuId`, plus the bespoke surfaces the
    /// restack attaches by name.
    fn every_positioned_name() -> Vec<&'static str> {
        let mut names: Vec<&'static str> = MenuId::ALL.iter().map(|id| id.menu_name()).collect();
        names.extend([SCREENSHARE_MENU, SETTINGS_MENU, NPLUGIN_PANEL_MENU]);
        names
    }

    #[test]
    fn every_menu_has_a_configured_position() {
        let menus = Menus::default();
        let missing: Vec<&str> = every_positioned_name()
            .into_iter()
            .filter(|name| menu_position_in(&menus, name).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "menus without a position arm: {missing:?}"
        );
    }

    #[test]
    fn translate_and_privacy_honour_their_configured_position() {
        let mut menus = Menus::default();
        menus.translate_menu.position = Position::BottomLeft;
        menus.privacy_menu.position = Position::Left;
        assert_eq!(
            menu_position_in(&menus, TRANSLATE_MENU),
            Some(Position::BottomLeft)
        );
        assert_eq!(menu_position_in(&menus, PRIVACY_MENU), Some(Position::Left));
    }

    #[test]
    fn wizard_follows_the_settings_position() {
        let mut menus = Menus::default();
        menus.settings_menu.position = Position::Bottom;
        assert_eq!(
            menu_position_in(&menus, WIZARD_MENU),
            Some(Position::Bottom)
        );
    }

    /// The position effect pushes one entry per table row; a menu missing
    /// from the table would never re-fire the restack when moved.
    #[test]
    fn the_position_effect_tracks_every_menu() {
        macro_rules! names {
            ($($menu:ident => $field:ident),* $(,)?) => { vec![$($menu),*] };
        }
        let tracked: Vec<&str> = positioned_menus!(names);
        for name in every_positioned_name() {
            assert!(
                tracked.contains(&position_slot(name)),
                "{name} is not tracked by the position effect"
            );
        }
    }

    #[test]
    fn unknown_names_have_no_position() {
        assert_eq!(menu_position_in(&Menus::default(), "nope"), None);
    }
}
