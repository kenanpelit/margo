//! Settings → Game Mode, the session part: the power profile, the switches
//! for what entering hides / pauses / clears, and the enter / exit hooks
//! (`config.game_mode`). Built imperatively and table-driven, then placed
//! under the effect switches by `game_mode_settings.rs`; one scoped effect
//! mirrors external edits (`mshellctl config`, a hand-edited profile) back
//! into the widgets.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GameMode};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::glib::prelude::ObjectExt;
use relm4::gtk::prelude::{BoxExt, EditableExt, EntryExt, WidgetExt};

/// The bool keys this section owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    PauseMedia,
    ClearNotifications,
    HideBars,
    HideDock,
    DisableOsd,
    DisableWallpaper,
}

/// `(key, row tag, title, description)` in page order.
const SWITCHES: &[(Key, &str, &str, &str)] = &[
    (
        Key::PauseMedia,
        "setting:/game_mode/pause_media",
        "Pause media",
        "Pause every playing media player when game mode starts. Not resumed afterwards.",
    ),
    (
        Key::ClearNotifications,
        "setting:/game_mode/clear_notifications",
        "Clear notification pop-ups",
        "Dismiss the pop-ups on screen when game mode starts. The notification centre keeps them.",
    ),
    (
        Key::HideBars,
        "setting:/game_mode/hide_bars",
        "Hide the bars",
        "On every monitor. The bars that show by default come back afterwards.",
    ),
    (
        Key::HideDock,
        "setting:/game_mode/hide_dock",
        "Hide the dock",
        "The standalone (Popup style) dock. The bar-attached dock only opens on demand and is left alone.",
    ),
    (
        Key::DisableOsd,
        "setting:/game_mode/disable_osd",
        "No volume / brightness pop-ups",
        "Hold back the on-screen display for volume, microphone, brightness and network changes.",
    ),
    (
        Key::DisableWallpaper,
        "setting:/game_mode/disable_wallpaper",
        "Stop drawing the wallpaper",
        "Close the shell's wallpaper windows to free their graphics memory; they come back afterwards. A wallpaper margo draws itself is left alone.",
    ),
];

/// `(config value, label)` for the power-profile dropdown.
const PROFILES: &[(&str, &str)] = &[
    ("", "Leave as is"),
    ("performance", "Performance"),
    ("balanced", "Balanced"),
    ("power-saver", "Power Saver"),
];

fn read(cfg: &GameMode, key: Key) -> bool {
    match key {
        Key::PauseMedia => cfg.pause_media,
        Key::ClearNotifications => cfg.clear_notifications,
        Key::HideBars => cfg.hide_bars,
        Key::HideDock => cfg.hide_dock,
        Key::DisableOsd => cfg.disable_osd,
        Key::DisableWallpaper => cfg.disable_wallpaper,
    }
}

fn write(key: Key, v: bool) {
    config_manager().update_config(move |c| match key {
        Key::PauseMedia => c.game_mode.pause_media = v,
        Key::ClearNotifications => c.game_mode.clear_notifications = v,
        Key::HideBars => c.game_mode.hide_bars = v,
        Key::HideDock => c.game_mode.hide_dock = v,
        Key::DisableOsd => c.game_mode.disable_osd = v,
        Key::DisableWallpaper => c.game_mode.disable_wallpaper = v,
    });
}

/// Dropdown index for a stored profile; anything unknown reads as "Leave
/// as is" (which is also what the shell does with it).
fn profile_index(stored: &str) -> u32 {
    let stored = stored.trim();
    PROFILES
        .iter()
        .position(|(v, _)| !v.is_empty() && stored.eq_ignore_ascii_case(v))
        .unwrap_or(0) as u32
}

/// A hook as stored: trimmed (blank = no hook).
fn normalise_hook(text: &str) -> String {
    text.trim().to_string()
}

fn save_hook(exit: bool, text: &str) {
    let v = normalise_hook(text);
    let gm = config_manager().config().game_mode().get_untracked();
    let current = if exit { gm.on_exit } else { gm.on_enter };
    if current == v {
        return;
    }
    if exit {
        config_manager().update_config(move |c| c.game_mode.on_exit = v);
    } else {
        config_manager().update_config(move |c| c.game_mode.on_enter = v);
    }
}

/// An Adwaita action row: title + subtitle; the caller appends the control.
fn row(tag: &str, title: &str, subtitle: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    row.add_css_class("action-row");
    row.set_widget_name(tag);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium-bold");
    t.set_halign(gtk::Align::Start);
    let s = gtk::Label::new(Some(subtitle));
    s.add_css_class("label-small");
    s.set_halign(gtk::Align::Start);
    s.set_xalign(0.0);
    s.set_wrap(true);
    s.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
    text.append(&t);
    text.append(&s);
    row.append(&text);
    row
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("label-large-bold");
    l.set_halign(gtk::Align::Start);
    l
}

fn boxed_list() -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("boxed-list");
    list
}

fn hook_entry(exit: bool, value: &str, placeholder: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_valign(gtk::Align::Center);
    entry.set_width_chars(24);
    entry.set_placeholder_text(Some(placeholder));
    entry.set_text(value);
    entry.connect_activate(move |e| save_hook(exit, &e.text()));
    // Leaving the field applies it too, so a typed command isn't lost.
    let focus = gtk::EventControllerFocus::new();
    let e = entry.clone();
    focus.connect_leave(move |_| save_hook(exit, &e.text()));
    entry.add_controller(focus);
    entry
}

/// The session + hooks section; its mirror effect is pushed into `effects`
/// (the page's scope, so it lives as long as the page).
pub(crate) fn section(effects: &mut EffectScope) -> gtk::Box {
    let cfg = config_manager().config().game_mode().get_untracked();
    let section = gtk::Box::new(gtk::Orientation::Vertical, 16);

    section.append(&heading("Session"));
    let session = boxed_list();

    let profile_row = row(
        "setting:/game_mode/power_profile",
        "Power profile",
        "Switch to this profile while game mode is on and put the previous one back afterwards (power-profiles-daemon). Kept if you pick another profile by hand meanwhile.",
    );
    let labels: Vec<&str> = PROFILES.iter().map(|(_, l)| *l).collect();
    let profile = gtk::DropDown::new(Some(gtk::StringList::new(&labels)), None::<gtk::Expression>);
    profile.set_width_request(150);
    profile.set_valign(gtk::Align::Center);
    profile.set_selected(profile_index(&cfg.power_profile));
    let profile_handler = profile.connect_selected_notify(|dd| {
        let v = PROFILES
            .get(dd.selected() as usize)
            .map_or("", |(v, _)| v)
            .to_string();
        config_manager().update_config(move |c| c.game_mode.power_profile = v);
    });
    profile_row.append(&profile);
    session.append(&profile_row);

    let mut switches = Vec::new();
    for (key, tag, title, desc) in SWITCHES {
        let r = row(tag, title, desc);
        let sw = gtk::Switch::new();
        sw.set_valign(gtk::Align::Center);
        sw.set_active(read(&cfg, *key));
        let k = *key;
        let handler = sw.connect_state_set(move |_, v| {
            write(k, v);
            glib::Propagation::Proceed
        });
        r.append(&sw);
        session.append(&r);
        switches.push((*key, sw, handler));
    }
    section.append(&session);

    section.append(&heading("Hooks"));
    let hooks = boxed_list();
    let enter_row = row(
        "setting:/game_mode/on_enter",
        "When game mode starts",
        "A command run through sh -c, e.g. a script that stops a sync daemon. Press Enter to apply; blank runs nothing.",
    );
    let enter = hook_entry(false, &cfg.on_enter, "systemctl --user stop syncthing");
    enter_row.append(&enter);
    hooks.append(&enter_row);
    let exit_row = row(
        "setting:/game_mode/on_exit",
        "When game mode ends",
        "Run when game mode turns off. Press Enter to apply; blank runs nothing.",
    );
    let exit = hook_entry(true, &cfg.on_exit, "systemctl --user start syncthing");
    exit_row.append(&exit);
    hooks.append(&exit_row);
    section.append(&hooks);

    effects.push(move |_| {
        let gm = config_manager().config().game_mode().get();
        let idx = profile_index(&gm.power_profile);
        if profile.selected() != idx {
            profile.block_signal(&profile_handler);
            profile.set_selected(idx);
            profile.unblock_signal(&profile_handler);
        }
        for (key, sw, handler) in &switches {
            let v = read(&gm, *key);
            if sw.is_active() != v {
                sw.block_signal(handler);
                sw.set_active(v);
                sw.unblock_signal(handler);
            }
        }
        for (entry, value) in [(&enter, &gm.on_enter), (&exit, &gm.on_exit)] {
            if !entry.has_focus() && entry.text() != value.as_str() {
                entry.set_text(value);
            }
        }
    });

    section
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_switch_row_names_its_own_key() {
        for (key, tag, ..) in SWITCHES {
            let field = tag.rsplit('/').next().unwrap();
            let on = GameMode {
                pause_media: field == "pause_media",
                clear_notifications: field == "clear_notifications",
                hide_bars: field == "hide_bars",
                hide_dock: field == "hide_dock",
                disable_osd: field == "disable_osd",
                disable_wallpaper: field == "disable_wallpaper",
                ..GameMode::default()
            };
            assert!(read(&on, *key), "{field}");
            assert!(!read(&GameMode::default(), *key), "{field} defaults off");
        }
    }

    #[test]
    fn the_profile_dropdown_maps_to_the_stored_value() {
        assert_eq!(profile_index(""), 0);
        assert_eq!(profile_index("performance"), 1);
        assert_eq!(profile_index(" Power-Saver "), 3);
        assert_eq!(profile_index("turbo"), 0, "unknown reads as leave as is");
        for (i, (v, _)) in PROFILES.iter().enumerate() {
            assert_eq!(profile_index(v), i as u32);
        }
    }

    #[test]
    fn hooks_are_stored_trimmed() {
        assert_eq!(normalise_hook("  notify-send hi \n"), "notify-send hi");
        assert_eq!(normalise_hook("   "), "");
    }
}
