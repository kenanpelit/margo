//! Settings → Notifications → "Sounds per app or contact": the editor for
//! `notifications.sound_rules`. Same look as the "Muted apps" list above
//! it (entry row + `ok-button-primary` Add, `notification-mute-row` rows
//! with a trash button); kept in its own file because the notification
//! page is already large.
//!
//! The first matching rule wins, so rows reorder like every other ordered
//! Settings list (grip drag + ↑/↓, DESIGN.md §5). A path that holds no
//! playable sound is refused when adding, and an existing rule whose file
//! has gone shows a "missing" badge (the default chime plays for it).

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, NotificationsStoreFields, SoundRule, SoundRuleField,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, WidgetExt};

/// Build the section. The returned scope keeps the "external edit"
/// mirror alive; the owner stores it next to its other effects.
pub(crate) fn build_sound_rules_section() -> (gtk::Box, EffectScope) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    section.set_widget_name("setting:/notifications/sound_rules");

    let title = gtk::Label::new(Some("Sounds per app or contact"));
    title.add_css_class("label-large-bold");
    title.set_halign(gtk::Align::Start);
    section.append(&title);

    let hint = gtk::Label::new(Some(
        "The first rule whose text appears in the chosen part of a notification \
         (case-insensitive) plays its sound instead of the chime; drag or use the \
         arrows to change the order. A folder plays one of its .ogg / .mp3 files \
         at random; if the sound is gone the chime plays. Needs notification \
         sounds on.",
    ));
    hint.add_css_class("label-small");
    hint.set_halign(gtk::Align::Start);
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    hint.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
    section.append(&hint);

    let input = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let pattern = gtk::Entry::new();
    pattern.set_hexpand(true);
    pattern.set_placeholder_text(Some("Contact, group or app (e.g. Alice)"));
    let labels: Vec<&str> = SoundRuleField::all().iter().map(|f| f.label()).collect();
    let field = gtk::DropDown::from_strings(&labels);
    field.set_valign(gtk::Align::Center);
    let sound = gtk::Entry::new();
    sound.add_css_class("sound-rule-path");
    sound.set_hexpand(true);
    sound.set_placeholder_text(Some("Sound file or folder"));
    let add = gtk::Button::with_label("Add");
    add.add_css_class("ok-button-primary");
    add.set_valign(gtk::Align::Center);
    input.append(&pattern);
    input.append(&field);
    input.append(&sound);
    input.append(&add);
    section.append(&input);

    // Shown only after a refused add: the path holds no playable sound.
    let not_found = gtk::Label::new(Some(
        "No .ogg / .mp3 file at that path (or the folder has none).",
    ));
    not_found.add_css_class("label-small");
    not_found.add_css_class("sound-rule-missing");
    not_found.set_halign(gtk::Align::Start);
    not_found.set_visible(false);
    section.append(&not_found);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    section.append(&list);

    {
        // Typing again clears the "not found" state.
        let not_found = not_found.clone();
        sound.connect_changed(move |e| {
            e.remove_css_class("error");
            not_found.set_visible(false);
        });
    }

    let submit = {
        let (pattern, field, sound) = (pattern.clone(), field.clone(), sound.clone());
        let not_found = not_found.clone();
        move || {
            let rule = SoundRule {
                pattern: pattern.text().trim().to_string(),
                field: SoundRuleField::all()
                    .get(field.selected() as usize)
                    .copied()
                    .unwrap_or_default(),
                sound: expand_home(sound.text().trim()),
            };
            if rule.pattern.is_empty() || rule.sound.is_empty() {
                return;
            }
            if !sound_exists(&rule.sound) {
                sound.add_css_class("error");
                not_found.set_visible(true);
                return;
            }
            config_manager().update_config(move |c| c.notifications.sound_rules.push(rule));
            pattern.set_text("");
            sound.set_text("");
        }
    };
    {
        let submit = submit.clone();
        add.connect_clicked(move |_| submit());
    }
    {
        let submit = submit.clone();
        pattern.connect_activate(move |_| submit());
    }
    sound.connect_activate(move |_| submit());

    // Rows follow the config (our own adds/removes and hand edits alike).
    let mut effects = EffectScope::new();
    let list_c = list.clone();
    effects.push(move |_| {
        let rules = config_manager()
            .config()
            .notifications()
            .sound_rules()
            .get();
        rebuild_rows(&list_c, &rules);
    });
    rebuild_rows(
        &list,
        &config_manager()
            .config()
            .notifications()
            .sound_rules()
            .get_untracked(),
    );

    (section, effects)
}

fn rebuild_rows(list: &gtk::Box, rules: &[SoundRule]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let count = rules.len();
    for (idx, rule) in rules.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("notification-mute-row");

        let grip = gtk::Image::from_icon_name("list-drag-handle-symbolic");
        grip.add_css_class("reorder-grip");
        grip.set_valign(gtk::Align::Center);
        grip.set_tooltip_text(Some("Drag to reorder"));
        row.append(&grip);
        crate::reorder_dnd::attach_grip_drag(&grip, &row, move |delta| {
            config_manager().update_config(move |c| {
                move_rule(&mut c.notifications.sound_rules, idx, delta);
            });
        });

        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        let name = gtk::Label::new(Some(&rule.pattern));
        name.add_css_class("label-medium");
        name.set_halign(gtk::Align::Start);
        name.set_xalign(0.0);
        let detail = gtk::Label::new(Some(&format!("{} · {}", rule.field.label(), rule.sound)));
        detail.add_css_class("label-small");
        detail.set_halign(gtk::Align::Start);
        detail.set_xalign(0.0);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        text.append(&name);
        text.append(&detail);
        row.append(&text);

        if !sound_exists(&rule.sound) {
            let badge = gtk::Label::new(Some("missing"));
            badge.add_css_class("label-small");
            badge.add_css_class("sound-rule-missing");
            badge.set_valign(gtk::Align::Center);
            badge.set_tooltip_text(Some(
                "No playable sound at this path any more — the default chime plays instead.",
            ));
            row.append(&badge);
        }

        for (icon, tip, name, delta, enabled) in [
            (
                "go-up-symbolic",
                "Move up",
                "Move sound rule up",
                -1,
                idx > 0,
            ),
            (
                "go-down-symbolic",
                "Move down",
                "Move sound rule down",
                1,
                idx + 1 < count,
            ),
        ] {
            let btn = gtk::Button::new();
            btn.add_css_class("ok-button-surface");
            btn.set_valign(gtk::Align::Center);
            btn.set_tooltip_text(Some(tip));
            btn.set_child(Some(&gtk::Image::from_icon_name(icon)));
            btn.set_sensitive(enabled);
            mshell_common::a11y::name(&btn, name);
            btn.connect_clicked(move |_| {
                config_manager().update_config(move |c| {
                    move_rule(&mut c.notifications.sound_rules, idx, delta);
                });
            });
            row.append(&btn);
        }

        let play = gtk::Button::new();
        play.add_css_class("ok-button-surface");
        play.set_valign(gtk::Align::Center);
        play.set_tooltip_text(Some("Play"));
        play.set_child(Some(&gtk::Image::from_icon_name(
            "media-playback-start-symbolic",
        )));
        mshell_common::a11y::name(&play, "Play sound");
        let path = rule.sound.clone();
        play.connect_clicked(move |_| {
            mshell_sounds::play_sound_path_or_folder(&path);
        });
        row.append(&play);

        let remove = gtk::Button::new();
        remove.add_css_class("ok-button-surface");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some("Remove"));
        remove.set_child(Some(&gtk::Image::from_icon_name("user-trash-symbolic")));
        mshell_common::a11y::name(&remove, "Remove sound rule");
        remove.connect_clicked(move |_| {
            config_manager().update_config(move |c| {
                if idx < c.notifications.sound_rules.len() {
                    c.notifications.sound_rules.remove(idx);
                }
            });
        });
        row.append(&remove);

        list.append(&row);
    }
}

/// Whether `path` resolves to a playable sound (a file, or a folder with an
/// .ogg / .mp3 in it) — the same check the player makes.
fn sound_exists(path: &str) -> bool {
    mshell_sounds::resolve_sound_path(path, 0).is_some()
}

/// Move the rule at `from` by `delta` positions, clamped to the list.
/// Returns whether anything moved.
fn move_rule(rules: &mut Vec<SoundRule>, from: usize, delta: i32) -> bool {
    if from >= rules.len() || delta == 0 {
        return false;
    }
    let last = rules.len() as i64 - 1;
    let to = (from as i64 + i64::from(delta)).clamp(0, last) as usize;
    if to == from {
        return false;
    }
    let rule = rules.remove(from);
    rules.insert(to, rule);
    true
}

/// `~/x` → `$HOME/x` so the stored path works for the sound player.
fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(names: &[&str]) -> Vec<SoundRule> {
        names
            .iter()
            .map(|n| SoundRule {
                pattern: n.to_string(),
                field: SoundRuleField::default(),
                sound: format!("/s/{n}.ogg"),
            })
            .collect()
    }

    fn names(r: &[SoundRule]) -> Vec<&str> {
        r.iter().map(|r| r.pattern.as_str()).collect()
    }

    /// First match wins, so order matters: a rule can be moved instead of
    /// deleted and re-added.
    #[test]
    fn a_rule_moves_by_the_delta_and_clamps_at_the_ends() {
        let mut r = rules(&["a", "b", "c"]);
        assert!(move_rule(&mut r, 2, -1));
        assert_eq!(names(&r), ["a", "c", "b"]);
        assert!(move_rule(&mut r, 0, 5));
        assert_eq!(names(&r), ["c", "b", "a"]);
        assert!(!move_rule(&mut r, 0, -1), "already first");
        assert!(!move_rule(&mut r, 2, 1), "already last");
        assert!(!move_rule(&mut r, 7, -1), "out of range");
        assert!(!move_rule(&mut r, 1, 0));
        assert_eq!(names(&r), ["c", "b", "a"]);
    }
}
