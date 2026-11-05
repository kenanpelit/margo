//! Settings → Widgets → Valent Connect → "Screen mirroring": the scrcpy
//! options (`valent.scrcpy_screen_off`, `valent.scrcpy_args`) that the
//! Valent menu's Mirror button uses. Appended to the generic menu page for
//! `MenuKind::Valent`, like the Control Center's tile editor.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ValentStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, EditableExt, EntryExt, WidgetExt};

/// Build the section. The returned scope keeps the "external edit" mirror
/// alive; the owner stores it next to its other effects.
pub(crate) fn build_valent_mirror_section() -> (gtk::Box, EffectScope) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);

    let title = gtk::Label::new(Some("Screen mirroring"));
    title.add_css_class("label-large-bold");
    title.set_halign(gtk::Align::Start);
    section.append(&title);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("boxed-list");
    section.append(&list);

    // ── Screen off ──
    let off_row = row(
        "setting:/valent/scrcpy_screen_off",
        "Turn the phone screen off",
        "Keep the phone's own display dark while its screen is mirrored (scrcpy --turn-screen-off).",
    );
    let off = gtk::Switch::new();
    off.set_valign(gtk::Align::Center);
    off.set_active(
        config_manager()
            .config()
            .valent()
            .scrcpy_screen_off()
            .get_untracked(),
    );
    off.connect_active_notify(|s| {
        let v = s.is_active();
        if config_manager()
            .config()
            .valent()
            .scrcpy_screen_off()
            .get_untracked()
            != v
        {
            config_manager().update_config(move |c| c.valent.scrcpy_screen_off = v);
        }
    });
    off_row.append(&off);
    list.append(&off_row);

    // ── Extra arguments ──
    let args_row = row(
        "setting:/valent/scrcpy_args",
        "Extra scrcpy arguments",
        "Added to every mirror window, separated by spaces (no shell), e.g. --max-size=1280 --stay-awake. Press Enter to apply.",
    );
    let args = gtk::Entry::new();
    args.set_valign(gtk::Align::Center);
    args.set_width_chars(24);
    args.set_placeholder_text(Some("--max-size=1280"));
    args.set_text(
        &config_manager()
            .config()
            .valent()
            .scrcpy_args()
            .get_untracked(),
    );
    args.connect_activate(|e| save_args(&e.text()));
    {
        // Leaving the field applies it too, so a typed value isn't lost.
        let focus = gtk::EventControllerFocus::new();
        let entry = args.clone();
        focus.connect_leave(move |_| save_args(&entry.text()));
        args.add_controller(focus);
    }
    args_row.append(&args);
    list.append(&args_row);

    // Mirror external edits (a hand-edited profile, `mshellctl config`).
    let mut effects = EffectScope::new();
    let (off_c, args_c) = (off.clone(), args.clone());
    effects.push(move |_| {
        let valent = config_manager().config().valent();
        let screen_off = valent.clone().scrcpy_screen_off().get();
        let extra = valent.scrcpy_args().get();
        if off_c.is_active() != screen_off {
            off_c.set_active(screen_off);
        }
        if !args_c.has_focus() && args_c.text() != extra {
            args_c.set_text(&extra);
        }
    });

    (section, effects)
}

fn save_args(text: &str) {
    let v = normalise_args(text);
    if config_manager()
        .config()
        .valent()
        .scrcpy_args()
        .get_untracked()
        != v
    {
        config_manager().update_config(move |c| c.valent.scrcpy_args = v);
    }
}

/// The stored form of the arguments: whitespace-separated words, single
/// spaces, no leading / trailing space (scrcpy splits on whitespace anyway).
fn normalise_args(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// An Adwaita action row: title + subtitle on the left; the caller appends
/// the control.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_stored_as_single_spaced_words() {
        assert_eq!(
            normalise_args("  --max-size=1280   --stay-awake \t"),
            "--max-size=1280 --stay-awake"
        );
        assert_eq!(normalise_args("   "), "");
    }
}
