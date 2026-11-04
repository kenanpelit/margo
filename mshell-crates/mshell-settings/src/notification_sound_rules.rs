//! Settings → Notifications → "Sounds per app or contact": the editor for
//! `notifications.sound_rules`. Same look as the "Muted apps" list above
//! it (entry row + `ok-button-primary` Add, `notification-mute-row` rows
//! with a trash button); kept in its own file because the notification
//! page is already large.

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
         (case-insensitive) plays its sound instead of the chime. A folder plays \
         one of its .ogg / .mp3 files at random. Needs notification sounds on.",
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

    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    section.append(&list);

    let submit = {
        let (pattern, field, sound) = (pattern.clone(), field.clone(), sound.clone());
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
    for (idx, rule) in rules.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("notification-mute-row");

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

        let play = gtk::Button::new();
        play.add_css_class("ok-button-surface");
        play.set_valign(gtk::Align::Center);
        play.set_tooltip_text(Some("Play"));
        play.set_child(Some(&gtk::Image::from_icon_name(
            "media-playback-start-symbolic",
        )));
        mshell_common::a11y::name(&play, "Play sound");
        let path = rule.sound.clone();
        play.connect_clicked(move |_| mshell_sounds::play_sound_path_or_folder(&path));
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

/// `~/x` → `$HOME/x` so the stored path works for the sound player.
fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => path.to_string(),
    }
}
