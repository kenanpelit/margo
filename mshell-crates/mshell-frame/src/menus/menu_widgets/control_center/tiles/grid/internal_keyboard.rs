//! The Internal Keyboard tile: margo's built-in keyboard toggle
//! (`internal_keyboard`, port of the internal-keyboard plugin's tile).
//!
//! Highlighted (`.active`) while the built-in keyboard is OFF, as in the
//! plugin: the tile lights up when it is doing something. A click sends
//! `internal_keyboard toggle`; margo refuses to turn it off while it is the
//! only keyboard (and says so in a notification). The state is margo's
//! `internal_keyboard` snapshot object, mirrored reactively — no polling.
//!
//! Wired here rather than through the tile component's model so the tile
//! needs no new component messages: it owns its click and its state.

use crate::menus::menu_widgets::control_center::tile::TileWidget;
use futures::StreamExt;
use mshell_margo_client::state_json::RawInternalKeyboard;
use mshell_services::margo_service;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};

/// Subtitle + active (= keyboard off) for a state; `None` = a margo
/// without the toggle.
pub(super) fn tile_state(state: Option<&RawInternalKeyboard>) -> (&'static str, bool) {
    match state {
        None => ("Unavailable", false),
        Some(s) if s.name.is_none() => ("Not found", false),
        Some(s) if s.disabled => ("Off", true),
        Some(s) if !s.external => ("On · only keyboard", false),
        Some(_) => ("On", false),
    }
}

fn tooltip(state: Option<&RawInternalKeyboard>) -> String {
    let name = state
        .and_then(|s| s.name.as_deref())
        .unwrap_or("no built-in keyboard found");
    let hint = match state {
        Some(s) if s.disabled => "Click: turn it back on",
        Some(s) if s.name.is_some() && s.external => "Click: turn it off",
        Some(s) if s.name.is_some() => "Connect another keyboard to turn it off",
        _ => "Set internal_keyboard = <device> in margo's config",
    };
    format!("Built-in keyboard: {name}\n{hint}")
}

fn render(tile: &TileWidget, state: Option<&RawInternalKeyboard>) {
    let (subtitle, active) = tile_state(state);
    tile.set_subtitle(subtitle);
    tile.set_active(active);
    tile.button.set_tooltip_text(Some(&tooltip(state)));
}

/// Wire the tile's click and keep its look in step with margo.
pub(super) fn wire(tile: &TileWidget) {
    tile.button.connect_clicked(|_| {
        mshell_margo_client::ctl::dispatch_detached("internal_keyboard", &["toggle"]);
    });
    render(tile, margo_service().internal_keyboard.get().as_ref());
    let (button, icon, subtitle, title, frame) = (
        tile.button.clone(),
        tile.icon.clone(),
        tile.subtitle.clone(),
        tile.title.clone(),
        tile.frame.clone(),
    );
    relm4::spawn_local(async move {
        let tile = TileWidget {
            button,
            icon,
            subtitle,
            title,
            frame,
        };
        let mut stream = margo_service().internal_keyboard.watch();
        while let Some(state) = stream.next().await {
            render(&tile, state.as_ref());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: Option<&str>, disabled: bool, external: bool) -> RawInternalKeyboard {
        RawInternalKeyboard {
            name: name.map(str::to_string),
            disabled,
            external,
            auto: false,
        }
    }

    #[test]
    fn tile_lights_up_while_the_keyboard_is_off() {
        let at = Some("AT Translated Set 2 keyboard");
        assert_eq!(tile_state(None), ("Unavailable", false));
        assert_eq!(
            tile_state(Some(&s(None, false, true))),
            ("Not found", false)
        );
        assert_eq!(tile_state(Some(&s(at, true, true))), ("Off", true));
        assert_eq!(tile_state(Some(&s(at, false, true))), ("On", false));
        assert_eq!(
            tile_state(Some(&s(at, false, false))),
            ("On · only keyboard", false)
        );
    }

    #[test]
    fn tooltip_says_what_a_click_does() {
        let at = Some("AT Translated Set 2 keyboard");
        assert!(tooltip(Some(&s(at, true, true))).contains("turn it back on"));
        assert!(tooltip(Some(&s(at, false, true))).contains("turn it off"));
        assert!(tooltip(Some(&s(at, false, false))).contains("Connect another keyboard"));
        assert!(tooltip(None).contains("internal_keyboard ="));
    }
}
